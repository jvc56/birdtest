#!/usr/bin/env bash
# Set up this machine to deploy (LAUNCH_PLAN Part 2, "What they do"), after
# the owner has given you an AWS login and write access on GitHub.
#
#   scripts/onboard-deployer.sh
#
# Run it from a clone of birdtest, after `aws configure sso` (profile name
# `birdtest`) with the access portal URL you were sent. It:
#   1. checks the tools (Step 3 of the launch plan lists how to install them);
#   2. writes ~/.birdtest-env if there is none, asking for the site's domain
#      and the regions, and adds ACCOUNT, STATE_BUCKET and REGISTRY to it;
#   3. checks the login (`aws sso login` if it has expired);
#   4. clones MAGPIE to ~/MAGPIE (MAGPIE_DIR) for deploy.sh's pin check, if
#      it is not there;
#   5. writes infra/backend.tf for the shared state (the S3 bucket
#      birdtest-tfstate-<account>, locked with a lock file; ignored by git)
#      if there is none, and runs `terraform init`;
#   6. fetches prod.tfvars from the state bucket;
#   7. ends with a plan that must show no changes. If it shows some, stop and
#      compare with the person who deployed last: usually one of you has an
#      out-of-date main or prod.tfvars.
#
# It changes nothing in AWS.
set -euo pipefail
# shellcheck source=scripts/lib/ops.sh
source "$(dirname "$0")/lib/ops.sh"

[[ $# == 0 ]] || { sed -n '5p' "$0" >&2; exit 2; }

ops_init
ops_need git aws session-manager-plugin terraform jq docker python3 openssl curl gh
ops_say "terraform $(terraform version -json | jq -r .terraform_version): use the same version as the other deployers (state written by a newer one cannot be read by an older)"

ask() {  # variable, question, default
  local answer=""
  printf '%s [%s] ' "$2" "$3" >&2
  ops_read answer || answer=""
  printf -v "$1" '%s' "${answer:-$3}"
}

# 2. The settings file.
if [[ ! -e "$OPS_ENV_FILE" ]]; then
  ops_say "no $OPS_ENV_FILE yet; writing one"
  site="" region="" state_region="" profile=""
  ask site "The site's domain?" birdtest.org
  ask region "The stack's region?" us-east-1
  ask state_region "The region of the Terraform state bucket?" us-east-2
  ask profile "The AWS CLI profile (aws configure sso)?" birdtest
  for v in "$site" "$region" "$state_region" "$profile"; do
    [[ "$v" =~ ^[A-Za-z0-9.-]+$ ]] || ops_die "not a plain name: $v"
  done
  cat > "$OPS_ENV_FILE" <<EOF
export AWS_PROFILE=$profile
export AWS_PAGER=""                 # stops the CLI swallowing pasted commands
export REGION=$region             # the stack's region
export AWS_REGION=\$REGION AWS_DEFAULT_REGION=\$REGION
export STATE_REGION=$state_region       # where Terraform's state lives
export SITE=$site            # the site's domain
EOF
  ops_say "wrote $OPS_ENV_FILE; add  source $OPS_ENV_FILE  to your shell's startup file"
fi
ops_load_env
ops_require AWS_PROFILE REGION STATE_REGION SITE
aws configure list-profiles 2>/dev/null | grep -qx "$AWS_PROFILE" \
  || ops_die "no AWS CLI profile $AWS_PROFILE: run  aws configure sso  first (profile name $AWS_PROFILE)"

# 3. The login, and the account's values.
ops_login
if [[ -z "${ACCOUNT:-}" || -z "${STATE_BUCKET:-}" || -z "${REGISTRY:-}" ]]; then
  printf 'export ACCOUNT=%s STATE_BUCKET=birdtest-tfstate-%s REGISTRY=%s.dkr.ecr.%s.amazonaws.com\n' \
    "$OPS_ACCOUNT" "$OPS_ACCOUNT" "$OPS_ACCOUNT" "${REGION:?}" >> "$OPS_ENV_FILE"
  ops_say "added ACCOUNT, STATE_BUCKET and REGISTRY to $OPS_ENV_FILE"
  ops_load_env
  ops_require ACCOUNT STATE_BUCKET REGISTRY
fi
aws s3api head-bucket --bucket "$STATE_BUCKET" --region "${STATE_REGION:?}" >/dev/null 2>&1 \
  || ops_die "cannot reach the state bucket $STATE_BUCKET in $STATE_REGION: ask the owner for its name"

# 4. MAGPIE, for deploy.sh's pin check.
magpie=${MAGPIE_DIR:-$HOME/MAGPIE}
if [[ ! -d "$magpie/.git" ]]; then
  if ops_confirm "Clone MAGPIE to $magpie (deploy.sh checks the pin is pushed)?"; then
    git clone https://github.com/jvc56/MAGPIE.git "$magpie"
  else
    ops_say "skipped: set MAGPIE_DIR to your MAGPIE checkout before deploying"
  fi
fi

# 5. The shared state.
backend=$OPS_INFRA/backend.tf
if [[ ! -e "$backend" ]]; then
  cat > "$backend" <<EOF
terraform {
  backend "s3" {
    bucket       = "$STATE_BUCKET"
    key          = "birdtest/terraform.tfstate"
    region       = "$STATE_REGION"
    use_lockfile = true
  }
}
EOF
  ops_say "wrote infra/backend.tf (ignored by git)"
elif ! grep -q "\"$STATE_BUCKET\"" "$backend"; then
  ops_die "infra/backend.tf does not name $STATE_BUCKET: check it against LAUNCH_PLAN Step 5"
fi
terraform -chdir="$OPS_INFRA" init -input=false

# 6. The variables, and 7. the plan that must show nothing.
ops_stack
ops_tfvars_fetch
rc=0
terraform -chdir="$OPS_INFRA" plan -input=false -detailed-exitcode -lock=false "${OPS_VARFILES[@]}" || rc=$?
case $rc in
  0) ops_say "no changes: this machine is ready to deploy (scripts/deploy.sh)" ;;
  2) ops_die "the plan shows changes: stop, and compare with the person who deployed last (an out-of-date main or prod.tfvars, usually)" ;;
  *) ops_die "terraform plan failed (above)" ;;
esac
