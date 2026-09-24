# birdtest — Recovery Runbook

Copy-pasteable procedures for the scenarios in
[PLAN.md's Backups and Restore](PLAN.md#backups-and-restore). That section
explains why; this one is what to type at 2am. Read the whole procedure before starting any of it.

Placeholders throughout: `$REGION` (default `us-east-1`), `$CLUSTER`
(`birdtest`), `$BUCKET` (the `backups_bucket` Terraform output).

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
aws rds restore-db-instance-to-point-in-time --region "$REGION" \
  --source-db-instance-identifier birdtest \
  --target-db-instance-identifier "birdtest-restore-$STAMP" \
  --restore-time '2026-09-07T02:55:00Z' \
  --db-subnet-group-name birdtest-db \
  --vpc-security-group-ids "$(terraform -chdir=infra output -raw db_security_group_id 2>/dev/null || echo sg-XXXX)" \
  --no-publicly-accessible \
  --db-instance-class db.t4g.micro

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

Repoint the application. The master password is set by hand and lives only in
the `DATABASE_URL` parameter, so keep it and swap the host. A PITR copy keeps
the password the source had at the restore point; if it was rotated after that
point, set it on the new instance first (see "Rotating the database password"):

```bash
ENDPOINT=$(aws rds describe-db-instances --region "$REGION" \
  --db-instance-identifier "birdtest-restore-$STAMP" \
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

Then run §4 (verification). Only once it passes, rename or retire the damaged
instance — never before.

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
shape is always the same — restore a copy somewhere else, copy the missing rows
across, repair the counters, recompute the derived state.

### 2.1 Get a copy of the old data

Either a PITR instance from just before the mistake (fresher, §1 steps 2–3 with
a `-scratch-` identifier and no repointing), or the latest nightly dump:

```bash
STAMP=2026-09-07T03-00-00Z
aws s3 cp "s3://$BUCKET/pg/$STAMP/dump" /tmp/dump --recursive
createdb -h "$SCRATCH_HOST" -U birdtest birdtest_scratch
pg_restore -h "$SCRATCH_HOST" -U birdtest -d birdtest_scratch -j4 \
  --no-owner --no-privileges --exit-on-error /tmp/dump
```

For a dump small enough, the scratch database can be the local docker compose
stack: `./scripts/dev-restore.sh /tmp/dump` (which scrubs on the way in).

### 2.2 Copy the rows back, in dependency order

Dump only the job's rows from the scratch copy and load them into production.
`ON CONFLICT DO NOTHING` throughout, so a partial re-run is safe:

```bash
JOB=00000000-0000-0000-0000-000000000000

psql "$SCRATCH_URL" -v job="$JOB" -At <<'SQL' > /tmp/restore.sql
\set ON_ERROR_STOP on
-- Order matters: tasks, then claims, then everything hanging off a claim.
COPY (SELECT * FROM tasks WHERE job_id = :'job') TO STDOUT;
SQL
```

In practice this is a table-by-table `COPY ... TO` / `COPY ... FROM` for:

| Order | Table | Filter |
|---|---|---|
| 1 | `tasks` | `job_id = :job` |
| 2 | `opening_rack_requests` / `game_requests` / `leave_requests` | `task_id IN (...)` |
| 3 | `task_claims` | `task_id IN (...)` |
| 4 | `game_results`, `leave_records` | `job_id = :job` / `task_id IN (...)` |
| 5 | `position_analysis_records` → `_moves` → `_plies` | `job_id = :job`, then by parent id |
| 6 | `leave_rack_progress`, `leave_rack_staging`, `leave_generation_progress`, `leave_selection_cursors`, `leave_generation_artifacts`, `leave_generation_transitions` | `job_id = :job` |

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

`position_analysis_records.id` and `_moves.id` are `BIGSERIAL`. Restoring them
with their original ids preserves the parent-child links; afterwards the
sequences must be moved past what was inserted, or the next insert collides:

```sql
SELECT setval('position_analysis_records_id_seq', (SELECT max(id) FROM position_analysis_records));
SELECT setval('position_analysis_moves_id_seq',   (SELECT max(id) FROM position_analysis_moves));
SELECT setval('position_analysis_plies_id_seq',   (SELECT max(id) FROM position_analysis_plies));
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
 WHERE t.id = actual.id;

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
 WHERE j.id = t.job_id AND t.job_id = :'job';

-- The job's own counters. claims_issued is the scheduler's deficit numerator,
-- measured from claims_baseline; the statement after this one puts the
-- restored job level with the jobs beside it, as activation and purge do
-- (scheduler::join_at_parity), so it neither owes nor is owed a backlog. The
-- rest are the dashboard's progress totals; they
-- are maintained one task at a time in the claim and submit paths, so a row
-- copy leaves them describing the results the job had before. Each is
-- recomputed here exactly as the read it replaced computed it: one result per
-- task, because redundant claims replay the same work.
UPDATE jobs j
   SET claims_issued = (SELECT count(*) FROM task_claims c
                          JOIN tasks t ON t.id = c.task_id
                         WHERE t.job_id = j.id),
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
                          WHERE p.job_id = j.id)
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

