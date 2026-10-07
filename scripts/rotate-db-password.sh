#!/usr/bin/env bash
# Rotate the database master password (RUNBOOK.md, "Rotating the database
# password"), and with --signing-key the session signing key too.
#
#   scripts/rotate-db-password.sh [--signing-key] [--resume]
#
# The password is set by hand (RDS does not manage it) and lives only inside
# the DATABASE_URL parameter. This writes the new URL to SSM *first*, then
# sets the password on the instance from what SSM holds, then restarts the
# service so its task reads the new URL (the backup and builder tasks read it
# at each run). So the new password is never only in this shell: if anything
# stops it part-way -- an expired login, a closed laptop -- the site fails new
# connections until  scripts/rotate-db-password.sh --resume  sets the
# instance's password from SSM and restarts the service. --resume never makes
# a new password. Between the two the site fails new database connections, so
# run it at a quiet moment.
#
# --signing-key also writes a new SESSION_SIGNING_KEY, which signs everyone
# out (README "Deploying" makes the first one the same way).
#
# Needs aws, terraform, jq, python3, openssl, curl, the settings in
# ~/.birdtest-env and infra/ initialized with the stack's backend.
set -euo pipefail
# shellcheck source=scripts/lib/ops.sh
source "$(dirname "$0")/lib/ops.sh"

usage() { sed -n '5p' "$0" >&2; exit 2; }
resume=0 signing_key=0
while (($#)); do
  case $1 in
    --resume) resume=1 ;;
    --signing-key) signing_key=1 ;;
    *) usage ;;
  esac
  shift
done

ops_init
ops_need aws terraform jq python3 openssl curl
ops_load_env
ops_login
ops_stack
url_param=$(tf -json ssm_parameter_names | jq -r '.[] | select(endswith("/DATABASE_URL"))')
key_param=$(tf -json ssm_parameter_names | jq -r '.[] | select(endswith("/SESSION_SIGNING_KEY"))')
[[ -n "$url_param" && -n "$key_param" ]] || ops_die "the ssm_parameter_names output does not name both parameters"
instance=$OPS_CLUSTER   # rds.tf: identifier = local.name
master=$(aws rds describe-db-instances --db-instance-identifier "$instance" \
  --query 'DBInstances[0].MasterUsername' --output text) || ops_die "no instance $instance"

get_url() {
  aws ssm get-parameter --name "$url_param" --with-decryption --query Parameter.Value --output text
}
# shellcheck disable=SC2016
url_part() {  # user | password, from the URL in $URL
  URL_PART=$1 python3 -c 'import os, urllib.parse as u
p = u.urlsplit(os.environ["URL"])
print(u.unquote((p.username if os.environ["URL_PART"] == "user" else p.password) or ""))'
}

if ((resume == 0)); then
  ops_say "rotating the password of $instance ($url_param): the site fails new database connections until the service restarts"
  ops_confirm "Go on?" || ops_die "nothing changed"
  URL=$(get_url) || ops_die "could not read $url_param"
  export URL
  [[ "$(url_part user)" == "$master" ]] || ops_die "$url_param's user is not the instance's master user ($master)"
  # Hex: nothing in it to percent-encode.
  NEW_PASSWORD=$(openssl rand -hex 24)
  export NEW_PASSWORD
  # shellcheck disable=SC2016
  new_url=$(python3 -c 'import os, urllib.parse as u
p = u.urlsplit(os.environ["URL"])
user, host = p.netloc.rsplit("@", 1)[0].split(":", 1)[0], p.netloc.rsplit("@", 1)[1]
print(p._replace(netloc=user + ":" + os.environ["NEW_PASSWORD"] + "@" + host).geturl())')
  unset NEW_PASSWORD URL
  # From here on an interrupted run is finished by --resume.
  trap 'rm -rf "$OPS_TMP"; ops_say "STOPPED PART-WAY: $url_param holds the new password. Finish with  scripts/rotate-db-password.sh --resume"' EXIT
  aws ssm put-parameter --name "$url_param" --type SecureString --overwrite --value "$new_url" >/dev/null
  unset new_url
  ops_say "$url_param written"
else
  trap 'rm -rf "$OPS_TMP"; ops_say "STOPPED PART-WAY: run  scripts/rotate-db-password.sh --resume  again"' EXIT
fi

# The instance's password, from what SSM holds.
URL=$(get_url) || ops_die "could not read $url_param"
export URL
[[ "$(url_part user)" == "$master" ]] || ops_die "$url_param's user is not the instance's master user ($master)"
password=$(url_part password)
unset URL
[[ -n "$password" ]] || ops_die "$url_param has no password"
aws rds modify-db-instance --db-instance-identifier "$instance" \
  --master-user-password "$password" --apply-immediately --query 'DBInstance.DBInstanceStatus' --output text >/dev/null
unset password
ops_say "password set on $instance; waiting for it to apply"
sleep 20   # the change starts a moment after the call returns
for round in 1 2 3 4; do
  aws rds wait db-instance-available --db-instance-identifier "$instance" && break
  [[ $round != 4 ]] || ops_die "$instance is still not available after two hours"
  ops_say "still not available; waiting again"
done

if ((signing_key)); then
  aws ssm put-parameter --name "$key_param" --type SecureString --overwrite \
    --value "$(openssl rand -hex 32)" >/dev/null
  ops_say "$key_param written: every session ends"
fi

# Tasks read SSM at start; the backup and builder tasks read it per run.
aws ecs update-service --cluster "$OPS_CLUSTER" --service "$OPS_SERVICE" --force-new-deployment \
  --query 'service.serviceName' --output text >/dev/null
trap 'rm -rf "$OPS_TMP"' EXIT
ops_say "the service is restarting on the new password"
if [[ "$(ops_desired_count)" != 0 ]]; then
  ops_wait_release "$(ops_running_backend_image)"
fi
ops_say "done"
