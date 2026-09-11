#!/usr/bin/env bash
#
# Prove that a dump of this schema restores into an empty database and comes
# back identical. Run it against the local docker compose stack:
#
#   docker compose up -d postgres backend
#   ./scripts/restore-roundtrip.sh
#
# This is what keeps scripts/backup.sh honest as the schema moves: the dump
# path, the manifest's row counts, and the verification queries the production
# restore drill runs (PLAN.md, "Verifying a restore") all execute here, against a
# database seeded with a row in every table a result touches.
#
# It runs entirely inside the Postgres container, so the host needs no
# Postgres client — Docker is the only dependency, as everywhere else.

set -Eeuo pipefail

COMPOSE="${COMPOSE:-docker compose}"
PGUSER_="${PGUSER_:-birdtest}"
SRC_DB="${SRC_DB:-birdtest}"
DEST_DB="${DEST_DB:-birdtest_roundtrip}"
DUMP_DIR="/tmp/roundtrip-dump"

psql_() {
  ${COMPOSE} exec -T postgres psql -U "${PGUSER_}" -v ON_ERROR_STOP=1 -q "$@"
}
psql_val() {
  ${COMPOSE} exec -T postgres psql -U "${PGUSER_}" -d "$1" -tA -v ON_ERROR_STOP=1 -c "$2"
}

cleanup() {
  local status=$?
  ${COMPOSE} exec -T postgres psql -U "${PGUSER_}" -d postgres -q \
    -c "DROP DATABASE IF EXISTS ${DEST_DB} WITH (FORCE)" >/dev/null 2>&1 || true
  ${COMPOSE} exec -T postgres rm -rf "${DUMP_DIR}" >/dev/null 2>&1 || true
  if (( status == 0 )); then echo "round trip passed"; else echo "round trip FAILED" >&2; fi
  exit "${status}"
}
trap cleanup EXIT

# The schema has to exist: the backend applies it on start.
if ! psql_val "${SRC_DB}" "SELECT to_regclass('public.tasks') IS NOT NULL" | grep -q '^t$'; then
  echo "no schema in ${SRC_DB} -- run: ${COMPOSE} up -d postgres backend" >&2
  exit 1
fi

# --- Seed ------------------------------------------------------------------
# A dump of an empty schema would round-trip trivially and prove very little.
# This walks the same dependency chain a real job does: input data a job pins,
# a job, a task, a claim, and a result hanging off that claim -- including the
# bytea and jsonb columns, which are where a dump format goes wrong if it is
# going to.
echo "seeding"
psql_ -d "${SRC_DB}" <<'SQL'
BEGIN;

INSERT INTO users (username, email, password_hash, is_admin)
VALUES ('roundtrip', 'roundtrip@example.invalid', 'x', true)
ON CONFLICT (username) DO NOTHING;

INSERT INTO input_data (path, role, name, sha256, bytes, tarball_date, content)
VALUES ('letterdistributions/roundtrip.csv', 'letterdist', 'roundtrip',
        repeat('a', 64), 3, '20260101', '\x00ff00'::bytea),
       ('layouts/roundtrip.txt', 'layout', 'roundtrip',
        repeat('b', 64), 3, '20260101', '\x010203'::bytea)
ON CONFLICT (path, sha256) DO NOTHING;

INSERT INTO jobs (job_type, status, priority, redundancy, variant,
                  letterdist_id, layout_id, created_by)
SELECT 'games', 'active', 1, 1, 'classic',
       (SELECT id FROM input_data WHERE name = 'roundtrip' AND role = 'letterdist'),
       (SELECT id FROM input_data WHERE name = 'roundtrip' AND role = 'layout'),
       (SELECT id FROM users WHERE username = 'roundtrip')
WHERE NOT EXISTS (SELECT 1 FROM jobs);

INSERT INTO tasks (job_id, seed, state, accepted_count)
SELECT id, 42, 'completed', 1 FROM jobs
WHERE NOT EXISTS (SELECT 1 FROM tasks);

INSERT INTO task_claims (task_id, claim_token, state, claimed_by_user_id, completed_at)
SELECT t.id, gen_random_uuid(), 'completed',
       (SELECT id FROM users WHERE username = 'roundtrip'), now()
FROM tasks t
WHERE NOT EXISTS (SELECT 1 FROM task_claims);

INSERT INTO game_results (task_claim_id, task_id, games, wins, losses, ties,
                          p1_score_mean, p1_score_sd, p2_score_mean, p2_score_sd)
SELECT c.id, c.task_id, 10, 6, 4, 0, 412.5, 55.25, 398.0, 61.5
FROM task_claims c
WHERE NOT EXISTS (SELECT 1 FROM game_results);

-- A backups row, so the jsonb column is exercised too.
INSERT INTO backups (kind, s3_key, started_at, finished_at, dump_bytes, row_counts, sha256, ok)
SELECT 'pg_dump', 'pg/roundtrip', now() - interval '4 minutes', now(), 1234,
       '{"users": 1}'::jsonb, repeat('c', 64), true
