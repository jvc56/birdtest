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
# operator's, and are set here instead: the DAMAGED_HOST assignment, and the
# damaged instance's URL (in production the two instances differ by host;
# here, by database name).
#
# The cases:
#   - the export refuses a damaged URL that reaches the restored instance, and
#     a DATABASE_URL that reaches the damaged one; the apply refuses before an
#     export has finished, after one that failed part-way, without a reviewed
#     /tmp/after-exclude, against another server than the export's restored
#     one, with an exclusion that matches nothing (rolled back, the reset
#     links still unspent) and with one that is not an id;
#   - legitimate actions since the restore point are applied -- a key revoked,
#     one suspended, one suspended and resumed, a password reset, an address
#     confirmed, bans added and lifted (a reason with a line of its own reading
#     `\.` included), a ban added and lifted again, an account deleted (an
#     admin), an action whose transaction began before the restore point --
#     and a bad migration's changes, which write no audit rows, are not, nor an
#     action from before the restore point that the export's hour takes in;
#   - an actor left out by id (upper case, with a comment) -- its key
#     revocations, suspensions and confirmations included -- and one action by
#     id (with a Windows line end); a deletion left out after its account's
#     own reset copies no password; an action hours after the restored
#     instance's newest row is taken;
#   - ids the two logs use for different rows are refused: the restored
#     instance writing after the repoint, a damaged log renumbered; a later reset left out keeps the restored password, and is listed;
#   - an admin demoted since is demoted again from the reviewed list, a line
#     deleted from it is kept deleted by a second export, none is proposed
#     when the damaged instance has no admins, and an admin the restored
#     instance lacks is listed;
#   - both blocks pasted again give the same state;
#   - every action the export asks for is one the backend writes.
#
# Sessions are not here: they end with the new signing key §1 sets when it
# repoints the application.
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
i = s.index("DAMAGED_HOST=''   # the damaged instance's endpoint address")
j = s.index("```", i)
k = s.index("\n", s.index("```bash", j + 3)) + 1
l = s.index("```", k)
print(s[i:j] + "\n\x1e\n" + s[k:l])
PY
)
BLOCK1=${blocks%%$'\x1e'*}
BLOCK2=${blocks#*$'\x1e'}
# The operator's lines, set by run() instead.
BLOCK1=$(sed -e "/^DAMAGED_HOST=''/d" \
  -e 's#^DAMAGED_URL=.*#DAMAGED_URL=$DAMAGED_URL_FOR_CHECK#' <<<"$BLOCK1")
grep -q 'DAMAGED_URL=$DAMAGED_URL_FOR_CHECK' <<<"$BLOCK1" || fail "the export's DAMAGED_URL line was not found"

url() { echo "postgresql:///$1?user=$PGUSER_"; }
# run <block> [VAR=value...]: one block in the container; prints its output,
# returns its status.
run() {
  local block=$1; shift
  ${EXEC} env DATABASE_URL="$(url "$RESTORED")" DAMAGED_URL_FOR_CHECK="$(url "$DAMAGED")" \
    DAMAGED_HOST=unused "$@" bash -s <<<"$block" 2>&1
}

for db in "$RESTORED" "$DAMAGED"; do
  psql_ -d postgres -c "DROP DATABASE IF EXISTS $db WITH (FORCE)" >/dev/null
done
${EXEC} sh -c 'rm -f /tmp/after-* /tmp/after.done'
psql_ -d postgres -c "CREATE DATABASE $RESTORED" >/dev/null
psql_ -d "$RESTORED" < "$HERE/../backend/migrations/0001_initial.sql" >/dev/null

# The restore point's state, with one audit row: a reset from just before it,
# which the export's hour takes in and must not apply again.
psql_ -d "$RESTORED" <<'SQL'
INSERT INTO users (id, username, email, password_hash, email_confirmed_at, is_admin) VALUES
 ('00000000-0000-0000-0000-000000000001', 'reset', 'reset@x', 'OLD-HASH', '2026-01-01', false),
 ('00000000-0000-0000-0000-000000000002', 'bannedlater', 'b2@x', 'h', now(), false),
 ('00000000-0000-0000-0000-000000000003', 'unbannedlater', 'b3@x', 'h', now(), false),
 ('00000000-0000-0000-0000-000000000004', 'deletedlater', 'd4@x', 'h', now(), true),
 ('00000000-0000-0000-0000-000000000005', 'admin', 'a@x', 'h', now(), true),
 ('00000000-0000-0000-0000-000000000007', 'confirmlater', 'c7@x', 'h', NULL, false),
 ('00000000-0000-0000-0000-0000000000a8', 'rogue', 'r8@x', 'h', now(), true),
 ('00000000-0000-0000-0000-000000000009', 'resetbefore', 'r9@x', 'H9', now(), false),
 ('00000000-0000-0000-0000-000000000010', 'flipflop', 'f10@x', 'h', now(), false),
 ('00000000-0000-0000-0000-000000000011', 'victim', 'v11@x', 'h', now(), false),
 ('00000000-0000-0000-0000-000000000012', 'hijacked', 'h12@x', 'H12-OLD', now(), false),
 ('00000000-0000-0000-0000-000000000013', 'rogueconfirmed', 'c13@x', 'h', NULL, false),
 ('00000000-0000-0000-0000-000000000014', 'victim2', 'v14@x', 'H14', now(), false);
INSERT INTO api_keys (id, user_id, key_hash, label) VALUES
 ('00000000-0000-0000-0000-00000000000a', '00000000-0000-0000-0000-000000000001', 'hash-a', 'revoked later'),
 ('00000000-0000-0000-0000-00000000000b', '00000000-0000-0000-0000-000000000001', 'hash-b', 'suspended later'),
 ('00000000-0000-0000-0000-00000000000c', '00000000-0000-0000-0000-000000000001', 'hash-c', 'untouched'),
 ('00000000-0000-0000-0000-00000000000d', '00000000-0000-0000-0000-000000000004', 'hash-d', 'deleted user key'),
 ('00000000-0000-0000-0000-00000000000e', '00000000-0000-0000-0000-000000000001', 'hash-e', 'flipped'),
 ('00000000-0000-0000-0000-0000000000ef', '00000000-0000-0000-0000-000000000001', 'hash-f', 'revoked in flight'),
 ('00000000-0000-0000-0000-0000000000a1', '00000000-0000-0000-0000-000000000001', 'hash-g', 'rogue revoked'),
 ('00000000-0000-0000-0000-0000000000a2', '00000000-0000-0000-0000-000000000001', 'hash-h', 'rogue suspended'),
 ('00000000-0000-0000-0000-0000000000a3', '00000000-0000-0000-0000-000000000001', 'hash-i', 'revoked hours after');
INSERT INTO anonymous_workers (uuid) VALUES ('00000000-0000-0000-0000-0000000000f1'), ('00000000-0000-0000-0000-0000000000f2');
INSERT INTO worker_bans (user_id, reason) VALUES ('00000000-0000-0000-0000-000000000003', 'old ban');
INSERT INTO worker_bans (anon_uuid, reason) VALUES ('00000000-0000-0000-0000-0000000000f2', 'old anon ban');
INSERT INTO password_reset_tokens (user_id, token_hash, expires_at)
VALUES ('00000000-0000-0000-0000-000000000001', 'tok-old', now() + interval '1 hour');
INSERT INTO email_confirmations (user_id, code_hash, expires_at)
VALUES ('00000000-0000-0000-0000-000000000004', 'code-4', now() + interval '1 day');
-- Three hours before the damage: the export's window hangs on the restored
-- instance's newest row, not on the damaged one's or on the clock.
INSERT INTO audit_log (action, actor_user_id, target_type, target_id, created_at) VALUES
 ('user.password_reset', '00000000-0000-0000-0000-000000000009', 'user', '00000000-0000-0000-0000-000000000009',
  now() - interval '3 hours');
SQL
sleep 1
psql_ -d postgres -c "CREATE DATABASE $DAMAGED TEMPLATE $RESTORED" >/dev/null

# What happened after the restore point: actions through the application,
# each with its audit row, then a bad migration and admin flags set by hand,
# with none.
psql_ -d "$DAMAGED" <<'SQL'
DELETE FROM api_keys WHERE id IN ('00000000-0000-0000-0000-00000000000a', '00000000-0000-0000-0000-0000000000ef');
UPDATE api_keys SET is_active = false WHERE id = '00000000-0000-0000-0000-00000000000b';
UPDATE users SET password_hash = 'NEW-HASH' WHERE id = '00000000-0000-0000-0000-000000000001';
UPDATE users SET password_hash = 'ATTACKER-12' WHERE id = '00000000-0000-0000-0000-000000000012';
UPDATE users SET email_confirmed_at = now() WHERE id = '00000000-0000-0000-0000-000000000007';
INSERT INTO worker_bans (user_id, reason) VALUES ('00000000-0000-0000-0000-000000000002', E'new ban\n\\.\nsecond line');
INSERT INTO worker_bans (anon_uuid, reason) VALUES ('00000000-0000-0000-0000-0000000000f1', 'anon ban');
DELETE FROM worker_bans WHERE user_id = '00000000-0000-0000-0000-000000000003';
INSERT INTO users (id, username, email, password_hash) VALUES ('00000000-0000-0000-0000-000000000006', 'newer', 'n@x', 'h');
INSERT INTO worker_bans (user_id, reason) VALUES ('00000000-0000-0000-0000-000000000006', 'newer ban');
DELETE FROM api_keys WHERE user_id IN ('00000000-0000-0000-0000-000000000004', '00000000-0000-0000-0000-000000000011');
UPDATE users SET deleted_at = now(), username = 'deleted-' || id, email = id || '@deleted.invalid', password_hash = '!', is_admin = false
 WHERE id IN ('00000000-0000-0000-0000-000000000004', '00000000-0000-0000-0000-000000000011');
INSERT INTO audit_log (action, actor_user_id, target_type, target_id, reason) VALUES
 ('api_key.revoked', '00000000-0000-0000-0000-000000000001', 'api_key', '00000000-0000-0000-0000-00000000000a', NULL),
 ('api_key.deactivated', '00000000-0000-0000-0000-000000000001', 'api_key', '00000000-0000-0000-0000-00000000000b', NULL),
 ('api_key.deactivated', '00000000-0000-0000-0000-000000000001', 'api_key', '00000000-0000-0000-0000-00000000000e', NULL),
 ('api_key.reactivated', '00000000-0000-0000-0000-000000000001', 'api_key', '00000000-0000-0000-0000-00000000000e', NULL),
 ('user.password_reset', '00000000-0000-0000-0000-000000000001', 'user', '00000000-0000-0000-0000-000000000001', NULL),
 ('user.password_reset', '00000000-0000-0000-0000-000000000012', 'user', '00000000-0000-0000-0000-000000000012', 'OWNER'),
 ('user.email_confirmed', '00000000-0000-0000-0000-000000000007', 'user', '00000000-0000-0000-0000-000000000007', NULL),
 ('worker.banned', '00000000-0000-0000-0000-000000000005', 'worker', '00000000-0000-0000-0000-000000000002', E'new ban\n\\.\nsecond line'),
 ('worker.banned', '00000000-0000-0000-0000-000000000005', 'worker', '00000000-0000-0000-0000-0000000000f1', 'anon ban'),
 ('worker.unbanned', '00000000-0000-0000-0000-000000000005', 'worker', '00000000-0000-0000-0000-000000000003', NULL),
 ('worker.banned', '00000000-0000-0000-0000-000000000005', 'worker', '00000000-0000-0000-0000-000000000006', 'newer ban'),
 ('worker.banned', '00000000-0000-0000-0000-000000000005', 'worker', '00000000-0000-0000-0000-000000000010', 'flip'),
 ('worker.unbanned', '00000000-0000-0000-0000-000000000005', 'worker', '00000000-0000-0000-0000-000000000010', NULL),
 ('user.password_reset', '00000000-0000-0000-0000-000000000004', 'user', '00000000-0000-0000-0000-000000000004', NULL),
 ('user.deleted', '00000000-0000-0000-0000-000000000005', 'user', '00000000-0000-0000-0000-000000000004', NULL),
 -- Damage through the application, to be left out: everything the rogue
 -- admin did, a deletion by the legitimate one, and a second reset.
 ('worker.unbanned', '00000000-0000-0000-0000-0000000000a8', 'worker', '00000000-0000-0000-0000-000000000002', NULL),
 ('user.deleted', '00000000-0000-0000-0000-000000000005', 'user', '00000000-0000-0000-0000-000000000011', 'VICTIM'),
 ('user.password_reset', '00000000-0000-0000-0000-000000000012', 'user', '00000000-0000-0000-0000-000000000012', 'ATTACKER'),
 ('worker.unbanned', '00000000-0000-0000-0000-000000000005', 'worker', '00000000-0000-0000-0000-0000000000f2', NULL),
 ('user.password_reset', '00000000-0000-0000-0000-000000000014', 'user', '00000000-0000-0000-0000-000000000014', NULL),
 -- More of the rogue's: a key revoked, one suspended, an address confirmed,
 -- and a deletion after its owner's own reset.
 ('api_key.revoked', '00000000-0000-0000-0000-0000000000a8', 'api_key', '00000000-0000-0000-0000-0000000000a1', NULL),
 ('api_key.deactivated', '00000000-0000-0000-0000-0000000000a8', 'api_key', '00000000-0000-0000-0000-0000000000a2', NULL),
 ('user.email_confirmed', '00000000-0000-0000-0000-0000000000a8', 'user', '00000000-0000-0000-0000-000000000013', NULL),
 ('user.deleted', '00000000-0000-0000-0000-0000000000a8', 'user', '00000000-0000-0000-0000-000000000014', NULL);
DELETE FROM worker_bans WHERE anon_uuid = '00000000-0000-0000-0000-0000000000f2';
DELETE FROM api_keys WHERE id IN ('00000000-0000-0000-0000-0000000000a1', '00000000-0000-0000-0000-0000000000a3');
UPDATE users SET email_confirmed_at = now() WHERE id = '00000000-0000-0000-0000-000000000013';
UPDATE users SET deleted_at = now(), username = 'deleted-' || id, password_hash = '!'
 WHERE id = '00000000-0000-0000-0000-000000000014';
-- A confirmation of an address confirmed long before, which must not move
-- its date; and the deletion dated as it happened, not as it is re-applied.
INSERT INTO audit_log (action, actor_user_id, target_type, target_id) VALUES
 ('user.email_confirmed', '00000000-0000-0000-0000-000000000001', 'user', '00000000-0000-0000-0000-000000000001');
UPDATE audit_log SET created_at = now() - interval '30 minutes'
 WHERE action = 'user.deleted' AND target_id = '00000000-0000-0000-0000-000000000004';
-- An action an hour after the restored instance's newest row and hours before
-- the damaged one's.
INSERT INTO audit_log (action, actor_user_id, target_type, target_id, created_at) VALUES
 ('api_key.revoked', '00000000-0000-0000-0000-000000000001', 'api_key', '00000000-0000-0000-0000-0000000000a3',
  now() - interval '2 hours');
-- A revocation whose transaction began before the restore point's last row
-- and committed after it.
INSERT INTO audit_log (action, actor_user_id, target_type, target_id, created_at)
SELECT 'api_key.revoked', '00000000-0000-0000-0000-000000000001', 'api_key', '00000000-0000-0000-0000-0000000000ef',
       min(created_at) - interval '5 minutes' FROM audit_log;
-- A bad migration, and admin flags set by hand: no audit rows.
UPDATE api_keys SET is_active = false;
UPDATE users SET password_hash = 'MIGRATED' WHERE id = '00000000-0000-0000-0000-000000000009';
UPDATE users SET is_admin = false WHERE username = 'rogue';
UPDATE users SET is_admin = true WHERE username = 'confirmlater';
SQL
audit_id() { val "$DAMAGED" "SELECT id FROM audit_log WHERE reason = '$1'"; }

state() {
  val "$RESTORED" "
    SELECT concat_ws(' | ',
      (SELECT string_agg(label || '=' || is_active, ',' ORDER BY label) FROM api_keys),
      (SELECT string_agg(username || ':' || password_hash, ',' ORDER BY username) FROM users
        WHERE username IN ('reset', 'resetbefore', 'hijacked', 'victim2')),
      (SELECT (email_confirmed_at IS NOT NULL)::text FROM users WHERE id = '00000000-0000-0000-0000-000000000007'),
      (SELECT (email_confirmed_at IS NOT NULL)::text FROM users WHERE id = '00000000-0000-0000-0000-000000000013'),
      (SELECT email_confirmed_at::date::text FROM users WHERE id = '00000000-0000-0000-0000-000000000001'),
      (SELECT username || '/' || (deleted_at < now() - interval '10 minutes') || '/' || (email_confirmed_at IS NULL)
              || '/' || (email LIKE '%@deleted.invalid') || '/' || password_hash || '/' || is_admin
              || '/' || (SELECT count(*) FROM email_confirmations WHERE user_id = users.id)
         FROM users WHERE id = '00000000-0000-0000-0000-000000000004'),
      (SELECT username || '/' || (deleted_at IS NULL) FROM users WHERE id = '00000000-0000-0000-0000-000000000011'),
      (SELECT string_agg(username, ',' ORDER BY username) FROM users WHERE is_admin),
      (SELECT count(*) FROM password_reset_tokens WHERE used_at IS NULL),
      (SELECT string_agg(coalesce(b.user_id::text, b.anon_uuid::text) || ':' || replace(b.reason, E'\n', '/')
                         || ':' || coalesce(u.username, '-'), ',' ORDER BY b.reason)
         FROM worker_bans b LEFT JOIN users u ON u.id = b.banned_by))"
}
WANT="flipped=true,rogue revoked=true,rogue suspended=true,suspended later=false,untouched=true | hijacked:H12-OLD,reset:NEW-HASH,resetbefore:H9,victim2:H14 | true | false | 2026-01-01 | deleted-00000000-0000-0000-0000-000000000004/true/true/true/!/false/0 | victim/true | admin,rogue | 0 | 00000000-0000-0000-0000-0000000000f1:anon ban:admin,00000000-0000-0000-0000-000000000002:new ban/\./second line:admin"
BEFORE=$(state)
unspent() { val "$1" "SELECT count(*) FROM password_reset_tokens WHERE used_at IS NULL"; }
exclude() { ${EXEC} sh -c "cat > /tmp/after-exclude"; }

echo "-- refusals"
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
echo | exclude
out=$(run "$BLOCK2") && fail "the apply ran after a failed export"
grep -q "has not finished" <<<"$out" || fail "no word on the failed export: $out"
[ "$(state)" = "$BEFORE" ] || fail "the apply after a failed export changed the restored instance"

echo "-- the apply refuses an unreviewed export, the wrong server and a bad exclusion"
${EXEC} rm -f /tmp/after-exclude
# Under a client's own date style and zone, which moved the window when the
# export read times in them.
out=$(run "$BLOCK1" PGDATESTYLE=Postgres PGTZ=Asia/Kolkata) || fail "the export failed: $out"
grep -q "rogue" <<<"$out" || fail "the summary does not name the rogue admin: $out"
out=$(run "$BLOCK2") && fail "the apply ran with no /tmp/after-exclude"
grep -q "no /tmp/after-exclude" <<<"$out" || fail "no word on the missing review: $out"
echo | exclude
out=$(run "$BLOCK2" DATABASE_URL="$(url "$DAMAGED")") && fail "the apply ran against the damaged instance"
grep -q "another server" <<<"$out" || fail "no word on the server: $out"
[ "$(unspent "$DAMAGED")" = 1 ] || fail "the refused apply wrote to the damaged instance"
echo 00000000-0000-0000-0000-0000000000ff | exclude
out=$(run "$BLOCK2") && fail "the apply ran with an exclusion that matches nothing"
grep -q "match no action" <<<"$out" || fail "no word on the unmatched exclusion: $out"
echo rogue | exclude
out=$(run "$BLOCK2") && fail "the apply ran with an exclusion that is not an id"
grep -q "do not start with" <<<"$out" || fail "no word on the name: $out"
[ "$(state)" = "$BEFORE" ] || fail "a refused apply changed the restored instance"

echo "-- review, apply"
exclude <<EOF
# the rogue admin, by account, as the summary shows it; one deletion and one reset, by id
00000000-0000-0000-0000-0000000000A8   # rogue$(printf '\r')
$(audit_id VICTIM) # deleted by mistake
$(audit_id ATTACKER)$(printf '\r')

EOF
# The operator keeps the rogue an admin (to deal with by hand), and leaves the
# deleted admin's demotion to the deletion itself.
${EXEC} sed -i -e '/rogue/d' -e '/deletedlater/d' /tmp/after-demote.csv
out=$(run "$BLOCK2" PGDATESTYLE=Postgres PGTZ=Asia/Kolkata) || fail "the apply failed: $out"
copied=$(sed -n '/^passwords taken from the damaged instance/,/^accounts /p' <<<"$out")
grep -qx ' *reset *' <<<"$copied" || fail "the copied hashes were not listed: $out"
grep -q "victim2\|hijacked" <<<"$copied" && fail "a hash not taken was listed as taken: $copied"
kept=$(sed -n '/^accounts reset since/,/^demoted/p' <<<"$out")
grep -q "victim2" <<<"$kept" || fail "the account whose password the damaged instance no longer holds was not listed: $out"
grep -q "00000000-0000-0000-0000-000000000006" <<<"$out" || fail "the newer identity's ban was not listed: $out"
grep -q "hijacked" <<<"$kept" || fail "the account whose last reset was left out was not listed: $out"
grep -q "deletedlater" <<<"$kept" && fail "an account deleted again was listed for a reset: $kept"
grep -q "confirmlater" <<<"$out" || fail "the damaged instance's extra admin was not listed: $out"
grep -q "| rogue " <<<"$out" || fail "the rogue admin was not listed as left out: $out"
got=$(state)
[ "$got" = "$WANT" ] || fail "after the apply: $got, want $WANT"

echo "-- both pasted again: the review is kept"
deleted_email=$(val "$RESTORED" "SELECT email FROM users WHERE id = '00000000-0000-0000-0000-000000000004'")
out=$(run "$BLOCK1") || fail "the export failed on a second paste: $out"
grep -q "kept as edited" <<<"$out" || fail "no word on the kept demotions: $out"
grep -q "^> .*rogue" <<<"$out" || fail "the proposal the edit set aside was not shown: $out"
run "$BLOCK2" >/dev/null || fail "the apply failed on a second paste"
[ "$(val "$RESTORED" "SELECT email FROM users WHERE id = '00000000-0000-0000-0000-000000000004'")" = "$deleted_email" ] \
  || fail "an account already deleted again was deleted anew"
got=$(state)
[ "$got" = "$WANT" ] || fail "after a second paste: $got"

echo "-- a damaged instance with no admins proposes no demotion"
psql_ -d "$DAMAGED" -c "UPDATE users SET is_admin = false" >/dev/null
psql_ -d "$RESTORED" -c "UPDATE users SET is_admin = true WHERE username = 'rogue'" >/dev/null
${EXEC} rm -f /tmp/after-demote.csv
out=$(run "$BLOCK1") || fail "the export failed: $out"
grep -q "no admins at all" <<<"$out" || fail "no word on a damaged instance with no admins: $out"
run "$BLOCK2" >/dev/null || fail "the apply failed"
[ "$(val "$RESTORED" "SELECT string_agg(username, ',' ORDER BY username) FROM users WHERE is_admin")" = "admin,rogue" ] \
  || fail "admins were demoted from a damaged instance with none"

echo "-- once the restored instance writes, its ids are no longer the damaged one's: no second paste"
psql_ -d "$RESTORED" -c "INSERT INTO audit_log (action, target_type, target_id) VALUES ('user.password_reset', 'user', '00000000-0000-0000-0000-000000000001')" >/dev/null
out=$(run "$BLOCK2") && fail "the apply ran after the restored instance had written"
grep -q "written audit rows since the export" <<<"$out" || fail "no word on the restored instance's new rows: $out"
out=$(run "$BLOCK1") && fail "the export ran with ids the two logs use for different rows"
grep -q "renumbered, or written here" <<<"$out" || fail "no word on the reused ids: $out"
psql_ -d "$RESTORED" -c "DELETE FROM audit_log WHERE id = (SELECT max(id) FROM audit_log)" >/dev/null

echo "-- a damaged log renumbered by a migration is refused, not read as nothing new"
psql_ -d "$DAMAGED" <<'SQL' >/dev/null
DELETE FROM audit_log WHERE id = (SELECT min(id) FROM audit_log);
UPDATE audit_log SET id = -id;
UPDATE audit_log a SET id = r.n FROM (SELECT id, row_number() OVER (ORDER BY id DESC) AS n FROM audit_log) r
 WHERE a.id = r.id;
SQL
out=$(run "$BLOCK1") && fail "the export ran over a renumbered log"
grep -q "renumbered, or written here" <<<"$out" || fail "no word on the renumbered log: $out"

echo "-- every action exported is one the backend writes"
for action in $(grep -o "'[a-z_]*\.[a-z_]*'" <<<"$BLOCK1" | tr -d "'" | sort -u); do
  grep -rqF "\"$action\"" "$HERE/../backend/src" || grep -rqF "'$action'" "$HERE/../backend/src" \
    || fail "the export asks for $action, which the backend never writes"
done
