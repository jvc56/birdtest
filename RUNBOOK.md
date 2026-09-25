# birdtest — Recovery Runbook

Copy-pasteable procedures for the scenarios in
[PLAN.md's Backups and Restore](PLAN.md#backups-and-restore). That section
explains why; this one is what to type at 2am. Read the whole procedure before starting any of it.

Placeholders throughout: `$REGION` (default `us-east-1`), `$CLUSTER`
(`birdtest`), `$BUCKET` (the `backups_bucket` Terraform output).

**Where the SQL runs.** The database has no public address and admits only the
service's security group, so no `psql` on an operator's machine can reach it.
Every `psql "$DATABASE_URL" ...` below runs inside the VPC, in the ops task
(`infra/ops.tf`: the postgres image, `DATABASE_URL` already set, read access to
the backups bucket): `scripts/prod-sql.sh "<SQL>"` for one batch of statements,
or `scripts/prod-shell.sh` for an interactive shell (it needs the AWS CLI's
Session Manager plugin) — which is where §2's scratch restore and row copy run,
with the scratch database a Postgres started inside that shell the way
`scripts/restore-drill.sh` starts one.

---

## 0. Before anything: what is the damage?

```bash
# What the admin actions did, and how big the thing they destroyed was.
psql "$DATABASE_URL" -c "
  SELECT created_at, action, target_id, reason
  FROM audit_log
  WHERE action LIKE '%.census' OR action IN ('job.deleted','job.purged','user.deleted')
  ORDER BY created_at DESC LIMIT 20"
```

Every destructive admin endpoint writes a `*.census` row *before* it destroys
anything, so `reason` holds the row counts that were about to be lost. That is
the scope of the restore.

```bash
# What is restorable, and how old it is.
aws s3 ls "s3://$BUCKET/pg/" | grep manifest | tail -5
psql "$DATABASE_URL" -c "SELECT finished_at, ok, dump_bytes, s3_key FROM backups ORDER BY finished_at DESC LIMIT 5"
```

The same information is on `/admin/backups` if the site is up.

---

## 1. Full restore, point in time (instance failure, bad migration, dropped schema)

Loses at most ~5 minutes. Takes under an hour.

```bash
# 1. STOP WRITES. Workers submitting into a database about to be replaced have
#    their results silently discarded.
aws ecs update-service --cluster "$CLUSTER" --service birdtest --desired-count 0 --region "$REGION"

# 2. Pick the restore point: the latest possible instant before the damage.
aws rds describe-db-instances --db-instance-identifier birdtest --region "$REGION" \
  --query 'DBInstances[0].LatestRestorableTime'

# 3. Restore to a NEW instance. The original is left untouched until the
#    restore is confirmed good.
STAMP=$(date -u +%Y%m%d%H%M)
# The source's parameter group (a generated name): a restore without one gets
# the default group and loses the WAL settings leave-generation merges need.
PARAMETER_GROUP=$(aws rds describe-db-instances --region "$REGION" --db-instance-identifier birdtest \
  --query 'DBInstances[0].DBParameterGroups[0].DBParameterGroupName' --output text)
# And its class: the restore serves production from step 5, before the final
# `terraform apply` puts anything else back.
INSTANCE_CLASS=$(aws rds describe-db-instances --region "$REGION" --db-instance-identifier birdtest \
  --query 'DBInstances[0].DBInstanceClass' --output text)
# And a storage ceiling, which a point-in-time restore is not promised to
# carry over: without one the restored instance cannot grow at all. RDS
# refuses a ceiling less than 10% above the allocation, which the source's own
# is once autoscaling has taken it near the top (and it reads `None` if it was
# ever switched off), so it is at least 30% above.
read -r ALLOCATED MAX_STORAGE < <(aws rds describe-db-instances --region "$REGION" \
  --db-instance-identifier birdtest \
  --query 'DBInstances[0].[AllocatedStorage,MaxAllocatedStorage]' --output text)
[[ "$MAX_STORAGE" =~ ^[0-9]+$ ]] || MAX_STORAGE=0
# 130%: RDS warns once allocation passes 80% of the ceiling.
MIN_CEILING=$(( (ALLOCATED * 130 + 99) / 100 ))
(( MAX_STORAGE >= MIN_CEILING )) || MAX_STORAGE=$MIN_CEILING
aws rds restore-db-instance-to-point-in-time --region "$REGION" \
  --source-db-instance-identifier birdtest \
  --target-db-instance-identifier "birdtest-restore-$STAMP" \
  --restore-time '2026-09-07T02:55:00Z' \
  --db-subnet-group-name birdtest-db \
  --vpc-security-group-ids "$(terraform -chdir=infra output -raw db_security_group_id)" \
  --db-parameter-group-name "$PARAMETER_GROUP" \
  --no-publicly-accessible \
  --db-instance-class "$INSTANCE_CLASS" \
  --max-allocated-storage "$MAX_STORAGE"

aws rds wait db-instance-available --region "$REGION" \
  --db-instance-identifier "birdtest-restore-$STAMP"
```

A restored instance does **not** inherit the source's backup settings. Fix that
before it becomes the production database:

```bash
aws rds modify-db-instance --region "$REGION" \
  --db-instance-identifier "birdtest-restore-$STAMP" \
  --backup-retention-period 30 --deletion-protection --apply-immediately
```

Swap the names, and hand the restored instance to Terraform. The rename moves
the endpoint with it, which is why it comes before repointing. A rename returns
before it takes effect, and `wait db-instance-available` on a name that does not
exist yet fails at once rather than waiting, so each wait is preceded by a poll
for the new name:

```bash
renamed() {  # wait until instance $1 exists under its new name, then until it is available
  until aws rds describe-db-instances --region "$REGION" --db-instance-identifier "$1" \
      >/dev/null 2>&1; do sleep 10; done
  aws rds wait db-instance-available --region "$REGION" --db-instance-identifier "$1"
}
aws rds modify-db-instance --region "$REGION" --db-instance-identifier birdtest \
  --new-db-instance-identifier "birdtest-damaged-$STAMP" --apply-immediately
renamed "birdtest-damaged-$STAMP"
aws rds modify-db-instance --region "$REGION" --db-instance-identifier "birdtest-restore-$STAMP" \
  --new-db-instance-identifier birdtest --apply-immediately
renamed birdtest

# Terraform's state holds the damaged instance by its resource id (db-...),
# not by name, so it would follow the damaged one under its new name. Point it
# at the restored one instead; import takes the identifier.
# With the stack's variables (README.md "Deploying" keeps them in
# infra/prod.tfvars): import evaluates the whole configuration, in the
# stack's region. The import must succeed before going on -- after the
# `state rm`, a failed import leaves nothing in state for the next apply.
terraform -chdir=infra state rm aws_db_instance.main
terraform -chdir=infra import -var-file=prod.tfvars aws_db_instance.main birdtest
```

Left under its restore name, or left out of the state, the next `terraform
apply` would find `birdtest` missing once the damaged one was retired and
create a new, empty database in its place.

Repoint the application. The master password is set by hand and lives only in
the `DATABASE_URL` parameter, so keep it and swap the host. A PITR copy keeps
the password the source had at the restore point; if it was rotated after that
point, set it on the new instance first (see "Rotating the database password"):

```bash
ENDPOINT=$(aws rds describe-db-instances --region "$REGION" \
  --db-instance-identifier birdtest \
  --query 'DBInstances[0].Endpoint.Address' --output text)

OLD_URL=$(aws ssm get-parameter --region "$REGION" --name /birdtest/DATABASE_URL \
  --with-decryption --query Parameter.Value --output text)
NEW_URL=$(python3 -c 'import sys, urllib.parse as u
p = u.urlsplit(sys.argv[1])
print(p._replace(netloc=p.netloc.rsplit("@", 1)[0] + "@" + sys.argv[2] + ":5432").geturl())' \
  "$OLD_URL" "$ENDPOINT")

aws ssm put-parameter --region "$REGION" --name /birdtest/DATABASE_URL --type SecureString --overwrite \
  --value "$NEW_URL"

# Tasks read SSM at start, so this is the whole deploy.
aws ecs update-service --cluster "$CLUSTER" --service birdtest --desired-count 1 --region "$REGION"
```

(If the ceiling had to be raised above the stack's `db_allocated_storage * 5`,
raise `db_allocated_storage` in `prod.tfvars` before the closing apply below:
it sets the ceiling back to five times that, and RDS refuses one less than a
tenth above the current allocation. The allocation itself Terraform leaves
alone — autoscaling owns it.)

Then run §4 (verification), **Check artifacts** on every leave-generation job
(§3: the database now describes the objects as they were at the restore point,
and the button makes what workers are sent match what the bucket holds), and
`terraform apply -var-file=prod.tfvars`: the restore set none of
Multi-AZ, the backup window or tag copying, and the apply puts them back. Only
once §4 passes, retire `birdtest-damaged-$STAMP` — never before.

**What the fleet does meanwhile.** Workers go on playing the tasks they hold.
A worker asking for a task keeps asking, once a minute, for as long as the
server is away, and picks up again by itself when it is back. A worker
*submitting* retries for about fifteen minutes and then ends its `contribute`
run — its claim will have lapsed anyway — so an outage this long stops the
contributors that finished a task in the middle of it, and only they can start
again. A routine deployment, a minute or two, stops nobody. When the server comes back it reclaims **no** claim until it has been
up for the heartbeat timeout (`HEARTBEAT_TIMEOUT_SECONDS`, 300): a claim's
evidence of life is a heartbeat, and nobody could deliver one while the server
was away. Workers still running are heard from within thirty seconds and their
results are accepted; the claims of those that gave up lapse after the grace
and are handed out again. Claims issued after the restore point do not exist in
the restored database, so results for them are answered `accepted: false`.

---

## 2. Selective restore (a mistaken purge or delete)

The database must **not** be rolled back: everything else has moved on. The
shape is always the same — stop the job, restore a copy somewhere else, clear
out what the job has done since, copy the missing rows across, repair the
counters, recompute the derived state.

### 2.0 Stop the job, and clear what it has done since

A purge leaves the job's status alone, so an active job goes straight on
dispatching: its seed cursor is back at zero, a leave job re-seeds generation
1, and every new task takes a `(job_id, seed)` — and every new progress row a
`(job_id, generation, rack)` — that the restored rows need. Copied over them
with `ON CONFLICT DO NOTHING`, the restored rows lose silently, their claims and
results then fail their foreign keys, and the restore reports success with the
contributors' work still gone. So first, from the admin page or the API,
**deactivate the job** (a purged completed job is already inactive). Then
delete what it has generated since the purge — nothing of it predates the
mistake — in one transaction, in the ops shell. The SQL here and in §2.3 reads
the job's id as `:'job'`, so start psql with it set:

```bash
psql "$DATABASE_URL" -v job=00000000-0000-0000-0000-000000000000
```

```sql
BEGIN;
DELETE FROM task_claims c USING tasks t WHERE c.task_id = t.id AND t.job_id = :'job';
DELETE FROM tasks WHERE job_id = :'job';
DELETE FROM leave_rack_progress         WHERE job_id = :'job';
DELETE FROM leave_rack_staging          WHERE job_id = :'job';
DELETE FROM leave_generation_progress   WHERE job_id = :'job';
DELETE FROM leave_selection_cursors     WHERE job_id = :'job';
DELETE FROM leave_generation_artifacts  WHERE job_id = :'job';
DELETE FROM leave_generation_transitions WHERE job_id = :'job';
COMMIT;
```

(A deleted job has nothing to clear: re-insert its `jobs` row and its config
rows from the scratch copy first, then the rest as below.) After this, any
conflict in §2.2 means this step was missed — not that the row is safe to
skip.

### 2.1 Get a copy of the old data

Either a PITR instance from just before the mistake (fresher, §1 steps 2–3 with
a `-scratch-` identifier and no repointing), or the latest nightly dump
restored into a Postgres of the ops shell's own (`scripts/prod-shell.sh`), the
way the monthly drill does it. The shell's task stops itself after
`SHELL_HOURS` (default 4); a large dump on the task's one vCPU can take longer,
so start it with `SHELL_HOURS=12 scripts/prod-shell.sh` for anything big:

First fetch the dump and check it against its manifest, as the drill does
(`scripts/restore-drill.sh`, `dump_digest`): a partial download, or a replica
whose files were still arriving, restores part of a database and says nothing.
This block can be run again as it is; do not go on to the next until it says
`checksum matches`.

```bash
# Inside scripts/prod-shell.sh.
apt-get update -qq && apt-get install -y -qq awscli >/dev/null
STAMP=2026-09-07T03-00-00Z
BUCKET=${BUCKET:-$BACKUP_BUCKET}   # the stack's own; set BUCKET/S3_REGION only to read another
rm -rf /tmp/dump   # a re-fetch must not keep files from an earlier one
aws s3 cp ${S3_REGION:+--region $S3_REGION} "s3://$BUCKET/pg/$STAMP/dump" /tmp/dump --recursive
aws s3 cp ${S3_REGION:+--region $S3_REGION} "s3://$BUCKET/pg/$STAMP.manifest.json" /tmp/manifest.json
want=$(grep -o '"sha256"[^,}]*' /tmp/manifest.json | sed 's/.*: *"//;s/"//')
got=$( (cd /tmp/dump && find . -type f | LC_ALL=C sort | xargs -r sha256sum) | sha256sum | cut -d' ' -f1)
[ -n "$want" ] && [ "$want" = "$got" ] && echo "checksum matches" \
  || echo "CHECKSUM MISMATCH ($want vs $got): run this block again; do not restore"
```

Then restore it. If an earlier attempt got part of the way, start clean first:
`gosu postgres pg_ctl -D /tmp/scratch stop; rm -rf /tmp/scratch`.

```bash
mkdir -p /tmp/scratch /tmp/sock && chown postgres /tmp/scratch /tmp/sock
gosu postgres initdb -D /tmp/scratch -U postgres --auth=trust >/dev/null
# As the drill starts its own (scripts/restore-drill.sh): no parallel query,
# whose workers share memory through the task's small /dev/shm and fail a big
# join part-way, and no durability, which a scratch copy does not need.
gosu postgres pg_ctl -D /tmp/scratch -w -l /tmp/scratch.log start \
  -o "-c listen_addresses='' -c unix_socket_directories=/tmp/sock \
      -c max_parallel_workers_per_gather=0 -c fsync=off -c full_page_writes=off \
      -c synchronous_commit=off"
SCRATCH_URL="postgresql:///birdtest_scratch?host=/tmp/sock&user=postgres"
# Kept in a file: an `--attach`ed shell, or the detached §2.2 script, does not
# inherit this one's variables. `source /tmp/restore.env` in either.
echo "export SCRATCH_URL='$SCRATCH_URL'" > /tmp/restore.env
createdb -h /tmp/sock -U postgres birdtest_scratch
# Detached, so the ECS Exec session ending (twenty idle minutes, a laptop
# asleep) does not end the restore; `scripts/prod-shell.sh --attach <task>`
# comes back to it.
# pg_restore prints nothing when it succeeds, so the last line says it ended.
setsid nohup sh -c 'pg_restore -d "$0" -j4 --no-owner --no-privileges \
  --exit-on-error /tmp/dump; echo "pg_restore exit $?"' "$SCRATCH_URL" \
  > /tmp/pg_restore.log 2>&1 &
tail -f /tmp/pg_restore.log   # Ctrl-C leaves the restore running
```

Do not start §2.2 until the log's last line is `pg_restore exit 0`: with `-j4`
each table commits on its own, so a copy-back taken mid-restore finds some of
the job's tables loaded and others empty, and reports success.

The §2.2 loop is best run the same way (`setsid nohup bash /tmp/copyback.sh >
/tmp/copyback.log 2>&1 &`, with the script written to a file first).

### 2.2 Copy the rows back, in dependency order

Dump only the job's rows from the scratch copy, a file per table, and load them
into production (`$DATABASE_URL`, in the same shell) in dependency order. Each
file is loaded into a temporary table and inserted from there with `ON CONFLICT
DO NOTHING`, so a partial re-run is safe — which is all it is for, after §2.0.
(`COPY` itself has no `ON CONFLICT`: loaded straight in, a re-run stopped at the
first row already there.)

For a large job, look at the space first. The copy-back adds the job's rows to
every table, with their indexes, and stages each table's in an unindexed
temporary table on the production volume first, plus the WAL the inserts
write; autoscaling keeps only about a tenth of the volume free and grows it at
most every six hours. Run the script once with `COPYBACK_DUMP_ONLY=1`: it dumps
the job's rows and prints their sizes, and stops before loading anything. Allow
about twice the total (measured: the loaded rows and their indexes came to 1.5
times the text), plus `db_max_wal_size_mb` (4 GiB by default) for the WAL the
inserts write (3.2 times the text, measured). Compare with `FreeStorageSpace`,
raise `db_allocated_storage` first if it is close, then run it again without.

Save this as `/tmp/copyback.sh` — `cat > /tmp/copyback.sh <<'EOF'` … `EOF`,
the quotes mattering: unquoted, the shell fills in `$JOB` and the rest as it
writes the file — and run it (`bash /tmp/copyback.sh`, or detached as §2.1
runs `pg_restore`), not pasted: its `exit` on a failure would end the
interactive shell.

From a PITR scratch instance rather than a dump there is no restore to wait
for; write what the script reads first:

```bash
# The production URL with the scratch instance's endpoint for its host: a PITR
# restore keeps the source's master password, already encoded in the URL as
# it must be. sed, not python: the ops image (postgres:16) has no python3.
# The last '@' ends the password (one in it is encoded); the host runs to the
# path or query. %q, so no character in the URL can break the file.
ENDPOINT="<the scratch instance's endpoint>"
ENDPOINT=${ENDPOINT%%:*}   # a pasted ":5432" would be doubled
# The userinfo is everything up to the '@' before the first '/', '?' or '#'
# after the scheme: an '@' in a query string is not it.
SCRATCH_URL=$(printf '%s' "$DATABASE_URL" | sed -E "s#^([a-z]+://[^/?\#]*@)[^/?\#]+#\1$ENDPOINT:5432#")
case "$SCRATCH_URL" in *"@$ENDPOINT:5432"*) ;; *) echo "could not build SCRATCH_URL: stop"; SCRATCH_URL= ;; esac
[ -n "$SCRATCH_URL" ] && printf 'export SCRATCH_URL=%q\n' "$SCRATCH_URL" > /tmp/restore.env \
  && echo 'pg_restore exit 0' > /tmp/pg_restore.log   # nothing to wait for
```

```bash
source /tmp/restore.env
grep -qx 'pg_restore exit 0' /tmp/pg_restore.log \
  || { echo "the scratch restore has not finished, or failed" >&2; exit 1; }
# The dump is in the scratch database now; its files would share the task's
# disk with the job's rows dumped below.
rm -rf /tmp/dump
JOB=00000000-0000-0000-0000-000000000000
TASKS="SELECT id FROM tasks WHERE job_id = '$JOB'"
RECORDS="SELECT id FROM position_analysis_records WHERE job_id = '$JOB'"
MOVES="SELECT id FROM position_analysis_moves WHERE record_id IN ($RECORDS)"
mkdir -p /tmp/restore

# Order matters: tasks, then what hangs off a task, then claims, then what
# hangs off a claim.
TABLES=(
  "tasks|job_id = '$JOB'"
  "opening_rack_requests|task_id IN ($TASKS)"
  "game_requests|task_id IN ($TASKS)"
  "leave_requests|task_id IN ($TASKS)"
  "task_claims|task_id IN ($TASKS)"
  "worker_data_gaps|job_id = '$JOB'"
  "game_results|job_id = '$JOB'"
  "leave_records|task_id IN ($TASKS)"
  "position_analysis_records|job_id = '$JOB'"
  "position_analysis_moves|record_id IN ($RECORDS)"
  "position_analysis_plies|move_id IN ($MOVES)"
  "leave_rack_progress|job_id = '$JOB'"
  "leave_rack_staging|job_id = '$JOB'"
  "leave_generation_progress|job_id = '$JOB'"
  "leave_selection_cursors|job_id = '$JOB'"
  "leave_generation_artifacts|job_id = '$JOB'"
  "leave_generation_transitions|job_id = '$JOB'"
)

for entry in "${TABLES[@]}"; do
  table=${entry%%|*} filter=${entry#*|}
  psql "$SCRATCH_URL" -v ON_ERROR_STOP=1 -c \
    "COPY (SELECT * FROM $table WHERE $filter) TO STDOUT" > "/tmp/restore/$table" \
    || { echo "stopped: could not dump $table" >&2; exit 1; }
done
du -ch /tmp/restore/* | sort -h | tail -6   # the job's own rows, largest last
if [ -n "${COPYBACK_DUMP_ONLY:-}" ]; then
  echo "dumped only; compare the sizes above with FreeStorageSpace"; exit 0
fi

for entry in "${TABLES[@]}"; do
  table=${entry%%|*}
  psql "$DATABASE_URL" -v ON_ERROR_STOP=1 <<SQL
BEGIN;
CREATE TEMP TABLE restoring (LIKE $table) ON COMMIT DROP;
\copy restoring FROM '/tmp/restore/$table'
INSERT INTO $table SELECT * FROM restoring ON CONFLICT DO NOTHING;
COMMIT;
SQL
  # Every later table hangs off this one: carrying on buried the first error
  # under a foreign-key failure per table.
  [[ $? -eq 0 ]] || { echo "stopped: could not load $table" >&2; exit 1; }
done
```

| Order | Table | Filter |
|---|---|---|
| 1 | `tasks` | `job_id = :job` |
| 2 | `opening_rack_requests` / `game_requests` / `leave_requests` | `task_id IN (...)` |
| 3 | `task_claims`, then `worker_data_gaps` | `task_id IN (...)` / `job_id = :job` |
| 4 | `game_results`, `leave_records` | `job_id = :job` / `task_id IN (...)` |
| 5 | `position_analysis_records` → `_moves` → `_plies` | `job_id = :job`, then by parent id |
| 6 | `leave_rack_progress`, `leave_rack_staging`, `leave_generation_progress`, `leave_selection_cursors`, `leave_generation_artifacts`, `leave_generation_transitions` | `job_id = :job` |

`worker_data_gaps` is what the admin page's data gaps and the job list's
`stalled` flag read: left out, a job's declines are forgotten.

Restoring a *deleted* job also means its `jobs` row and its config row
(`job_game_config`, `job_game_pair_config`, `job_opening_rack_config` or
`job_leave_config`) first, from the scratch copy the same way — and any
`player_configs` row they name that has been deleted since (nothing pinned it
once the job was gone), with its `input_data` rows if those went too.

Ratings are not in this list: they belong to rating pools rather than jobs, and
are recomputed from `game_results` (see §2.4).

`leave_generation_transitions` is in that list for a reason: a generation's
artifact row without its transition row would leave the next claim free to
re-run a transition that already happened, and a transition row without its
artifact row would stall the job until the takeover timeout. Copy both or
neither.

`leave_rack_staging` goes with `leave_rack_progress` for the same kind of
reason: it holds the accepted leave results that have not yet been merged into
the per-rack totals (`leave_gen::merge_staged`: half-hourly, near a generation's
end, and before one closes). Progress without its staging rows is a generation
that silently lost up to half an hour of accepted work, and the tasks that did
it still read as completed, so nothing would ever redo it. Copy both from the
same snapshot. `leave_generation_progress` is the dashboard's per-generation
summary; its rack figures are rebuilt by the next merge
(`POST /api/admin/jobs/:id/merge-progress` forces one), so only its live
counters are lost if it is left out.

`game_results` and `position_analysis_records` carry `job_id` as well as
`task_id`, so those two can be selected by the job directly rather than through
a task list — which is also how every read of them works. `_moves` and `_plies`
still hang off their parent's id.

The job's counters are repaired in §2.3, and the contributors' in §2.3b — the
latter globally rather than per job, because an identity's total spans every job
it has worked on.

`position_analysis_records.id`, `_moves.id` and `leave_rack_staging.id` are
`BIGSERIAL`, restored with their original ids so the parent-child links hold.
Those ids came from these sequences, which a selective restore does not rewind,
so nothing needs moving; the check below only ever moves a sequence forward
(a plain `setval(..., max(id))` could move it *back* below ids the running
fleet had taken since, and the next insert collided; each statement below
sets nothing unless the table is ahead of its sequence):

```sql
SELECT setval('position_analysis_records_id_seq', m)
  FROM (SELECT max(id) AS m FROM position_analysis_records) x
 WHERE m > (SELECT last_value FROM position_analysis_records_id_seq);
SELECT setval('position_analysis_moves_id_seq', m)
  FROM (SELECT max(id) AS m FROM position_analysis_moves) x
 WHERE m > (SELECT last_value FROM position_analysis_moves_id_seq);
SELECT setval('leave_rack_staging_id_seq', m)
  FROM (SELECT max(id) AS m FROM leave_rack_staging) x
 WHERE m > (SELECT last_value FROM leave_rack_staging_id_seq);
```

### 2.3 Repair the counters

This is the step a naive row copy gets wrong, and the one that makes the
scheduler dispatch work that is already done. Run it against production after
the copy, in one transaction:

```sql
BEGIN;

UPDATE tasks t
   SET accepted_count     = actual.accepted,
       active_claim_count = actual.active
  FROM (
    SELECT t2.id,
           count(*) FILTER (WHERE c.state = 'completed')::int AS accepted,
           count(*) FILTER (WHERE c.state = 'claimed')::int   AS active
      FROM tasks t2
      LEFT JOIN task_claims c ON c.task_id = t2.id
     WHERE t2.job_id = :'job'
     GROUP BY t2.id
  ) actual
 WHERE t.id = actual.id
   -- Only rows that are wrong: every task of a large job rewritten was
   -- seconds of writes and as many dead tuples, for rows already right.
   AND (t.accepted_count, t.active_claim_count) IS DISTINCT FROM (actual.accepted, actual.active);

-- State and completed_at follow from the counters and the job's redundancy,
-- exactly as the submit path computes them.
UPDATE tasks t
   SET state = CASE
         WHEN t.accepted_count >= j.redundancy THEN 'completed'::task_state
         WHEN t.accepted_count + t.active_claim_count >= j.redundancy THEN 'claimed'::task_state
         ELSE 'available'::task_state
       END,
       completed_at = CASE WHEN t.accepted_count >= j.redundancy
                           THEN COALESCE(t.completed_at, now()) ELSE NULL END
  FROM jobs j
 WHERE j.id = t.job_id AND t.job_id = :'job'
   AND t.state IS DISTINCT FROM CASE
         WHEN t.accepted_count >= j.redundancy THEN 'completed'::task_state
         WHEN t.accepted_count + t.active_claim_count >= j.redundancy THEN 'claimed'::task_state
         ELSE 'available'::task_state
       END;

-- The job's own counters. claims_issued is the scheduler's deficit numerator,
-- measured from claims_baseline; the statement after this one puts the
-- restored job level with the jobs beside it, as activation and purge do
-- (scheduler::join_at_parity), so it neither owes nor is owed a backlog. The
-- rest are the dashboard's progress totals; they
-- are maintained one task at a time in the claim and submit paths, so a row
-- copy leaves them describing the results the job had before. Each is
-- recomputed here exactly as the read it replaced computed it: one result per
-- task, because redundant claims replay the same work.
-- The claims are read once, for both of their columns: a second subquery for
-- last_completed_at was a second pass over them.
UPDATE jobs j
   SET claims_issued = cl.issued,
       last_completed_at = cl.last,
       tasks_total = (SELECT count(*) FROM tasks t WHERE t.job_id = j.id),
       tasks_completed = (SELECT count(*) FROM tasks t
                           WHERE t.job_id = j.id AND t.state = 'completed'),
       games_completed = (SELECT COALESCE(sum(g.games), 0) FROM (
                            SELECT DISTINCT ON (r.task_id) r.games
                              FROM game_results r
                             WHERE r.job_id = j.id
                             ORDER BY r.task_id, r.submitted_at, r.task_claim_id
                          ) g),
       racks_analyzed = (SELECT count(DISTINCT p.rack)
                           FROM position_analysis_records p
                          WHERE p.job_id = j.id AND p.game_index IS NULL)
  FROM (SELECT count(*) AS issued,
               max(c.completed_at) FILTER (WHERE c.state = 'completed') AS last
          FROM task_claims c JOIN tasks t ON t.id = c.task_id
         WHERE t.job_id = :'job') cl
 WHERE j.id = :'job';

-- Level with the jobs being *served* -- those that issued a claim within the
-- heartbeat timeout (300 s unless HEARTBEAT_TIMEOUT_SECONDS says otherwise) --
-- or, when none has, with every job on offer: scheduler::join_at_parity's rule.
WITH others AS (
  SELECT (o.claims_issued - o.claims_baseline)::float8 / o.allocation AS ratio,
         COALESCE(o.last_claimed_at > now() - interval '300 seconds', FALSE) AS served
    FROM jobs o
   WHERE o.status = 'active' AND o.allocation > 0 AND o.id <> :'job'
)
UPDATE jobs j
   SET claims_baseline = j.claims_issued - floor(
         COALESCE((SELECT MIN(ratio) FROM others WHERE served),
                  (SELECT MIN(ratio) FROM others), 0)
         * COALESCE(j.allocation, 0))::bigint
 WHERE j.id = :'job';

COMMIT;
```

`racks_analyzed` is meaningful only for an opening-rack job and
`games_completed` only for a games or game-pairs job; the statement above leaves
each at 0 for the job types that do not use it, which is what they hold anyway.

Run `tasks_total`/`tasks_completed` before §2.4's state repair or after it, but
not between the two `UPDATE tasks` statements above: `tasks_completed` counts
tasks whose `state` is `completed`, which the second of those recomputes.

**A purged job that had completed** comes back inactive with no verdict: a
purge returns a completed job to inactive and clears the SPRT verdict it was
completed on, and neither is a row the copy brings back. Put both back from the
scratch copy's `jobs` row (for a deleted job, the whole row came back in §2.0):

```sql
-- Set each variable from the scratch copy's row
--   SELECT status, sprt_decided_status, sprt_decided_llr, sprt_decided_units
--   FROM jobs WHERE id = :'job';
-- and to the empty string where it is NULL (a job an admin completed has no
-- verdict): :'var' always quotes, so NULLIF is what turns empty back into NULL.
UPDATE jobs SET status = :'old_status',
               sprt_decided_status = NULLIF(:'old_verdict', ''),
               sprt_decided_llr    = NULLIF(:'old_llr', '')::float8,
               sprt_decided_units  = NULLIF(:'old_units', '')::bigint
 WHERE id = :'job';
```

Activating it instead would dispatch more work on a job that had finished.

### 2.3b Repair the contributor counters

The counters on `users` and `anonymous_workers` are not scoped to one job — they
span every job an identity ever worked on — so a partial restore of one job
cannot repair them in isolation the way §2.3 repairs the job's own. Recompute
them globally, once, after every job has been restored — in the ops shell's
psql (`scripts/prod-shell.sh`): the loops below commit as they go, which
`scripts/prod-sql.sh`, running everything as one transaction, refuses.

**Write it to a file and run it with `psql -f`**, not pasted: pasted into the
interactive psql, an error stops only the statement it is in and the rest of
the paste runs on, and pasting it again in the same session found the old
snapshot's tables already there and applied that snapshot — zeroing everyone
whose first result came after it. Run as a file, psql stops at the first error
and exits, the temporary tables go with the session, and running the file
again starts from a fresh count; rows already right are skipped, so a re-run
costs little.

Count first, into a temporary table, then apply in small batches. A single
`UPDATE` over every account would hold each one's row lock until it finished,
and every submission increments its worker's counter: submissions would wait,
then be answered 503 after five seconds, for as long as the recount ran.

```sql
-- /tmp/recount.sql -- written with  cat > /tmp/recount.sql <<'EOF' ... EOF
-- (quoted: unquoted, the shell turns each $$ below into its process id) --
-- and run as  psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f /tmp/recount.sql
-- Room for the temporary tables below in memory; they are read once per batch.
-- (Set before any temporary table is touched, which a fresh session is.)
SET temp_buffers = '64MB';

-- 1. Count. Reads only; nothing waits on this. `recount` is kept whole for
--    the rest of the procedure (step 2 reads it); step 3 works through a copy.
CREATE TEMP TABLE recount AS
SELECT 'u' AS kind, c.claimed_by_user_id AS id, count(*) AS n, max(c.completed_at) AS last
  FROM task_claims c
 WHERE c.state = 'completed' AND c.claimed_by_user_id IS NOT NULL
 GROUP BY 2
UNION ALL
SELECT 'a', c.claimed_by_anon_uuid, count(*), max(c.completed_at)
  FROM task_claims c
 WHERE c.state = 'completed' AND c.claimed_by_anon_uuid IS NOT NULL
 GROUP BY 2;
CREATE INDEX ON recount (kind, id);
ANALYZE recount;
CREATE TEMP TABLE pending AS SELECT * FROM recount;

-- 2. Zero the identities with no completed claims left. Required after
--    §2.0, which deletes the claims made after the purge: the counters those
--    claims raised would otherwise stay, with nothing behind them. (An
--    identity with no completed claims appears nowhere in step 1.)
-- 3. Apply the counts, a thousand a transaction, only where they differ.
--    Each batch is taken out of `pending` as it is applied, so no batch
--    rereads the ones before it. A run stopped by a lock timeout -- it gives
--    up rather than queue behind a submission -- is run again, the whole
--    file, from a fresh count.
SET lock_timeout = '2s';

DO $$
DECLARE zeroed int;
BEGIN
  LOOP
    WITH z AS (
      UPDATE users SET tasks_completed = 0, last_completed_at = NULL
       WHERE id IN (SELECT u.id FROM users u
                     WHERE u.tasks_completed > 0
                       AND NOT EXISTS (SELECT 1 FROM recount r
                                        WHERE r.kind = 'u' AND r.id = u.id)
                     LIMIT 1000)
      RETURNING 1),
    za AS (
      UPDATE anonymous_workers SET tasks_completed = 0, last_completed_at = NULL
       WHERE uuid IN (SELECT w.uuid FROM anonymous_workers w
                       WHERE w.tasks_completed > 0
                         AND NOT EXISTS (SELECT 1 FROM recount r
                                          WHERE r.kind = 'a' AND r.id = w.uuid)
                       LIMIT 1000)
      RETURNING 1)
    SELECT (SELECT count(*) FROM z) + (SELECT count(*) FROM za) INTO zeroed;
    EXIT WHEN zeroed = 0;
    COMMIT;
  END LOOP;
END $$;

DO $$
DECLARE taken int;
BEGIN
  LOOP
    WITH batch AS (
      DELETE FROM pending
       WHERE ctid IN (SELECT ctid FROM pending LIMIT 1000)
      RETURNING kind, id, n, last),
    u AS (
      UPDATE users u SET tasks_completed = b.n, last_completed_at = b.last
        FROM batch b
       WHERE b.kind = 'u' AND u.id = b.id
         AND (u.tasks_completed, u.last_completed_at) IS DISTINCT FROM (b.n, b.last)
      RETURNING 1),
    a AS (
      UPDATE anonymous_workers w SET tasks_completed = b.n, last_completed_at = b.last
        FROM batch b
       WHERE b.kind = 'a' AND w.uuid = b.id
         AND (w.tasks_completed, w.last_completed_at) IS DISTINCT FROM (b.n, b.last)
      RETURNING 1)
    SELECT count(*) INTO taken FROM batch;
    EXIT WHEN taken = 0;
    COMMIT;
  END LOOP;
END $$;

-- Then §4's check 3b, which counts from the claims afresh: 0 when done.
```

The counts are a snapshot: a submission accepted between step 1 and a row's
update is overwritten by the older figure. Run this when the fleet is quiet, or
run the file again afterwards; a second run changes only what moved. §4
checks the result.

This is the one recount a restore is most likely to need, and the one most
likely to be forgotten: nothing about a single job's restore makes a wrong
leaderboard visible.

### 2.4 Recompute derived state

- **Job exports** (`job_exports`): derived data, and the one thing here that a
  partial restore can make actively misleading — a row still saying `ready`
  describes results the restore may not have brought back, and hands an admin a
  stable-looking artifact of something else. Delete the job's rows
  (`DELETE FROM job_exports WHERE job_id = :'job'`) and re-export if anyone
  wants one; the objects behind them expire from the bucket on their own.

- **Ratings** (`rating_runs` / `player_config_ratings`): a batch fit over
  each pool's `game_results`, never applied per submission. Once the results
  are back the two-minute sweep notices the pool's evidence changed and refits
  it; `POST /api/admin/rating-pools/:id/recompute` does it immediately. Nothing
  to copy. Runs older than a month are thinned to one a day in any case
  (PLAN.md, "Ratings"), so a restored history is at that resolution past the
  month whatever the backup's age.
- **SPRT**: computed from `game_results` on read, so it corrects itself once
  the results are back.
- **Leave-generation artifacts**: run `POST /api/admin/jobs/:id/rebuild-artifacts`
  (the "Check artifacts" button on the admin job page) on every restored
  leave-generation job, whether or not an object is missing: it rebuilds the
  missing ones, and makes the hash workers are sent match the object each
  generation's key now holds — restored rows describe the objects as they were,
  and a worker refuses a KLV whose hash does not match. See §3.
- **Derived data** (`derived_data`): the SHA-256 of each wordmap and rack info
  table the server built. Derived by definition, and **a job whose rows are
  missing does not dispatch** — which is the symptom a restore produces here:
  active jobs handing out nothing. Activating each job again re-queues them —
  the **Activate** button on its admin page, with the allocation it had
  (`POST /api/admin/jobs/:id/activate` takes `{"allocation": N}` and the CSRF
  header, and a different `N` changes the job's share) — and the builder task
  fills them in within a few minutes; `/admin/derived-data` shows the queue. The files
  themselves are not restored because none is kept: the server hashes and
  discards them.

  A row whose `kwg_id` or `klv_id` points at an `input_data` row restored
  without its object-store bytes will fail with that as its reason. Re-import
  that tarball; the import is idempotent and adds no rows for files whose bytes
  have not changed.

---

## 3. A missing or altered artifact

KLVs are derivable from `leave_rack_progress`, so they need no backup:

- **Missing object.** Admin → the job → **Check artifacts**. Every generation
  whose object is gone is rebuilt from the database and rewritten; every
  generation that is present is left alone.
- **Object present but the hash differs** from what was recorded when the
  generation closed. This is *not* automatically overwritten, and usually
  should not be: the results have moved on since the generation closed, so a
  rebuild legitimately produces different bytes, and rewriting would replace
  the KLV that workers actually played with. Investigate before forcing
  (**Force rebuild** on the admin job page; `?force=true` on the API), which
  rewrites every generation and is refused while the job is active. A check
  or a forced rebuild that started before a purge goes on writing objects
  after it; if §2 then copies the old rows back, run **Check artifacts**
  again, which puts the recorded builds back.
- **Object nothing accounts for** (the **Served** column says so): the bytes
  are neither the recorded build, nor the rebuild, nor what workers were being
  sent — typically another run's KLV under the same key, after a purge, a
  re-run and §2's copy-back of the old rows. When the rows reproduce the
  recorded build exactly the check puts that build back itself; otherwise
  workers refuse the object until you restore the version that matches the
  recorded hash (below) or force a rebuild.
- **Built by a different builder.** Read this column first. MAGPIE builds these
  KLVs, so a MAGPIE upgrade can legitimately change the bytes for the same
  leave values; the report says which builder wrote the artifact and which one
  rebuilt it. Only two artifacts from the *same* builder disagreeing is
  evidence of anything.
- **Corrupted object with a known-good older version.** The bucket is
  versioned; restore that specific object version rather than rolling the
  bucket back:

```bash
aws s3api list-object-versions --bucket "$ARTIFACTS_BUCKET" --prefix "leaves/$JOB/"
aws s3api copy-object --bucket "$ARTIFACTS_BUCKET" \
  --copy-source "$ARTIFACTS_BUCKET/leaves/$JOB/generation-3.klv2?versionId=$VERSION" \
  --key "leaves/$JOB/generation-3.klv2"
```

Then **Check artifacts** (without `force`). Workers verify the KLV they fetch
against the hash they are sent, and that hash still describes the object the
copy replaced: until the check records the restored object's hash, every task
of the next generation is declined.

Never restore the artifact bucket wholesale to an older point: the bucket is
allowed to be newer than the database, never older (PLAN.md, "Artifacts: back up, or rebuild?").

---

## 4. Verifying a restore

Do not declare it finished because the page loads.

```bash
# 1. Row counts, against the manifest of the dump that was restored (after a
#    dump restore, §2.1 or §5: STAMP is the dump's stamp. After §1's PITR there
#    is no dump; skip this one).
aws s3 cp "s3://$BUCKET/pg/$STAMP.manifest.json" - | python3 -m json.tool | head -40
```

```sql
-- 2. Referential sanity.
SELECT count(*) AS jobs_missing_data FROM jobs j
 WHERE NOT EXISTS (SELECT 1 FROM input_data d WHERE d.id = j.letterdist_id)
    OR NOT EXISTS (SELECT 1 FROM input_data d WHERE d.id = j.layout_id);

SELECT count(*) AS inputs_missing_content FROM input_data
 WHERE role IN ('letterdist','layout') AND content IS NULL;

-- 3. Counter sanity. Must be zero. Serial: in parallel, each worker builds
--    the whole grouped aggregate of the claims itself.
SET max_parallel_workers_per_gather = 0;
SELECT count(*) AS counter_disagreements
  FROM tasks t
  LEFT JOIN (
    -- One pass over the claims, grouped: a per-task probe (as a lateral
    -- join) was a random index read per task, hours on the drill's disk
    -- at tens of millions of tasks.
    SELECT c.task_id,
           count(*) FILTER (WHERE c.state = 'completed') AS accepted,
           count(*) FILTER (WHERE c.state = 'claimed')   AS active
      FROM task_claims c GROUP BY c.task_id
  ) actual ON actual.task_id = t.id
 WHERE t.accepted_count <> COALESCE(actual.accepted, 0)
    OR t.active_claim_count <> COALESCE(actual.active, 0);
-- 3b. Contributor counters (§2.3b's result). Must be zero.
SELECT
  (SELECT count(*) FROM users u
     LEFT JOIN (SELECT claimed_by_user_id AS id, count(*) AS n FROM task_claims
                 WHERE state = 'completed' AND claimed_by_user_id IS NOT NULL
                 GROUP BY 1) c ON c.id = u.id
    WHERE u.tasks_completed <> COALESCE(c.n, 0))
+ (SELECT count(*) FROM anonymous_workers w
     LEFT JOIN (SELECT claimed_by_anon_uuid AS id, count(*) AS n FROM task_claims
                 WHERE state = 'completed' AND claimed_by_anon_uuid IS NOT NULL
                 GROUP BY 1) c ON c.id = w.uuid
    WHERE w.tasks_completed <> COALESCE(c.n, 0))
  AS contributor_disagreements;
```

4. **Functional smoke**: run one real task against the restored stack with
   MAGPIE, from a machine with the pinned data installed -- a `contribute.txt`
   with `server https://<host>` and `maxtasks 1`, then `magpie contribute`.
   This exercises dispatch, data verification, the artifact fetch and the
   result write, and the result it submits is a genuine one.

   **Never use `worker/fake_worker.py` for this.** It submits invented
   results, the server records them as real contributions to real jobs, and
   they skew SPRT verdicts and rating fits until someone finds and deletes
   them. It is test tooling for disposable stacks only.

In-flight claims need no action. Claims open at the restore point are reclaimed
by the heartbeat timeout, and a worker submitting against a claim the restored
database never issued is rejected the same way any stale claim is.

---

## 5. Region loss

A second copy of the stack, applied into the same account in another region.
Every bucket name is global and every IAM role name account-wide, and the lost
region's still hold theirs — as do the DR replicas the rebuild restores from —
so the copy is named apart with `name_suffix`, and kept in state of its own.

1. `terraform apply` the copy, in its own workspace so the lost region's state
   is left as it was, and with the service at **zero tasks**: the backend
   migrates its database before it binds, and a schema made in the empty new
   instance would stop the restore in step 3 at its first object:

   ```bash
   terraform -chdir=infra workspace select -or-create dr
   # The stack's own settings first, the DR overrides after (a later -var
   # wins): without prod.tfvars every other setting fell back to its default
   # -- a micro instance, 20 GiB, and the fleet's MAGPIE floor back at 0.1.1.
   # Storage sized for the restore in step 3 at creation: autoscaling cannot
   # keep up with a bulk load (a step at most every six hours), and raising
   # the variable later is no quicker. Sized from the replicated dump's
   # manifest: database_bytes is the database's own size when it was dumped
   # (the dump compresses four times or more, so a multiple of the dump
   # undershoots), plus 30% and room for WAL (db_max_wal_size_mb/1024; 4 by
   # default); never under RDS's minimum of 20. Raise it by hand to
   # production's own allocation if that is larger and known.
   # The replica bucket is in the lost stack's dr_region, which need not be
   # $DR_REGION; with no --region the CLI asks the lost region first.
   DR_REGION=<the region the copy is built in>
   THIRD_REGION=<a third region, up, for the copy's own replicas>
   REPLICA_REGION=<the lost stack's dr_region>
   # <account>: this account's id, the lost stack's (a drill: §6).
   REPLICA=s3://birdtest-backups-dr-<account>/pg
   MANIFEST=$(aws s3 ls --region $REPLICA_REGION "$REPLICA/" | grep manifest | tail -1 | awk '{print $4}')
   DR_STORAGE_GB=$(aws s3 cp --region $REPLICA_REGION "$REPLICA/$MANIFEST" - | python3 -c 'import json, math, sys; print(max(20, math.ceil(json.load(sys.stdin)["database_bytes"] * 1.3 / 2**30) + 4))')
   echo "restoring ${MANIFEST%.manifest.json}: $DR_STORAGE_GB GiB"
   # Everything below needs these; an unset one wrote region = "" (the CLI's
   # default region, likely the lost one) into dr.tfvars. One `if`, not a
   # `: "${X:?}"` line: pasted into a terminal, a failed expansion abandons
   # only its own line, and the lines after it ran anyway.
   # An existing one for this region -- filled in, then this block pasted
   # again -- is kept; one for another (a drill's, §6) is not reused.
   if [ -e infra/dr.tfvars ] && grep -q "^region *= *\"$DR_REGION\"" infra/dr.tfvars \
      && grep -q "^db_allocated_storage *= *$DR_STORAGE_GB\$" infra/dr.tfvars; then
     echo "infra/dr.tfvars exists for $DR_REGION; kept"
   elif [ -e infra/dr.tfvars ]; then
     echo "infra/dr.tfvars is for another region or size (a drill's?): move it aside first"
   elif [ -n "$DR_REGION" ] && [ -n "$THIRD_REGION" ] && [ -n "$REPLICA_REGION" ] \
      && [ -n "$MANIFEST" ] && [ -n "$DR_STORAGE_GB" ]; then
     # The DR overrides, in a file of their own beside prod.tfvars, so every
     # later DR command -- step 4's apply, the ones after it -- carries the same
     # ones (a later -var-file wins). Every ARN prod.tfvars names in the lost
     # region is overridden: a task whose secrets live there cannot start.
     # GITHUB_TOKEN is optional; leave it empty, or create the parameter in
     # $DR_REGION and put its ARN here. Fill in the <...> before applying.
     # printf, not a heredoc: copied from this indented list, a heredoc's
     # closing EOF keeps its indent and never ends it.
     printf '%s\n' \
       "region                     = \"$DR_REGION\"" \
       "dr_region                  = \"$THIRD_REGION\"" \
       'name_suffix                = "-dr"' \
       "db_allocated_storage       = $DR_STORAGE_GB" \
       'github_token_parameter_arn = ""' \
       "acm_certificate_arn        = \"<a certificate issued in $DR_REGION>\"" \
       "backend_image              = \"<pullable from $DR_REGION>\"" \
       "derived_builder_image      = \"<pullable from $DR_REGION>\"" \
       "frontend_image             = \"<pullable from $DR_REGION>\"" \
       > infra/dr.tfvars
   else echo "set DR_REGION, THIRD_REGION, REPLICA_REGION, MANIFEST and DR_STORAGE_GB first"; fi
   # Scheduled tasks off until step 4: the derived builder would fail rows
   # whose inputs are not synced yet, and a 03:00 backup would dump the
   # half-restored database as the newest.
   ```

   Fill in the `<...>` in `infra/dr.tfvars`, then apply. (`azs=null` undoes
   `prod.tfvars`' pinned zones, which are the lost region's: the copy takes
   `$DR_REGION`'s first two, and step 4 pins those in `dr.tfvars`.)

   ```bash
   if ! grep -q "^region *= *\"$DR_REGION\"" infra/dr.tfvars; then
     echo "infra/dr.tfvars is not for $DR_REGION"
   elif grep -n '<' infra/dr.tfvars; then echo "fill these in first"; else
     terraform -chdir=infra apply -var-file=prod.tfvars -var-file=dr.tfvars \
       -var desired_count=0 -var scheduled_tasks_enabled=false -var 'azs=null'
   fi
   ```

   ACM certificates are regional, so the lost region's cannot be used; request
   one in `$DR_REGION` first. The three images must be pullable from
   `$DR_REGION`: a registry in the lost region is lost with it, so push
   releases to one that is not (ECR with cross-region replication, or a
   registry outside AWS) — or rebuild them from this repository and the pinned
   MAGPIE commit (README.md, "Deploying") first. And this needs Terraform's
   state for the copy only — it is a new workspace — but step 9's return to the
   default workspace needs the original's, which README.md says to keep off
   the lost machine and region. `dr_region` must be a region that is up — the
   copy replicates into it as the original did — not the one that was lost.
   `$CLUSTER` below is then `birdtest-dr`.
2. Set the new instance's master password and the two SSM parameters by hand,
   as README.md "Deploying" does for a first deploy — but in `$DR_REGION`
   (`--region $DR_REGION` on every command; the CLI's default is likely the
   lost region) and with `DB_INSTANCE=birdtest-dr`. Terraform creates the
   instance with a placeholder password and manages the parameters' names,
   never their values. `SESSION_SIGNING_KEY` may be a fresh
   `openssl rand -hex 32`; every session cookie is invalidated, which costs a
   round of logins.
3. Restore the database from the replicated dump. The new stack's ops task
   can read only the new stack's own backups bucket, so first copy the dump
   step 1 sized for, and its manifest, across into it -- from the operator's
   machine, whose credentials can read the replica:

   ```bash
   # In step 1's shell, or with its variables set again: an empty REPLICA made
   # the source the local root, and --recursive would copy this machine's
   # files into the Object-Locked bucket, where they stay for 30 days. One
   # `if`, so nothing runs unless every one is set (a `: "${X:?}"` line stops
   # only itself when pasted into a terminal).
   NEW=s3://$(terraform -chdir=infra output -raw backups_bucket)/pg
   if [ -n "$REPLICA" ] && [ -n "$MANIFEST" ] && [ -n "$REPLICA_REGION" ] \
      && [ -n "$DR_REGION" ] && [ "$NEW" != "s3:///pg" ]; then
     STAMP=${MANIFEST%.manifest.json}
     aws s3 cp --recursive --source-region $REPLICA_REGION --region $DR_REGION \
       "$REPLICA/$STAMP/" "$NEW/$STAMP/"
     aws s3 cp --source-region $REPLICA_REGION --region $DR_REGION \
       "$REPLICA/$MANIFEST" "$NEW/$MANIFEST"
     echo "$STAMP"   # for the ops shell, which has none of these variables
   else echo "set REPLICA, MANIFEST, REPLICA_REGION and DR_REGION (step 1) first"; fi
   ```

   This needs `kms:Decrypt` on the lost stack's `backups-dr` key (in
   `$REPLICA_REGION`) and `kms:GenerateDataKey` and `kms:Decrypt` on the new
   stack's backups key; both keys leave that to IAM, so an administrator has
   it and a narrower role needs it granted. The copies take the new bucket's
   30-day Object Lock retention.

   Then, in `scripts/prod-shell.sh` against the new stack, fetch and check it
   with §2.1's first block with `STAMP=<the stamp printed above>` (and
   `BUCKET`, `S3_REGION` unset: it is the stack's own bucket now).
   Replication does not keep order, so just after a 03:00 backup the newest
   manifest can arrive before all of its dump's files, and with the source
   region gone the rest never will: if the block still says
   `CHECKSUM MISMATCH` after one fresh copy, take the previous stamp --
   `MANIFEST=$(aws s3 ls --region $REPLICA_REGION "$REPLICA/" | grep manifest | tail -2 | head -1 | awk '{print $4}')`,
   a day older, which step 1's size still covers -- and copy that across
   instead. Then the dump into the new instance, as in §2.1's second block but
   with `pg_restore -d "$DATABASE_URL"` — run detached as §2.1 does, since it
   takes longer than an ECS Exec session lasts, and not finished until its log
   ends `pg_restore exit 0`: step 4's `desired_count=1` against a
   half-restored database serves it.
4. The leave-generation KLVs (`leaves/`) and the imported input data
   (`inputs/`) are in `birdtest-artifacts-dr-<account>`; sync both prefixes
   into the new stack's artifact bucket (`birdtest-dr-artifacts-<account>`)
   with `aws s3 sync`. The service reads only its own bucket. (Input data
   imported before the `input-data` replication rule existed was never
   replicated: re-import those tarballs from the admin page instead —
   importing is idempotent.) Then pin the copy's zones and apply with the
   service up and the schedules on (their defaults):

   ```bash
   # Once, and on a line of its own: a second append redefines the attribute.
   # Captured first: a failed output wrote a bare `azs = `, which the grep
   # then took for done.
   AZS=$(terraform -chdir=infra output -json azs) && ! grep -q '^azs' infra/dr.tfvars \
     && printf '\nazs = %s\n' "$AZS" >> infra/dr.tfvars
   # Only with the zones pinned: without them prod.tfvars' -- the lost
   # region's -- apply, and the plan replaces the subnets.
   if grep -q '^azs' infra/dr.tfvars; then
     terraform -chdir=infra apply -var-file=prod.tfvars -var-file=dr.tfvars
   else echo "no azs in infra/dr.tfvars: is this the dr workspace, with step 1's state?"; fi
   ```

   Every later command against the copy takes both files, in that order.
5. Re-verify the SES domain identity and add the DKIM CNAMEs, and point the
   MAIL FROM MX record at the DR region (the `ses_mail_from_records` output of
   the `dr` workspace) — account mail is
   dead until this is done, which means no confirmations and no password
   resets. SES production access is per region: request it again.
6. Point DNS at the new ALB.
7. Confirm the new stack's alert subscription (SNS mails a confirmation) and
   run README.md's alert-path checks with `SUFFIX=-dr` and
   `REGION=$DR_REGION` (in the `dr` workspace, so the Terraform outputs are
   the copy's): with `REGION` still the lost region, they test nothing that
   exists.
8. Run §4, and **Check artifacts** on every leave-generation job (§3): the
   synced objects are the replicas' latest versions, which need not be the ones
   the restored rows describe.
9. `terraform -chdir=infra workspace select default` when done. The workspace
   is remembered in `infra/.terraform`, and `scripts/prod-sql.sh`,
   `scripts/prod-shell.sh` and §1's outputs would otherwise go on reading the
   DR stack's state (both scripts print the workspace they are using).

---

## 6. Testing this runbook

- `./scripts/restore-roundtrip.sh` — proves a dump of the current schema
  restores byte-identically into an empty database. Run it after any schema
  change; nightly CI runs it too.
- `./scripts/backup-drill-check.sh` — runs `backup.sh` and `restore-drill.sh`
  exactly as their Fargate tasks do (the `postgres:16` image, the script as
  `bash -c`) against the local stack's Postgres and MinIO: a backup taken while
  another session keeps writing, the drill of that backup, and a backup that
  cannot upload. Needs `docker compose up -d --wait postgres minio minio-init`
  and the schema applied; nightly CI runs it. It contacts no AWS endpoint:
  both scripts skip their CloudWatch metrics when `AWS_S3_ENDPOINT` names a
  stand-in object store, and `BACKUP_METRICS=false` (or `=true`) overrides that
  either way.
- `scripts/restore-drill.sh` runs monthly in production and restores the newest
  dump into a throwaway Postgres started inside its own task — never into the
  production instance, whose credentials it does not hold. A failure means the backups are not
  restorable and is the loudest alarm in the system.
- Twice a year, do §5 by hand into a **scratch account** (not only another
  region of this one). The manual drill exists to find the steps that live only
  in someone's head. In this account the drill's copy would hold §5's names --
  the `-dr` buckets are global, its IAM roles account-wide -- and a real region
  loss would fail at step 1 on the first of them.

  §5 reads production's replicas (`birdtest-backups-dr-<account>`,
  `birdtest-artifacts-dr-<account>`, with production's account id), which the
  scratch account cannot. Stage them first, through this machine, so neither
  account is granted anything on the other's buckets:

  ```bash
  # With PRODUCTION's credentials. PROD_REPLICA_REGION is production's
  # dr_region; PROD_ACCOUNT its account id.
  STAGE=$(mktemp -d)
  PROD_REPLICA=s3://birdtest-backups-dr-$PROD_ACCOUNT/pg
  M=$(aws s3 ls --region $PROD_REPLICA_REGION "$PROD_REPLICA/" | grep manifest | tail -1 | awk '{print $4}')
  if [ -n "$M" ] && [ -n "$PROD_ACCOUNT" ]; then
    aws s3 cp --recursive --region $PROD_REPLICA_REGION "$PROD_REPLICA/${M%.manifest.json}/" "$STAGE/pg/${M%.manifest.json}/"
    aws s3 cp --region $PROD_REPLICA_REGION "$PROD_REPLICA/$M" "$STAGE/pg/$M"
    for P in leaves inputs; do
      aws s3 sync --region $PROD_REPLICA_REGION "s3://birdtest-artifacts-dr-$PROD_ACCOUNT/$P" "$STAGE/$P"
    done
  else echo "set PROD_ACCOUNT and PROD_REPLICA_REGION first"; fi
  # Then with the SCRATCH account's credentials: one bucket stands for both
  # replicas. SCRATCH_ACCOUNT is its id.
  aws s3 mb --region $PROD_REPLICA_REGION s3://birdtest-drill-stage-$SCRATCH_ACCOUNT
  aws s3 sync --region $PROD_REPLICA_REGION "$STAGE" s3://birdtest-drill-stage-$SCRATCH_ACCOUNT/
  rm -rf "$STAGE"   # production's data: users' emails and password hashes
  ```

  Then run §5 in the scratch account with `REPLICA_REGION=$PROD_REPLICA_REGION`
  and `REPLICA=s3://birdtest-drill-stage-$SCRATCH_ACCOUNT/pg`. Step 4 syncs
  `leaves/` and `inputs/` from the same bucket. Step 3's `kms:Decrypt` on the
  replica's key does not apply: the staging bucket uses S3's own encryption.
  Skip step 6: DNS stays on production. The copy holds production's data, so
  tear it down the day the drill ends. Use the scratch account's credentials,
  the `dr` workspace, and the drill's `DR_REGION` and `THIRD_REGION`:

  ```bash
  # Nothing writes while the buckets empty: the service is stopped, and the
  # 03:00 backup and the derived builder are unscheduled.
  terraform -chdir=infra apply -var-file=prod.tfvars -var-file=dr.tfvars \
    -var desired_count=0 -var scheduled_tasks_enabled=false
  aws rds modify-db-instance --region $DR_REGION --db-instance-identifier birdtest-dr \
    --no-deletion-protection --apply-immediately
  # Destroy refuses a bucket that is not empty, and versioning keeps every
  # version, delete markers included. A listing page holds at most 1,000 --
  # all delete-objects takes -- so one page at a time (--no-paginate; the
  # CLI otherwise merges every page into one request that is refused), until
  # the listing is empty. The backups buckets' versions are GOVERNANCE-locked
  # for 30 days, so only they take --bypass-governance-retention. A version
  # that is not deleted stops the loop for that bucket.
  empty() {  # bucket region [--bypass-governance-retention]
    while aws s3api list-object-versions --bucket "$1" --region "$2" --no-paginate \
        --query '{Objects: [Versions[].{Key:Key,VersionId:VersionId}, DeleteMarkers[].{Key:Key,VersionId:VersionId}][]}' \
        --output json > /tmp/versions.json \
      && grep -q '"Key"' /tmp/versions.json; do
      aws s3api delete-objects --bucket "$1" --region "$2" $3 \
        --delete file:///tmp/versions.json --query Errors --output json > /tmp/errors.json
      if grep -q '"Key"' /tmp/errors.json; then echo "$1: not deleted:"; cat /tmp/errors.json; return 1; fi
    done
  }
  A=$(aws sts get-caller-identity --query Account --output text)
  empty birdtest-dr-backups-$A $DR_REGION --bypass-governance-retention
  empty birdtest-dr-artifacts-$A $DR_REGION
  empty birdtest-dr-backups-dr-$A $THIRD_REGION --bypass-governance-retention
  empty birdtest-dr-artifacts-dr-$A $THIRD_REGION
  terraform -chdir=infra destroy -var-file=prod.tfvars -var-file=dr.tfvars
  # Destroy leaves the instance's final snapshot behind, by design (rds.tf).
  aws rds delete-db-snapshot --region $DR_REGION --db-snapshot-identifier birdtest-dr-final
  aws s3 rb --force --region $PROD_REPLICA_REGION s3://birdtest-drill-stage-$A
  terraform -chdir=infra workspace select default
  terraform -chdir=infra workspace delete dr
  mv infra/dr.tfvars infra/dr.tfvars.drill-$(date +%Y%m%d)
  ```

  (A source bucket is emptied before its replica, so replication has nothing
  left to write into the replica. If `empty` stops on a bucket, fix what it
  printed and run it again. A real §5 must not start from the drill's
  `dr.tfvars`, so it is moved aside.)

---

## Rotating the database password

The master password is set by hand (`infra/rds.tf` sets a placeholder
`password`, so RDS does not manage it) and lives only inside
`/birdtest/DATABASE_URL`. Rotation is
this, in order; the service fails new connections between the first and last
step, so do it in a quiet moment:

```bash
DB_PASSWORD=$(openssl rand -hex 24)
aws rds modify-db-instance --region "$REGION" --db-instance-identifier birdtest \
  --master-user-password "$DB_PASSWORD" --apply-immediately
aws rds wait db-instance-available --region "$REGION" --db-instance-identifier birdtest

OLD_URL=$(aws ssm get-parameter --region "$REGION" --name /birdtest/DATABASE_URL \
  --with-decryption --query Parameter.Value --output text)
NEW_URL=$(python3 -c 'import sys, urllib.parse as u
p = u.urlsplit(sys.argv[1])
print(p._replace(netloc="birdtest:" + sys.argv[2] + "@" + p.netloc.rsplit("@", 1)[1]).geturl())' \
  "$OLD_URL" "$DB_PASSWORD")
aws ssm put-parameter --region "$REGION" --name /birdtest/DATABASE_URL --type SecureString --overwrite \
  --value "$NEW_URL"

# Tasks read SSM at start; the backup task reads it per run.
aws ecs update-service --region "$REGION" --cluster "$CLUSTER" --service birdtest --force-new-deployment
```
