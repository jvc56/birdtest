#!/usr/bin/env bash
#
# Time the two expensive steps of a leave-generation transition against the
# database DATABASE_URL points at (PLAN.md, "What these reads cost").
#
# Two things scale with the rack universe -- 3,199,724 rows for English:
#
#   1. materializing the next generation's universe, which is pure SQL and is
#      what this script times, two ways (see below);
#   2. streaming every rack out to derive leave values and build the KLV
#      (Rust and the network; only the read side is timed here).
#
# Step 1 took 56-66 seconds on the local compose Postgres and about a second on
# a small dev database, which is a wide enough spread that the production
# instance class cannot be guessed from either. This script answers it with the
# real thing, and it is safe to run against production: everything happens in a
# transaction that is rolled back, and it writes into a throwaway job id that no
# job row references.
#
# Step 1 is timed BOTH WAYS, which is the point of running it. The server can
# materialize a generation by copying the previous generation's rows in SQL, or
# by generating the racks from the pinned letter distribution and COPYing them
# in. The first keeps the rows server-side; the second sends them over the wire
# but shares one code path with job creation and derives from the source of
# truth rather than from prior rows. Which is faster is a property of the
# database and the link to it, so it is measured rather than argued about --
# and 1b here is a floor for the generate path: the real one also unranks each
# rack and ships the rows over the wire, neither of which psql can measure.
#
# Usage: DATABASE_URL=postgres://... scripts/leave-gen-bench.sh [racks]
#
# `racks` defaults to the English full-rack universe. A smaller number is
# proportionally faster and proves nothing about the real one.

set -Eeuo pipefail

: "${DATABASE_URL:?DATABASE_URL is required}"
RACKS="${1:-3199724}"

psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -v racks="$RACKS" <<'SQL'
\timing on
BEGIN;

-- leave_rack_progress references jobs, so the synthetic generation needs a
-- synthetic job to hang off. It is created inactive (the default), so even the
-- moment it exists inside this transaction it is not a job any worker can be
-- given, and the rollback removes it along with its rows.
\echo '== creating a throwaway job =='
INSERT INTO jobs (id, job_type, variant, letterdist_id, layout_id)
SELECT '00000000-0000-0000-0000-0000000000ff', 'leave_generation', 'classic',
       (SELECT id FROM input_data WHERE role = 'letterdist' ORDER BY imported_at LIMIT 1),
       (SELECT id FROM input_data WHERE role = 'layout' ORDER BY imported_at LIMIT 1);

\echo '== seeding a synthetic generation =='
INSERT INTO leave_rack_progress (job_id, generation, rack, occurrence_count, equity_sum)
SELECT '00000000-0000-0000-0000-0000000000ff', 1,
       -- Seven characters, like a full rack, and distinct per row.
       'R' || lpad(g::text, 7, '0'), 200, 2000.0
FROM generate_series(1, :racks) g;

-- So the plans below are chosen from row counts like a real generation's. Like
-- everything else here, the statistics it writes are rolled back with the
-- transaction.
ANALYZE leave_rack_progress;

\echo '== step 1a: copying the universe in SQL (server-side, reads 3.2M rows) =='
INSERT INTO leave_rack_progress (job_id, generation, rack)
SELECT job_id, generation + 1, rack FROM leave_rack_progress
WHERE job_id = '00000000-0000-0000-0000-0000000000ff' AND generation = 1
ON CONFLICT (job_id, generation, rack) DO NOTHING;

-- The same rows again, generated rather than read. Together with 1a this
-- separates the two halves of the copy's cost: 1b is the write and the index
-- maintenance alone, 1a is that plus reading 3.2 million existing rows. The
-- difference between them is what the generate path actually saves.
\echo '== step 1b: writing a generated universe (no read of existing rows) =='
INSERT INTO leave_rack_progress (job_id, generation, rack)
SELECT '00000000-0000-0000-0000-0000000000ff', 3, 'R' || lpad(g::text, 7, '0')
FROM generate_series(1, :racks) g
ON CONFLICT (job_id, generation, rack) DO NOTHING;

-- EXPLAIN ANALYZE rather than a plain SELECT: it executes the query the
-- transition streams, in the order it streams it, without shipping three
-- million rows to psql -- and a subquery around an aggregate would have let the
-- planner drop the sort that is half the point.
\echo '== step 2 (read side only): streaming a generation in rack order =='
EXPLAIN (ANALYZE, TIMING OFF)
SELECT rack, occurrence_count, equity_sum
FROM leave_rack_progress
WHERE job_id = '00000000-0000-0000-0000-0000000000ff' AND generation = 1
ORDER BY rack;

ROLLBACK;
SQL

cat <<'MSG'

Both numbers are the transition's floor, not its duration: the KLV derivation
(about 13 seconds of CPU for English in a release build) and the object-store
upload are on top, and a transition holds no lock while it runs -- workers on
other jobs are unaffected either way.

Step 1 no longer runs inside a generation transition: the universe is seeded
when a generation opens, from the claim path, so it is off the critical path
either way. What 1a against 1b decides is which implementation the server should
use for it -- the SQL copy from the previous generation, or generating the racks
and COPYing them in, which shares one code path with job creation and derives
from the pinned letter distribution rather than from prior rows. Take whichever
wins here; they produce identical rows.
MSG
