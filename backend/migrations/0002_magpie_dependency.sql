-- Server-built hashes for the files a worker derives locally, and the builder
-- that produced every KLV artifact.
--
-- See MAGPIE_DEPENDENCY.md. In short: a wordmap and a rack info table are built
-- on the contributor's own machine from files the job pins, and are far too
-- large to ship -- 179 MB and 1.9 GB for CSW24. Nothing checked them, so the
-- rack info table was refused outright and the wordmap was covered only by a
-- sidecar naming the .kwg it came from. A pinned MAGPIE on the server now
-- builds a reference copy of each, and the SHA-256 of that copy travels with
-- the claim; the worker builds its own and uses it only if the bytes agree.

-- The bytes of a lexicon or leaves file, in the object store, keyed by digest.
--
-- The import has always hashed every file in a tarball and kept the bytes only
-- for the two roles the server itself reads (letter distributions and
-- layouts). Building a reference wordmap means the server needs the .kwg a job
-- pins, and a reference rack info table means it needs the .klv2 as well, so
-- those two roles now go to the object store -- not into a column, because a
-- 6 MB lexicon and a 3.7 MB KLV per row is a different proposition from a
-- 489-byte distribution, and nothing queries their contents.
--
-- Nullable, and deliberately not backfilled: rows imported before this
-- migration have no stored bytes anywhere, and there is nothing to derive them
-- from short of re-downloading the tarball they came from. A derived build
-- whose inputs are such a row fails with that as its reason, which tells an
-- admin exactly what to do -- re-import the tarball, which is idempotent and
-- adds no rows for files whose bytes have not changed.
ALTER TABLE input_data ADD COLUMN object_key TEXT;

-- Carried from phase 1, like `content`, so confirmation writes the key without
-- re-downloading the tarball. The object itself is uploaded at staging time,
-- when the bytes are already in memory: an import the admin then cancels
-- leaves an object nobody references, which is keyed by digest and so is
-- exactly what the next import of the same file would have uploaded anyway.
ALTER TABLE input_data_import_rows ADD COLUMN object_key TEXT;

COMMENT ON COLUMN input_data.object_key IS
    'Object-store key for this file''s bytes (kwg and klv rows imported after '
    'the MAGPIE dependency landed). NULL means the bytes are not stored and a '
    'derived-file build from this row cannot run.';

-- One row per (role, inputs, builder): the file the server built and the hash
-- a worker has to reproduce.
--
-- The primary key is the whole identity of the file, not a surrogate, because
-- what makes two derived files the same file is that they were built from the
-- same inputs by the same builder. A wordmap depends on a .kwg and the letter
-- distribution it is built against; a rack info table depends on a .klv2 as
-- well, because its entries carry precomputed leave values.
--
-- `builder` is separate from the MAGPIE version on purpose. A CSW24 wordmap
-- built in December 2025 and one built nine months later differ in 72 million
-- bytes with the same inputs and the same wordmap format version: the builder
-- changed and the format did not have to. MAGPIE carries WMP_BUILDER_VERSION
-- and RIT_BUILDER_VERSION for exactly this, a test pins their output so a
-- change cannot pass without bumping them, and the server asks the binary it
-- runs (`magpie builders`) rather than being told in configuration.
CREATE TABLE derived_data (
    role          TEXT NOT NULL CHECK (role IN ('wmp','rit')),
    -- What the worker loads the file as. A wordmap's is its lexicon's name; a
    -- rack info table's is '<lexicon>.<leaves>', because a table belongs to a
    -- (.kwg, .klv2) pair and two jobs on CSW24 with different leaves must not
    -- share one.
    name          TEXT NOT NULL,
    builder       TEXT NOT NULL,          -- 'wmp-1', 'rit-1'
    kwg_id        UUID NOT NULL REFERENCES input_data(id),
    -- NULL for a wordmap, which is built from the lexicon alone. The partial
    -- unique index below is what makes (role, name, builder, kwg, NULL) a key
    -- rather than a duplicate waiting to happen: in a UNIQUE constraint two
    -- NULLs are distinct, so a plain UNIQUE would let a wordmap be queued
    -- twice.
    klv_id        UUID REFERENCES input_data(id),
    letterdist_id UUID NOT NULL REFERENCES input_data(id),
    state         TEXT NOT NULL DEFAULT 'pending'
                  CHECK (state IN ('pending','building','built','failed')),
    -- Set exactly when state = 'built'. The equivalence is a constraint rather
    -- than a convention because dispatch reads this hash: a row that says
    -- 'built' with no hash would be dispatched as if it had one.
    sha256        TEXT CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    bytes         BIGINT CHECK (bytes >= 0),
    -- The instruction-set target the building MAGPIE was compiled for, e.g.
    -- 'nehalem'. Recorded and reported, never compared: measurement says these
    -- builders' output does not depend on it, and a contributor who builds
    -- from source should not be locked out on the strength of a field. If that
    -- ever stops being true, this column is the evidence.
    build_target  TEXT,
    -- Why the last attempt failed, shown to the admin verbatim.
    error         TEXT,
    -- Taken by the builder task when it starts a row, so a second builder does
    -- not start the same three-minute build. A lease rather than a plain flag:
    -- a builder that dies leaves 'building' behind forever otherwise.
    leased_until  TIMESTAMPTZ,
    attempts      INT NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    requested_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    built_at      TIMESTAMPTZ,
    CONSTRAINT derived_data_built_has_hash CHECK (
        (state = 'built') = (sha256 IS NOT NULL AND bytes IS NOT NULL)
    ),
    -- A wordmap is built from the lexicon and the distribution; a rack info
    -- table additionally from the leaves. A wmp row carrying a klv_id would be
    -- claiming a dependency it does not have.
    CONSTRAINT derived_data_inputs_match_role CHECK (
        (role = 'rit') = (klv_id IS NOT NULL)
    )
);

-- The identity of a derived file, in the two shapes it comes in. Partial
-- indexes because a wordmap's klv_id is NULL and NULLs are distinct in a
-- UNIQUE constraint, which would silently permit duplicate wordmap rows.
CREATE UNIQUE INDEX derived_data_wmp_idx
    ON derived_data (name, builder, kwg_id, letterdist_id)
    WHERE role = 'wmp';
CREATE UNIQUE INDEX derived_data_rit_idx
    ON derived_data (name, builder, kwg_id, klv_id, letterdist_id)
    WHERE role = 'rit';

-- The builder task's queue: oldest request first, so a job that has been
-- waiting is not starved by one created since.
CREATE INDEX derived_data_queue_idx
    ON derived_data (requested_at)
    WHERE state IN ('pending','building');

-- Which builder wrote each generation's KLV.
--
-- Until now a rebuild that produced different bytes from the stored hash was
-- evidence of corruption, because there was only ever one implementation. With
-- MAGPIE building these, a MAGPIE upgrade can legitimately change them, and a
-- rebuild under a later builder has to report "built by a different builder"
-- rather than "differs" -- otherwise the first upgrade after a restore drill
-- reads as data loss.
--
-- Nullable: rows written before this migration were built by the server's own
-- Rust port, which no longer exists. NULL means exactly that, and is not the
-- same as any builder a rebuild could be running now.
ALTER TABLE leave_generation_artifacts ADD COLUMN builder TEXT;

COMMENT ON COLUMN leave_generation_artifacts.builder IS
    'The MAGPIE KLV builder that wrote these bytes (''klv-1''), or NULL for an '
    'artifact written by the server''s own Rust port before MAGPIE_DEPENDENCY.md.';
