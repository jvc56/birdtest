-- Make a production dump safe to have on a laptop.
--
-- Apply immediately after restoring a production dump locally, before doing
-- anything else with it (PLAN.md, "Local development"). A dump carries real email
-- addresses, argon2 password hashes, API key hashes, unexpired reset tokens and
-- every anonymous worker's UUID, which is that worker's whole credential; none
-- of that is needed to reproduce a bug, and all of it is a disclosure risk
-- sitting in a dev database.
--
-- NEVER against production: it sets every password to a public one, deletes
-- every API key and replaces every anonymous contributor's identity. It
-- refuses unless asked for by name, which `scripts/dev-restore.sh` does:
--
--   psql -v dev_copy=1 -v ON_ERROR_STOP=1 -d <a local copy> -f scripts/scrub.sql
--
-- Its usage line was once `psql "$DATABASE_URL" -f scripts/scrub.sql`, which in
-- the ops shell is production (the audit's pass 23).
--
-- Everything here is idempotent, so running it twice is harmless — which
-- matters, because the failure mode to protect against is forgetting whether
-- it was run at all.

-- The transaction opens before the guard, so that a refusal aborts it: psql
-- run on a file stops at the error, but pasted into an interactive psql it
-- only returns to the prompt and goes on reading, and with the refusal before
-- `BEGIN` the pasted scrub ran, and committed, after it (the audit's pass 24).
-- Now every statement after a refusal fails in the aborted transaction --
-- unless a psqlrc sets ON_ERROR_ROLLBACK, which wraps each statement in a
-- savepoint so that the refusal undoes only itself: turned off here.
\set ON_ERROR_STOP on
\set ON_ERROR_ROLLBACK off
BEGIN;
\if :{?dev_copy}
\else
\set dev_copy false
\endif
\if :dev_copy
\else
\echo 'scrub.sql refused: it rewrites every password, key and identity. Run it through scripts/dev-restore.sh, or with -v dev_copy=1 on a copy you are sure of -- never production.'
DO $$ BEGIN RAISE EXCEPTION 'scrub.sql refused: -v dev_copy=1 not given'; END $$;
\endif

-- Emails become derivable-from-id placeholders in a domain that can never
-- resolve, so a stray mail send in a dev stack cannot reach a real person.
UPDATE users
   SET email = 'user-' || id || '@example.invalid';

-- One shared throwaway password for every account, so a local stack can be
-- logged into as anyone. This is the argon2id hash of the literal string
-- 'birdtest-local' under the same parameters the server hashes with. It is
-- deliberately public: knowing it grants nothing anywhere a real hash was
-- replaced.
UPDATE users
   SET password_hash = '$argon2id$v=19$m=19456,t=2,p=1$YmlyZHRlc3Rsb2NhbHNjcnVi$Z2iFpCJQu1sWVr9w5180HQv0XnsDF+NW5fIwhZOSB9U';

-- Credentials and single-use tokens have no development value at all.
TRUNCATE api_keys, email_confirmations, password_reset_tokens;

-- The backup history describes buckets this stack cannot read.
TRUNCATE backups;

-- An anonymous worker is authenticated by its UUID alone (X-Worker-UUID), so
-- each one is replaced by a fresh UUID, and everything that names it follows:
-- its claims, its ban and its audit rows. What it did is kept. The new rows go
-- in first and the old ones come out last, so every foreign key holds
-- throughout and no superuser is needed to switch them off. (Until the
-- thirty-first audit this file left them all, and a scrubbed dump could
-- submit as any anonymous contributor.)
CREATE TEMP TABLE scrub_anon_remap ON COMMIT DROP AS
    SELECT uuid AS old_uuid, gen_random_uuid() AS new_uuid FROM anonymous_workers;
INSERT INTO anonymous_workers (uuid, first_seen_at, last_seen_at, tasks_completed, last_completed_at)
    SELECT m.new_uuid, a.first_seen_at, a.last_seen_at, a.tasks_completed, a.last_completed_at
    FROM anonymous_workers a JOIN scrub_anon_remap m ON m.old_uuid = a.uuid;
UPDATE task_claims c SET claimed_by_anon_uuid = m.new_uuid
    FROM scrub_anon_remap m WHERE c.claimed_by_anon_uuid = m.old_uuid;
UPDATE worker_bans b SET anon_uuid = m.new_uuid
    FROM scrub_anon_remap m WHERE b.anon_uuid = m.old_uuid;
UPDATE audit_log l SET actor_anon_uuid = m.new_uuid
    FROM scrub_anon_remap m WHERE l.actor_anon_uuid = m.old_uuid;
UPDATE audit_log l SET target_id = m.new_uuid::text
    FROM scrub_anon_remap m WHERE l.target_type = 'worker' AND l.target_id = m.old_uuid::text;
DELETE FROM anonymous_workers a USING scrub_anon_remap m WHERE a.uuid = m.old_uuid;

-- A claim still open when the dump was taken is live in production for a few
-- minutes more, and its token is what a submission names.
UPDATE task_claims SET claim_token = gen_random_uuid() WHERE state = 'claimed';

-- Ban reasons are an admin's free text about a person -- names, addresses,
-- IPs -- kept in the ban and in its audit row (the audit's pass 23).
UPDATE worker_bans SET reason = '[scrubbed]' WHERE reason IS NOT NULL;
UPDATE audit_log SET reason = '[scrubbed]'
 WHERE action IN ('worker.banned', 'worker.unbanned') AND reason IS NOT NULL;

COMMIT;

\echo 'Scrubbed. Every account now has the password birdtest-local.'