WHERE NOT EXISTS (SELECT 1 FROM backups);

COMMIT;
SQL

# --- Dump ------------------------------------------------------------------
echo "dumping"
${COMPOSE} exec -T postgres rm -rf "${DUMP_DIR}"
${COMPOSE} exec -T postgres pg_dump -U "${PGUSER_}" -d "${SRC_DB}" \
  --format=directory --jobs=4 --compress=6 --no-owner --no-privileges \
  --file="${DUMP_DIR}"
${COMPOSE} exec -T postgres pg_restore --list "${DUMP_DIR}" >/dev/null

COUNTS_SQL="
  SELECT COALESCE(jsonb_object_agg(relname, cnt), '{}'::jsonb)::text
  FROM (
    SELECT c.relname,
           (xpath('/row/c/text()',
                  query_to_xml(format('SELECT count(*) AS c FROM %I.%I', n.nspname, c.relname),
                               false, true, '')))[1]::text::bigint AS cnt
    FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE c.relkind = 'r' AND n.nspname = 'public'
  ) counts"
BEFORE="$(psql_val "${SRC_DB}" "${COUNTS_SQL}")"

# --- Restore ---------------------------------------------------------------
echo "restoring into ${DEST_DB}"
${COMPOSE} exec -T postgres psql -U "${PGUSER_}" -d postgres -q \
  -c "DROP DATABASE IF EXISTS ${DEST_DB} WITH (FORCE)"
${COMPOSE} exec -T postgres psql -U "${PGUSER_}" -d postgres -q \
  -c "CREATE DATABASE ${DEST_DB}"
${COMPOSE} exec -T postgres pg_restore -U "${PGUSER_}" -d "${DEST_DB}" \
  --jobs=4 --no-owner --no-privileges --exit-on-error "${DUMP_DIR}"

AFTER="$(psql_val "${DEST_DB}" "${COUNTS_SQL}")"

# --- Verify ----------------------------------------------------------------
if [[ "${BEFORE}" != "${AFTER}" ]]; then
  echo "ROW COUNTS DIFFER" >&2
  echo "  before: ${BEFORE}" >&2
  echo "  after:  ${AFTER}" >&2
  exit 1
fi
echo "row counts match across $(echo "${BEFORE}" | tr ',' '\n' | wc -l) tables"

# The referential and counter checks the production drill runs.
psql_ -d "${DEST_DB}" <<'SQL'
DO $$
DECLARE
    bad bigint;
BEGIN
    SELECT count(*) INTO bad FROM jobs j
     WHERE NOT EXISTS (SELECT 1 FROM input_data d WHERE d.id = j.letterdist_id)
        OR NOT EXISTS (SELECT 1 FROM input_data d WHERE d.id = j.layout_id);
    IF bad > 0 THEN RAISE EXCEPTION 'jobs with missing pinned input data: %', bad; END IF;

    SELECT count(*) INTO bad FROM input_data
     WHERE role IN ('letterdist', 'layout') AND content IS NULL;
    IF bad > 0 THEN RAISE EXCEPTION 'input_data rows missing content: %', bad; END IF;

    SELECT count(*) INTO bad
      FROM tasks t
      JOIN LATERAL (
        SELECT count(*) FILTER (WHERE c.state = 'completed') AS accepted,
               count(*) FILTER (WHERE c.state = 'claimed')   AS active
          FROM task_claims c WHERE c.task_id = t.id
      ) actual ON true
     WHERE t.accepted_count <> actual.accepted OR t.active_claim_count <> actual.active;
    IF bad > 0 THEN RAISE EXCEPTION 'tasks whose claim counters disagree with their claims: %', bad; END IF;
END
$$;
SQL
echo "referential and counter checks passed"

# Bytea and floating point survive the round trip byte for byte, which a row
# count would not notice.
BEFORE_CONTENT="$(psql_val "${SRC_DB}" "SELECT encode(content, 'hex') FROM input_data ORDER BY path")"
AFTER_CONTENT="$(psql_val "${DEST_DB}" "SELECT encode(content, 'hex') FROM input_data ORDER BY path")"
[[ "${BEFORE_CONTENT}" == "${AFTER_CONTENT}" ]] || { echo "input_data.content differs" >&2; exit 1; }

BEFORE_SCORES="$(psql_val "${SRC_DB}" "SELECT p1_score_mean, p1_score_sd FROM game_results ORDER BY task_id")"
AFTER_SCORES="$(psql_val "${DEST_DB}" "SELECT p1_score_mean, p1_score_sd FROM game_results ORDER BY task_id")"
[[ "${BEFORE_SCORES}" == "${AFTER_SCORES}" ]] || { echo "game_results scores differ" >&2; exit 1; }
echo "bytea and double precision columns match"
