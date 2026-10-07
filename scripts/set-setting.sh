#!/usr/bin/env bash
# Change settings in prod.tfvars and apply them (LAUNCH_PLAN Part 3,
# "Changes that aren't code").
#
#   scripts/set-setting.sh KEY=VALUE... [--unset KEY]...
#   e.g. scripts/set-setting.sh mail_max_per_second=14
#
# Fetches prod.tfvars from the state bucket, sets each KEY (a variable of
# infra/variables.tf; anything else is refused) to VALUE -- a number, true,
# false, null, a [list], a {map} or a "quoted string" as written, anything
# else as a string -- or removes it (back to the default), shows the change,
# plans, stops on a database, bucket or network destroyed or replaced, applies
# once you say so, uploads prod.tfvars, notes it in ~/birdtest-releases.log,
# and waits for the service if the plan restarted it. The image variables
# belong to scripts/deploy.sh and scripts/rollback.sh and are refused here.
#
# Needs aws, terraform, jq, curl, the settings in ~/.birdtest-env and infra/
# initialized with the stack's backend.
set -euo pipefail
# shellcheck source=scripts/lib/ops.sh
source "$(dirname "$0")/lib/ops.sh"

usage() { sed -n '5,6p' "$0" >&2; exit 2; }

sets=() unsets=()
while (($#)); do
  case $1 in
    --unset) [[ $# -ge 2 ]] || usage; unsets+=("$2"); shift ;;
    -*) usage ;;
    *=*) sets+=("$1") ;;
    *) usage ;;
  esac
  shift
done
((${#sets[@]} + ${#unsets[@]})) || usage
for kv in ${sets[@]+"${sets[@]}"} ${unsets[@]+"${unsets[@]}"}; do
  case ${kv%%=*} in
    backend_image | derived_builder_image | frontend_image)
      ops_die "${kv%%=*} is deploy.sh's and rollback.sh's to change" ;;
  esac
done

ops_init
ops_need aws terraform jq curl
ops_load_env
ops_require STATE_BUCKET STATE_REGION
ops_login
ops_stack
ops_tfvars_fetch

summary=""
for kv in ${sets[@]+"${sets[@]}"}; do
  key=${kv%%=*}
  value=$(ops_hcl_value "${kv#*=}")
  ops_tfvars_set "$key" "$value"
  summary="$summary $key=$value"
done
for key in ${unsets[@]+"${unsets[@]}"}; do
  [[ "$key" =~ ^[a-z_][a-z0-9_]*$ ]] || ops_die "not a variable name: $key"
  grep -Eq "^[[:space:]]*${key}[[:space:]]*=" "$OPS_TFVARS" || ops_die "$key is not set in prod.tfvars"
  grep -Ev "^[[:space:]]*${key}[[:space:]]*=" "$OPS_TFVARS" > "$OPS_TFVARS.new"
  mv "$OPS_TFVARS.new" "$OPS_TFVARS"
  summary="$summary unset:$key"
done
if cmp -s "$OPS_TFVARS_BASE" "$OPS_TFVARS"; then
  ops_say "prod.tfvars already says that: nothing to do"
  exit 0
fi
diff -u --label "prod.tfvars (live)" --label "prod.tfvars (new)" "$OPS_TFVARS_BASE" "$OPS_TFVARS" >&2 || true

ops_ship "setting$summary"
ops_say "applied:$summary"
