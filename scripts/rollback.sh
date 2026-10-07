#!/usr/bin/env bash
# Put an earlier release's images back (RUNBOOK.md, "Rolling back a deploy").
#
#   scripts/rollback.sh [--to TAG] [--reset-db] [--min-magpie-version V]
#
# First it looks at what ECS runs. If the service's circuit breaker already
# rolled a release back (its -deploy-failed mail), the service runs an older
# image than prod.tfvars names, and the next apply of any kind would deploy
# the broken one again: that older image is then the default target, and this
# makes Terraform agree with it. Otherwise the default is the release before
# the live one, from ~/birdtest-releases.log. The tags the log knows are
# listed and you choose (Enter takes the default; --to skips the question).
#
# Then the runbook's questions: whether the release being undone added an
# enum value (a new job type) that a job uses -- deactivate those first, or
# the older image stops every claim -- and, when the log says the target ran
# with another min_magpie_version, whether to put that back (the backend
# refuses to start below its own MAGPIE). --min-magpie-version sets it
# outright.
#
# A target whose migrations differ from the live release's cannot start on
# this database (until launch 0001 is edited in place): that is refused
# unless --reset-db, which empties the database as scripts/deploy.sh
# --reset-db does. Then the plan gate, apply, upload, release log and health
# wait, as deploy.sh. Nothing is built: the images must be in ECR.
#
# Needs git, aws, terraform, jq, curl, the settings in ~/.birdtest-env and
# infra/ initialized with the stack's backend.
set -euo pipefail
# shellcheck source=scripts/lib/ops.sh
source "$(dirname "$0")/lib/ops.sh"

usage() { sed -n '4p' "$0" >&2; exit 2; }

target="" reset=0 floor=""
while (($#)); do
  case $1 in
    --to) [[ $# -ge 2 ]] || usage; target=$2; shift ;;
    --reset-db) reset=1 ;;
    --min-magpie-version) [[ $# -ge 2 ]] || usage; floor=$2; shift ;;
    *) usage ;;
  esac
  shift
done

ops_init
ops_need git aws terraform jq curl
ops_load_env
ops_require STATE_BUCKET STATE_REGION
ops_login
ops_stack
ops_tfvars_fetch
state_tag=$(ops_image_tag "$(ops_tfvar backend_image)")

# What ECS runs, against what Terraform names.
running=$(ops_running_backend_image) || running=""
running_tag=$(ops_image_tag "$running")
default=""
if [[ -n "$running" && "$running" != None && "$running_tag" != "$state_tag" ]]; then
  ops_say "the service runs $running_tag, but prod.tfvars (and Terraform) name $state_tag:"
  ops_say "ECS's circuit breaker rolled the release back. Recent events:"
  aws ecs describe-services --cluster "$OPS_CLUSTER" --services "$OPS_SERVICE" \
    --query 'services[0].events[:6].[createdAt, message]' --output text >&2 || true
  default=$running_tag
fi
logged=$(ops_logged_tags | grep -vx "$state_tag" || true)
if [[ -z "$default" ]]; then
  default=$(printf '%s\n' "$logged" | head -1)
fi

if [[ -z "$target" ]]; then
  ops_say "live (prod.tfvars): $state_tag"
  if [[ -n "$logged" ]]; then
    ops_say "earlier releases in $OPS_RELEASE_LOG, newest first:"
    printf '%s\n' "$logged" | head -8 | sed 's/^/    /' >&2
  fi
  [[ -n "$default" ]] || ops_die "no earlier release known: give one with --to TAG"
  printf 'Roll back to which tag? [%s] ' "$default" >&2
  ops_read target || target=""
  target=${target:-$default}
fi
[[ "$target" != "$state_tag" ]] || ops_die "$target is already what prod.tfvars names"

# The images must exist: nothing is built here.
for repo in backend derived-builder frontend; do
  aws ecr describe-images --repository-name "birdtest-$repo" --image-ids "imageTag=$target" >/dev/null 2>&1 \
    || ops_die "birdtest-$repo:$target is not in ECR"
done

# The schema the live release left.
schema_rc=0
changed=$(ops_migrations_changed "$target" "$state_tag") || schema_rc=$?
case $schema_rc in
  0)
    printf '%s\n' "$changed" >&2
    if ((reset)); then
      ops_say "these migrations differ between $target and the live $state_tag: the database will be reset (--reset-db)"
    else
      ops_die "these migrations differ between $target and the live release ($state_tag): $target's backend cannot start on this database (sqlx checksums them). Until launch that needs a reset: --reset-db"
    fi ;;
  2)
    if ((reset)); then
      ops_say "$target or $state_tag is not a commit here; going on with a reset (--reset-db)"
    else
      ops_die "$target or $state_tag is not a commit in this checkout (git fetch), so whether the migrations differ is unknown; --reset-db if they do"
    fi ;;
esac

ops_say "If $state_tag added an enum value (a new job type, say) that a job uses, deactivate every such job first: the older image fails to read it and one active job of an unknown type stops every claim."
ops_confirm "Done, or nothing to do?" || ops_die "stopped: nothing changed"

current_floor=$(ops_tfvar min_magpie_version)
if [[ -z "$floor" ]]; then
  logged_floor=$(ops_logged_floor "$target")
  if [[ -n "$logged_floor" && "$logged_floor" != "$current_floor" ]]; then
    ops_say "$target ran with min_magpie_version ${logged_floor:-<default>}; prod.tfvars now says ${current_floor:-<default>}"
    if ops_confirm "Put $logged_floor back? (the backend refuses to start below its own MAGPIE; it readmits the builds the raise kept out)"; then
      floor=$logged_floor
    fi
  fi
fi

ops_retag "$target"
if [[ -n "$floor" ]]; then
  value=$(ops_hcl_value "$floor")
  ops_tfvars_set min_magpie_version "$value"
fi
diff -u --label "prod.tfvars (live)" --label "prod.tfvars (rollback)" \
  "$OPS_TFVARS_BASE" "$OPS_TFVARS" >&2 || true
ops_say "the apply uses this checkout's infra/: a task-definition change the release made stays unless it is reverted here too"

export OPS_RESET=$reset   # read by ops_ship
ops_ship "rolled-back tag=$target previous=$state_tag $(ops_release_fields)"
ops_say "rolled back to $target"
if ((reset)); then
  ops_reset_checklist "$(ops_site_host)"
fi
