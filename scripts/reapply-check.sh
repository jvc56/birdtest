#!/usr/bin/env bash
#
# Prove RUNBOOK.md §1's "re-apply what the restore undid for security" step
# against a real Postgres: its two blocks, taken from RUNBOOK.md as written,
# run against a "restored" database and a "damaged" one that went on after
# the restore point, through their failure and re-run cases. Run it against
# the local docker compose stack:
#
#   docker compose up -d postgres
#   ./scripts/reapply-check.sh
#
# or any Postgres 16 container, with PG_EXEC="docker exec -i <container>". The
# blocks run inside the container, as they run in the ops shell, in two
# databases of the check's own, which it drops afterwards. Two lines are the
# operator's, and are set here instead: the RESTORE_TIME and DAMAGED_HOST
# assignments, and the damaged instance's URL (in production the two
# instances differ by host; here, by database name).
#
# The cases:
#   - the export refuses a restore time not in UTC, a damaged URL that reaches
#     the restored instance, and a DATABASE_URL that reaches the damaged one;
#     the apply refuses before an export has finished, and after one that
#     failed part-way;
#   - legitimate actions since the restore point are applied -- a key revoked,
#     one suspended, a password reset, an address confirmed, bans added and
#     lifted (a reason with a line of its own reading `\.` included), an
#     account deleted -- and a bad migration's changes, which write no audit
#     rows, are not;
#   - an account deleted since is deleted again (name, address, hash, codes);
#     every session is ended;
#   - an admin demoted since is demoted again, from a reviewed list, and none
#     is proposed when the damaged instance has no admins; an action excluded
#     by its actor is not applied;
#   - both blocks pasted again give the same state;
#   - every action the export asks for is one the backend writes.
set -Eeuo pipefail

EXEC="${PG_EXEC:-docker compose exec -T postgres}"
PGUSER_="${PGUSER_:-birdtest}"
RESTORED=reapply_restored
DAMAGED=reapply_damaged
HERE="$(cd "$(dirname "$0")" && pwd)"

psql_() { ${EXEC} psql -U "${PGUSER_}" -X -q -v ON_ERROR_STOP=1 "$@"; }
val() { ${EXEC} psql -U "${PGUSER_}" -X -tA -v ON_ERROR_STOP=1 -d "$1" -c "$2"; }
fail() { echo "FAIL: $*" >&2; exit 1; }

cleanup() {
  local status=$?
  for db in "$RESTORED" "$DAMAGED"; do
    psql_ -d postgres -c "DROP DATABASE IF EXISTS $db WITH (FORCE)" >/dev/null 2>&1 || true
  done
  ${EXEC} sh -c 'rm -f /tmp/after-* /tmp/after.done' >/dev/null 2>&1 || true
  if (( status == 0 )); then echo "reapply check passed"; else echo "reapply check FAILED" >&2; fi
  exit "$status"
}
trap cleanup EXIT

