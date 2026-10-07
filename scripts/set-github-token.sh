#!/usr/bin/env bash
# Store the optional GitHub token for input-data imports (README.md,
# "Deploying": without one GitHub allows 60 calls an hour).
#
#   scripts/set-github-token.sh
#
# Reads the token from the terminal without echoing it, writes it to the SSM
# parameter /birdtest/GITHUB_TOKEN (SecureString, the default aws/ssm key --
# the tasks' execution role cannot decrypt any other -- in the stack's
# region), and then points github_token_parameter_arn at it with
# scripts/set-setting.sh (plan, apply, upload). If the ARN is already set, the
# service is restarted instead so its task reads the new value. A fine-grained
# token with read-only access to public repositories is enough.
#
# Needs aws, terraform, jq, curl, the settings in ~/.birdtest-env and infra/
# initialized with the stack's backend.
set -euo pipefail
# shellcheck source=scripts/lib/ops.sh
source "$(dirname "$0")/lib/ops.sh"

[[ $# == 0 ]] || { sed -n '5p' "$0" >&2; exit 2; }

ops_init
ops_need aws terraform jq curl
ops_load_env
ops_require STATE_BUCKET STATE_REGION
ops_login
ops_stack
ops_tfvars_fetch
# Beside the stack's other two (/birdtest/...).
param=$(tf -json ssm_parameter_names | jq -r '.[0]')
param=${param%/*}/GITHUB_TOKEN
[[ "$param" == /*/GITHUB_TOKEN ]] || ops_die "could not tell the stack's parameter prefix"

printf 'GitHub token (not echoed): ' >&2
token=""
if [[ -z "${OPS_TTY:-}" ]]; then stty -echo </dev/tty; fi
ops_read token || token=""
if [[ -z "${OPS_TTY:-}" ]]; then stty echo </dev/tty; fi
printf '\n' >&2
[[ -n "$token" ]] || ops_die "no token given: nothing changed"
[[ "$token" =~ ^[A-Za-z0-9_]+$ ]] || ops_die "that does not look like a GitHub token (letters, digits and _ only)"

# The value through a file, not the command line, where `ps` would show it.
printf '%s' "$token" > "$OPS_TMP/token"
unset token
aws ssm put-parameter --name "$param" --type SecureString --overwrite \
  --value "file://$OPS_TMP/token" >/dev/null
rm -f "$OPS_TMP/token"
arn=$(aws ssm get-parameter --name "$param" --query Parameter.ARN --output text)
ops_say "$param written ($arn)"

if [[ "$(ops_tfvar github_token_parameter_arn)" == "$arn" ]]; then
  ops_say "prod.tfvars already names it: restarting the service so its task reads the new token"
  aws ecs update-service --cluster "$OPS_CLUSTER" --service "$OPS_SERVICE" --force-new-deployment \
    --query 'service.serviceName' --output text >/dev/null
  ops_wait_release "$(ops_tfvar backend_image)"
else
  exec "$OPS_ROOT/scripts/set-setting.sh" "github_token_parameter_arn=$arn"
fi
