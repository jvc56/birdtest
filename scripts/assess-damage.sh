#!/usr/bin/env bash
# RUNBOOK.md §0, "what is the damage?", in one go. Reads only.
#
#   scripts/assess-damage.sh
#
# Through one ops task (scripts/prod-sql.sh): the last twenty destructive
# admin actions from audit_log -- a purge's or deletion's *.census row holds,
# in `reason`, the row counts it was about to destroy: the scope of the
# restore -- and the last five rows of the `backups` table. Then, from this
# machine, the newest five dump manifests in the backups bucket. RUNBOOK §0
# says how to read them.
#
# Needs aws, terraform, jq, the settings in ~/.birdtest-env and infra/
# initialized with the stack's backend.
set -euo pipefail
# shellcheck source=scripts/lib/ops.sh
source "$(dirname "$0")/lib/ops.sh"

[[ $# == 0 ]] || { sed -n '4p' "$0" >&2; exit 2; }

ops_init
ops_need aws terraform jq
ops_load_env
ops_login
ops_stack
bucket=$(tf -raw backups_bucket)

ops_say "the audit log and the backups table (an ops task takes a minute or two):"
INFRA_DIR=$OPS_INFRA "$OPS_ROOT/scripts/prod-sql.sh" "
SET TRANSACTION READ ONLY;
\\echo '-- destructive admin actions, newest first'
SELECT created_at, action, target_id, reason
  FROM audit_log
 WHERE action LIKE '%.census'
    OR action IN ('job.deleted','job.purged','user.deleted','input_data.deleted',
                  'player_config.deleted','worker.unbanned','job.artifacts_rebuild_started')
 ORDER BY created_at DESC LIMIT 20;
\\echo '-- the backups table'
SELECT finished_at, ok, dump_bytes, s3_key FROM backups ORDER BY finished_at DESC LIMIT 5;
"

ops_say "the newest dumps in s3://$bucket/pg/:"
aws s3 ls "s3://$bucket/pg/" | grep manifest | tail -5 || ops_say "no manifest listed"