# The step's two blocks, from RUNBOOK.md.
blocks=$(python3 - "$HERE/../RUNBOOK.md" <<'PY'
import sys
s = open(sys.argv[1]).read()
i = s.index("RESTORE_TIME=''   # the restore point, as above, in UTC")
j = s.index("```", i)
k = s.index("\n", s.index("```bash", j + 3)) + 1
l = s.index("```", k)
print(s[i:j] + "\n\x1e\n" + s[k:l])
PY
)
BLOCK1=${blocks%%$'\x1e'*}
BLOCK2=${blocks#*$'\x1e'}
# The operator's lines, set by run() instead.
BLOCK1=$(sed -e "/^RESTORE_TIME=''/d" -e "/^DAMAGED_HOST=''/d" \
  -e 's#^DAMAGED_URL=.*#DAMAGED_URL=$DAMAGED_URL_FOR_CHECK#' <<<"$BLOCK1")
grep -q 'DAMAGED_URL=$DAMAGED_URL_FOR_CHECK' <<<"$BLOCK1" || fail "the export's DAMAGED_URL line was not found"

url() { echo "postgresql:///$1?user=$PGUSER_"; }
# run <block> [VAR=value...]: one block in the container; prints its output,
# returns its status.
run() {
  local block=$1; shift
  ${EXEC} env DATABASE_URL="$(url "$RESTORED")" DAMAGED_URL_FOR_CHECK="$(url "$DAMAGED")" \
    RESTORE_TIME="$RESTORE_TIME" DAMAGED_HOST=unused "$@" bash -s <<<"$block" 2>&1
}

for db in "$RESTORED" "$DAMAGED"; do
  psql_ -d postgres -c "DROP DATABASE IF EXISTS $db WITH (FORCE)" >/dev/null
done
${EXEC} sh -c 'rm -f /tmp/after-* /tmp/after.done'
psql_ -d postgres -c "CREATE DATABASE $RESTORED" >/dev/null
psql_ -d "$RESTORED" < "$HERE/../backend/migrations/0001_initial.sql" >/dev/null

# The restore point's state.
psql_ -d "$RESTORED" <<'SQL'
INSERT INTO users (id, username, email, password_hash, email_confirmed_at, is_admin) VALUES
 ('00000000-0000-0000-0000-000000000001', 'reset', 'reset@x', 'OLD-HASH', now(), false),
 ('00000000-0000-0000-0000-000000000002', 'bannedlater', 'b2@x', 'h', now(), false),
 ('00000000-0000-0000-0000-000000000003', 'unbannedlater', 'b3@x', 'h', now(), false),
 ('00000000-0000-0000-0000-000000000004', 'deletedlater', 'd4@x', 'h', now(), false),
 ('00000000-0000-0000-0000-000000000005', 'admin', 'a@x', 'h', now(), true),
 ('00000000-0000-0000-0000-000000000007', 'confirmlater', 'c7@x', 'h', NULL, false),
 ('00000000-0000-0000-0000-000000000008', 'rogue', 'r8@x', 'h', now(), true);
INSERT INTO api_keys (id, user_id, key_hash, label) VALUES
 ('00000000-0000-0000-0000-00000000000a', '00000000-0000-0000-0000-000000000001', 'hash-a', 'revoked later'),
 ('00000000-0000-0000-0000-00000000000b', '00000000-0000-0000-0000-000000000001', 'hash-b', 'suspended later'),
 ('00000000-0000-0000-0000-00000000000c', '00000000-0000-0000-0000-000000000001', 'hash-c', 'untouched'),
 ('00000000-0000-0000-0000-00000000000d', '00000000-0000-0000-0000-000000000004', 'hash-d', 'deleted user key');
INSERT INTO anonymous_workers (uuid) VALUES ('00000000-0000-0000-0000-0000000000f1');
INSERT INTO worker_bans (user_id, reason) VALUES ('00000000-0000-0000-0000-000000000003', 'old ban');
INSERT INTO password_reset_tokens (user_id, token_hash, expires_at)
VALUES ('00000000-0000-0000-0000-000000000001', 'tok-old', now() + interval '1 hour');
INSERT INTO email_confirmations (user_id, code_hash, expires_at)
VALUES ('00000000-0000-0000-0000-000000000004', 'code-4', now() + interval '1 day');
SQL
RESTORE_TIME=$(val "$RESTORED" "SELECT to_char(now() AT TIME ZONE 'UTC' + interval '1 second', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')")
sleep 2
psql_ -d postgres -c "CREATE DATABASE $DAMAGED TEMPLATE $RESTORED" >/dev/null

# What happened after the restore point: actions through the application,
# each with its audit row, then a bad migration and a demotion, with none.
psql_ -d "$DAMAGED" <<'SQL'
DELETE FROM api_keys WHERE id = '00000000-0000-0000-0000-00000000000a';
UPDATE api_keys SET is_active = false WHERE id = '00000000-0000-0000-0000-00000000000b';
UPDATE users SET password_hash = 'NEW-HASH' WHERE id = '00000000-0000-0000-0000-000000000001';
UPDATE users SET email_confirmed_at = now() WHERE id = '00000000-0000-0000-0000-000000000007';
INSERT INTO worker_bans (user_id, reason) VALUES ('00000000-0000-0000-0000-000000000002', E'new ban\n\\.\nsecond line');
INSERT INTO worker_bans (anon_uuid, reason) VALUES ('00000000-0000-0000-0000-0000000000f1', 'anon ban');
DELETE FROM worker_bans WHERE user_id = '00000000-0000-0000-0000-000000000003';
INSERT INTO users (id, username, email, password_hash) VALUES ('00000000-0000-0000-0000-000000000006', 'newer', 'n@x', 'h');
INSERT INTO worker_bans (user_id, reason) VALUES ('00000000-0000-0000-0000-000000000006', 'newer ban');
DELETE FROM api_keys WHERE user_id = '00000000-0000-0000-0000-000000000004';
UPDATE users SET deleted_at = now(), username = 'deleted-4', email = 'x@deleted.invalid', password_hash = '!'
 WHERE id = '00000000-0000-0000-0000-000000000004';
INSERT INTO audit_log (action, actor_user_id, target_type, target_id, reason) VALUES
 ('api_key.revoked', '00000000-0000-0000-0000-000000000001', 'api_key', '00000000-0000-0000-0000-00000000000a', NULL),
 ('api_key.deactivated', '00000000-0000-0000-0000-000000000001', 'api_key', '00000000-0000-0000-0000-00000000000b', NULL),
 ('user.password_reset', '00000000-0000-0000-0000-000000000001', 'user', '00000000-0000-0000-0000-000000000001', NULL),
 ('user.email_confirmed', '00000000-0000-0000-0000-000000000007', 'user', '00000000-0000-0000-0000-000000000007', NULL),
 ('worker.banned', '00000000-0000-0000-0000-000000000005', 'worker', '00000000-0000-0000-0000-000000000002', E'new ban\n\\.\nsecond line'),
 ('worker.banned', '00000000-0000-0000-0000-000000000005', 'worker', '00000000-0000-0000-0000-0000000000f1', 'anon ban'),
 ('worker.unbanned', '00000000-0000-0000-0000-000000000005', 'worker', '00000000-0000-0000-0000-000000000003', NULL),
 ('worker.banned', '00000000-0000-0000-0000-000000000005', 'worker', '00000000-0000-0000-0000-000000000006', 'newer ban'),
 ('user.deleted', '00000000-0000-0000-0000-000000000005', 'user', '00000000-0000-0000-0000-000000000004', NULL),
 -- The damage through the application, by the rogue admin: to be excluded.
 ('worker.unbanned', '00000000-0000-0000-0000-000000000008', 'worker', '00000000-0000-0000-0000-000000000002', NULL);
-- A bad migration, and a demotion by hand: no audit rows.
UPDATE api_keys SET is_active = false;
UPDATE users SET is_admin = false WHERE username = 'rogue';
SQL

state() {
  val "$RESTORED" "
    SELECT concat_ws(' | ',
      (SELECT string_agg(label || '=' || is_active, ',' ORDER BY label) FROM api_keys),
      (SELECT password_hash FROM users WHERE id = '00000000-0000-0000-0000-000000000001'),
      (SELECT (email_confirmed_at IS NOT NULL)::text FROM users WHERE id = '00000000-0000-0000-0000-000000000007'),
      (SELECT username || '/' || (deleted_at IS NOT NULL) || '/' || (email_confirmed_at IS NULL)
              || '/' || (email LIKE '%@deleted.invalid') || '/' || password_hash || '/' || is_admin
              || '/' || (SELECT count(*) FROM email_confirmations WHERE user_id = users.id)
         FROM users WHERE id = '00000000-0000-0000-0000-000000000004'),
      (SELECT string_agg(DISTINCT session_generation::text, ',') FROM users),
      (SELECT string_agg(username, ',' ORDER BY username) FROM users WHERE is_admin),
      (SELECT count(*) FROM password_reset_tokens WHERE used_at IS NULL),
      (SELECT string_agg(coalesce(user_id::text, anon_uuid::text) || ':' || replace(reason, E'\n', '/'), ',' ORDER BY reason)
         FROM worker_bans))"
}
WANT="suspended later=false,untouched=true | NEW-HASH | true | deleted-00000000-0000-0000-0000-000000000004/true/true/true/!/false/0 | 1 | admin | 0 | 00000000-0000-0000-0000-0000000000f1:anon ban,00000000-0000-0000-0000-000000000002:new ban/\./second line"
BEFORE=$(state)

echo "-- refusals"
out=$(run "$BLOCK1" RESTORE_TIME="${RESTORE_TIME%Z}") && fail "the export ran with a restore time not in UTC"
grep -q "must be UTC" <<<"$out" || fail "no word on the restore time: $out"
out=$(run "$BLOCK1" DAMAGED_URL_FOR_CHECK="$(url "$RESTORED")") && fail "the export ran with the damaged URL at the restored instance"
grep -q "same server" <<<"$out" || fail "no word on the damaged URL: $out"
out=$(run "$BLOCK1" DATABASE_URL="$(url "$DAMAGED")" DAMAGED_URL_FOR_CHECK="$(url "$RESTORED")") \
  && fail "the export ran with DATABASE_URL at the damaged instance"
grep -q "not the restored one" <<<"$out" || fail "no word on DATABASE_URL: $out"
out=$(run "$BLOCK2") && fail "the apply ran before any export"
grep -q "has not finished" <<<"$out" || fail "no word on the missing export: $out"
[ "$(state)" = "$BEFORE" ] || fail "a refused block changed the restored instance"

echo "-- an export that fails part-way leaves nothing to apply"
run "$BLOCK1" >/dev/null || fail "the export failed"
out=$(run "$BLOCK1" DAMAGED_URL_FOR_CHECK="postgresql:///reapply_gone?user=$PGUSER_") && fail "an export from a missing instance succeeded"
out=$(run "$BLOCK2") && fail "the apply ran after a failed export"
grep -q "has not finished" <<<"$out" || fail "no word on the failed export: $out"
[ "$(state)" = "$BEFORE" ] || fail "the apply after a failed export changed the restored instance"

echo "-- export, exclude the rogue admin, apply"
out=$(run "$BLOCK1") || fail "the export failed: $out"
grep -q "rogue" <<<"$out" || fail "the summary does not name the rogue admin: $out"
${EXEC} sh -c 'echo 00000000-0000-0000-0000-000000000008 >> /tmp/after-exclude'
out=$(run "$BLOCK2") || fail "the apply failed: $out"
grep -q "00000000-0000-0000-0000-000000000006" <<<"$out" || fail "the newer identity's ban was not listed: $out"
got=$(state)
[ "$got" = "$WANT" ] || fail "after the apply: $got, want $WANT"

echo "-- both pasted again"
run "$BLOCK1" >/dev/null || fail "the export failed on a second paste"
run "$BLOCK2" >/dev/null || fail "the apply failed on a second paste"
got=$(state)
[ "$got" = "${WANT/| 1 |/| 2 |}" ] || fail "after a second paste: $got"

echo "-- a damaged instance with no admins proposes no demotion"
psql_ -d "$DAMAGED" -c "UPDATE users SET is_admin = false" >/dev/null
psql_ -d "$RESTORED" -c "UPDATE users SET is_admin = true WHERE username = 'rogue'" >/dev/null
out=$(run "$BLOCK1") || fail "the export failed: $out"
grep -q "no admins at all" <<<"$out" || fail "no word on a damaged instance with no admins: $out"
run "$BLOCK2" >/dev/null || fail "the apply failed"
[ "$(val "$RESTORED" "SELECT string_agg(username, ',' ORDER BY username) FROM users WHERE is_admin")" = "admin,rogue" ] \
  || fail "admins were demoted from a damaged instance with none"

echo "-- every action exported is one the backend writes"
for action in $(grep -o "'[a-z_]*\.[a-z_]*'" <<<"$BLOCK1" | tr -d "'" | sort -u); do
  grep -rqF "\"$action\"" "$HERE/../backend/src" || grep -rqF "'$action'" "$HERE/../backend/src" \
    || fail "the export asks for $action, which the backend never writes"
done
