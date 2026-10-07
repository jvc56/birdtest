#!/usr/bin/env bash
# Empty the production database and start the live release on it, which
# applies 0001_initial.sql afresh (UPDATES_PLAN, "schema changes keep editing
# 0001 in place": until launch, a release that changes 0001 needs this).
#
#   scripts/reset-prod-db.sh [--no-start]
#
# A release that changes 0001 resets as part of its deploy instead:
# scripts/deploy.sh --reset-db does the reset once its plan is approved and
# before the new task starts. This script is for a reset on its own -- the
# live release on an empty database.
#
# Steps: the site's hostname typed (it deletes every account, admin flag, API
# key, job, result and the imported input data), the service stopped (desired
# count 0, by the AWS CLI: Terraform's state keeps saying one task, and the
# next apply agrees with what this leaves), every other session of the
# database's role ended -- a derived-data build (every five minutes) or the
# 03:00 backup, which then mails a failure -- and the public schema dropped
# and made again, through the ops task (scripts/prod-sql.sh). Then, unless
# --no-start, the service started again and waited for until healthy, and the
# checklist printed: register again, scripts/confirm-user.sh --admin, import
# the input data.
#
# While it runs the derived-data builder's schedule goes on starting tasks;
# with no schema they fail and change nothing (the builder never migrates).
# The nightly dumps in the backups bucket are kept, but they are of the old
# schema: restoring one needs the release that wrote it.
#
# Needs aws, terraform, jq, curl, the settings in ~/.birdtest-env and infra/
# initialized with the stack's backend.
set -euo pipefail
# shellcheck source=scripts/lib/ops.sh
source "$(dirname "$0")/lib/ops.sh"

start=1
case ${1:-} in
  "") ;;
  --no-start) start=0 ;;
  *) sed -n '6p' "$0" >&2; exit 2 ;;
esac

ops_init
ops_need aws terraform jq curl
ops_load_env
ops_require STATE_BUCKET STATE_REGION
ops_login
ops_stack
ops_tfvars_fetch
host=$(ops_site_host)
ops_say "live release: $(ops_tfvar backend_image)"

ops_reset_database
if ((start)); then
  ops_start_service
  ops_wait_release "$(ops_tfvar backend_image)"
  if curl -fsS --max-time 20 "https://$host/health" >/dev/null; then
    ops_say "https://$host/health answers"
  else
    ops_say "WARNING: https://$host/health did not answer: look at the site"
  fi
else
  ops_say "the service is left stopped (--no-start): start it with  aws ecs update-service --cluster $OPS_CLUSTER --service $OPS_SERVICE --desired-count 1"
fi
ops_reset_checklist "$host"
