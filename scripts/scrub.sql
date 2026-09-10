-- Make a production dump safe to have on a laptop.
--
-- Apply immediately after restoring a production dump locally, before doing
-- anything else with it (PLAN.md, "Local development"). A dump carries real email
-- addresses, argon2 password hashes, API key hashes and unexpired reset
-- tokens; none of that is needed to reproduce a bug, and all of it is a
-- disclosure risk sitting in a dev database.
--
--   psql "$DATABASE_URL" -f scripts/scrub.sql
--
-- Everything here is idempotent, so running it twice is harmless — which
-- matters, because the failure mode to protect against is forgetting whether
-- it was run at all.

BEGIN;

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

COMMIT;

\echo 'Scrubbed. Every account now has the password birdtest-local.'