### 2.3b Repair the contributor counters

The counters on `users` and `anonymous_workers` are not scoped to one job — they
span every job an identity ever worked on — so a partial restore of one job
cannot repair them in isolation the way §2.3 repairs the job's own. Recompute
them globally, once, after every job has been restored:

```sql
BEGIN;

UPDATE users u
   SET tasks_completed = COALESCE(actual.n, 0),
       last_completed_at = actual.last
  FROM (SELECT c.claimed_by_user_id AS id, count(*) AS n, max(c.completed_at) AS last
          FROM task_claims c
         WHERE c.state = 'completed' AND c.claimed_by_user_id IS NOT NULL
         GROUP BY 1) actual
 WHERE u.id = actual.id;

UPDATE anonymous_workers w
   SET tasks_completed = COALESCE(actual.n, 0),
       last_completed_at = actual.last
  FROM (SELECT c.claimed_by_anon_uuid AS uuid, count(*) AS n, max(c.completed_at) AS last
          FROM task_claims c
         WHERE c.state = 'completed' AND c.claimed_by_anon_uuid IS NOT NULL
         GROUP BY 1) actual
 WHERE w.uuid = actual.uuid;

COMMIT;
```

This is the one recount a restore is most likely to need, and the one most
likely to be forgotten: nothing about a single job's restore makes a wrong
leaderboard visible. An identity with no completed claims at all keeps whatever
it had — the joins above only touch identities that appear in `task_claims` — so
if claims were *dropped* rather than restored, zero those rows first
(`UPDATE users SET tasks_completed = 0, last_completed_at = NULL;` and the
same for `anonymous_workers`) and let the statements above put back what the
rows actually support.

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
- **Leave-generation artifacts**: if any object is missing, use
  `POST /api/admin/jobs/:id/rebuild-artifacts` (the "Check artifacts" button on
  the admin job page) rather than restoring bytes — see §3.
- **Derived data** (`derived_data`): the SHA-256 of each wordmap and rack info
  table the server built. Derived by definition, and **a job whose rows are
  missing does not dispatch** — which is the symptom a restore produces here:
  active jobs handing out nothing. Activating each job again re-queues them
  (`POST /api/admin/jobs/:id/activate`), and the builder task fills them in
  within a few minutes; `/admin/derived-data` shows the queue. The files
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
  (`?force=true`).
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

Never restore the artifact bucket wholesale to an older point: the bucket is
allowed to be newer than the database, never older (PLAN.md, "Artifacts: back up, or rebuild?").

---

## 4. Verifying a restore

Do not declare it finished because the page loads.

```bash
# 1. Row counts, against the manifest of the dump that was restored.
aws s3 cp "s3://$BUCKET/pg/$STAMP.manifest.json" - | python3 -m json.tool | head -40
```

```sql
-- 2. Referential sanity.
SELECT count(*) AS jobs_missing_data FROM jobs j
 WHERE NOT EXISTS (SELECT 1 FROM input_data d WHERE d.id = j.letterdist_id)
    OR NOT EXISTS (SELECT 1 FROM input_data d WHERE d.id = j.layout_id);

SELECT count(*) AS inputs_missing_content FROM input_data
 WHERE role IN ('letterdist','layout') AND content IS NULL;

-- 3. Counter sanity. Must be zero.
SELECT count(*) AS counter_disagreements
  FROM tasks t
  JOIN LATERAL (
    SELECT count(*) FILTER (WHERE c.state = 'completed') AS accepted,
           count(*) FILTER (WHERE c.state = 'claimed')   AS active
      FROM task_claims c WHERE c.task_id = t.id
  ) actual ON true
 WHERE t.accepted_count <> actual.accepted OR t.active_claim_count <> actual.active;
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

1. `terraform apply` in the DR region: `terraform apply -var region=$DR_REGION -var dr_region=$REGION`.
2. Set the two SSM parameters by hand — Terraform manages their names, never
   their values. `SESSION_SIGNING_KEY` may be a fresh `openssl rand -hex 32`;
   every session cookie is invalidated, which costs a round of logins.
3. Restore the database from the replicated dump in
   `birdtest-backups-dr-<account>` (§2.1's `pg_restore`, into the new instance).
4. Artifacts are already in `birdtest-artifacts-dr-<account>`; sync them into
   the new region's artifact bucket, or point `S3_BUCKET` at the replica.
5. Re-verify the SES domain identity and add the DKIM CNAMEs — account mail is
   dead until this is done, which means no confirmations and no password
   resets.
6. Point DNS at the new ALB.
7. Run §4.

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
  dump into a throwaway database. A failure means the backups are not
  restorable and is the loudest alarm in the system.
- Twice a year, do §5 by hand into a scratch account or region. The manual
  drill exists to find the steps that live only in someone's head.

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

# Tasks read SSM at start; the backup and restore-drill tasks read it per run.
aws ecs update-service --region "$REGION" --cluster "$CLUSTER" --service birdtest --force-new-deployment
```
