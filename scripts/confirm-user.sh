#!/usr/bin/env bash
# Confirm an account's address by hand, and optionally make it an admin: for
# while SES is in the sandbox and the confirmation mail cannot arrive, and
# after every pre-launch database reset (scripts/reset-prod-db.sh).
#
#   scripts/confirm-user.sh [--admin] [--print-sql] USERNAME
#
# It does what the site's confirm-email route does (backend/src/routes/auth.rs,
# confirm_email), in one transaction through scripts/prod-sql.sh: the
# account's row locked, its unused confirmation codes marked used, its
# email_confirmed_at set, and an audit row written exactly as the route writes
# it -- action user.email_confirmed, the account as actor and target -- with
# a reason saying it was done by hand. RUNBOOK §1's re-apply reads that row
# like any other. An account already confirmed is left as it is. --admin
# also sets is_admin (sign out and in again to see the admin pages).
#
# It refuses unless exactly one account that is not deleted has the username,
# compared as the site compares it (case-insensitively). The username is the
# user's own text, so it travels as hex and is decoded by Postgres: no
# character of it can end a quote. Nothing personal is selected: what psql
# prints is kept in CloudWatch for thirty days. --print-sql prints the SQL
# instead of running it.
set -euo pipefail

usage() { sed -n '6p' "$0" >&2; exit 2; }

admin=false print=0 username=""
while (($#)); do
  case $1 in
    --admin) admin=true ;;
    --print-sql) print=1 ;;
    --) shift; [[ $# == 1 ]] || usage; username=$1; break ;;
    -*) usage ;;
    *) [[ -z "$username" ]] || usage; username=$1 ;;
  esac
  shift
done
[[ -n "$username" ]] || usage
# The site's limit is 32 characters; a UTF-8 character is at most 4 bytes.
((${#username} <= 128)) || { echo "$0: not a username: too long" >&2; exit 2; }

hex=$(printf '%s' "$username" | od -An -v -tx1 | tr -d ' \n')
[[ "$hex" =~ ^([0-9a-f]{2})+$ ]] || { echo "$0: could not encode the username" >&2; exit 1; }

sql="
SET client_min_messages = notice;
DO \$confirm\$
DECLARE
  wanted text := convert_from(decode('$hex', 'hex'), 'UTF8');
  make_admin boolean := $admin;
  found int;
  uid uuid;
  confirmed timestamptz;
BEGIN
  SELECT count(*) INTO found FROM users
   WHERE lower(username) = lower(wanted) AND deleted_at IS NULL;
  IF found <> 1 THEN
    RAISE EXCEPTION 'expected one account with that username, found %', found;
  END IF;
  -- The account first, then its codes: the order the route and an admin's
  -- delete take them in.
  SELECT id, email_confirmed_at INTO uid, confirmed FROM users
   WHERE lower(username) = lower(wanted) AND deleted_at IS NULL
   FOR NO KEY UPDATE;
  IF confirmed IS NULL THEN
    UPDATE email_confirmations SET used_at = now() WHERE user_id = uid AND used_at IS NULL;
    UPDATE users SET email_confirmed_at = now() WHERE id = uid;
    INSERT INTO audit_log (action, actor_user_id, target_type, target_id, reason)
    VALUES ('user.email_confirmed', uid, 'user', uid::text,
            'confirmed by hand by the operator while SES was in the sandbox');
    RAISE NOTICE 'address confirmed';
  ELSE
    RAISE NOTICE 'address already confirmed at %; left as it is', confirmed;
  END IF;
  IF make_admin THEN
    UPDATE users SET is_admin = true WHERE id = uid;
  END IF;
END
\$confirm\$;
SELECT username, email_confirmed_at IS NOT NULL AS confirmed, is_admin
  FROM users
 WHERE lower(username) = lower(convert_from(decode('$hex', 'hex'), 'UTF8')) AND deleted_at IS NULL;
"

if ((print)); then
  printf '%s\n' "$sql"
  exit 0
fi
exec "$(dirname "$0")/prod-sql.sh" "$sql"
