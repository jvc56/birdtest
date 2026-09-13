# birdtest audit — findings

**Dates:** 2026-09-11 (first pass, sections A–I); 2026-09-13 (second pass,
section J; third pass, section K).
**Branches:** birdtest `audit/birdtest-2026-09-11` (off `main` at `baa4094`) for
the first two passes, then `audit/birdtest-2026-09-13` off that for the third —
see K.0 for why it is not off bare `main`. MAGPIE `birdtest-contribute`,
committed directly on that branch as instructed (the second pass needed no
MAGPIE changes; the third made one, K.5).

This is the record of every decision the audit made, complete enough to
second-guess each one without reading the diffs. Where this file and PLAN.md
disagree about what the code does, this file describes the audited code.

## How to read the decisions

Each discrepancy between the code and PLAN.md got exactly one of three
decisions:

| Decision | Meaning | Count |
|---|---|---|
| **PLAN.md updated** ("code wins") | The code's behaviour was right, or at least deliberate, and PLAN.md was a stale or inaccurate summary of it. | **24** (21 in A.1, J3, K-D1, K-D2) |
| **Code updated** ("plan wins") | The code was wrong — a bug, or a clear mismatch with what the rest of the system needs — and PLAN.md described the intended behaviour. PLAN.md was also touched where its wording needed to follow the fix. | **18** (16 in A.2, J1, J2) |
| **Unresolved at first** | Reasonable arguments on both sides, or a real design decision, left for a human. **All five are now decided and implemented** (A.3, section F). The second pass found no new ones; the third raised seven; K-D5 and K-D11 are decided (and K-D11 resolves K-D6 and K-D10), none of them built yet, plus I3 carried forward. | **8 decided, 3 open** |

Section B lists fixes that were not discrepancies (PLAN.md and code agreed and
were both wrong, or PLAN.md was silent). Section F lists every question the
audit left open, the options offered, the option chosen, and what was
implemented. Section H lists what implementing those decisions turned up, and
section I the follow-ups worth deciding next. Section J is the second pass:
the status of section I (I1 and I2 implemented, I3 still open), the
discrepancies it found, and its verification. **Section K is the third pass**,
which found and fixed two more races in leave generation, a scheduler contention
bug, and a missing submission check. **K.6 is the current list of open
questions** — K-D5 to K-D11, each with options and a recommendation. K-D5 and
K-D11 are decided but not yet built, and K-D11 resolves K-D6 and K-D10 by
deleting the fields they were about. It
supersedes I3, whose suggested fix does not work (K-D9). The Verification
section just below describes the first pass; J.6 and then K.7 supersede it for
current numbers.

## Verification

After implementing the section F decisions:

- **birdtest backend:**
  - `cargo clippy --all-targets -- -D warnings` passes.
  - `cargo test` passes: 82 unit and contract tests in the library (3 ignored: two round-trips through a real MAGPIE and the English timing test), and 21 integration tests (`backend/tests/`: admin 4, auth 3, leave generation 4, worker 10) against a real Postgres.
  - Each new test names the bug or decision it pins.
- **Frontend:** `npm run check` passes (0 errors, 0 warnings).
- **Terraform:** `terraform fmt -check -recursive` and `terraform validate` pass (Terraform 1.9.8, `init -backend=false`). Validation found two real errors, now fixed (H2, and the RDS password conflict in F2).
- **MAGPIE** (`birdtest-contribute`):
  - `make magpie magpie_test` (dev build: `-Werror`, ASan, UBSan) and `make magpie BUILD=release` succeed.
  - `magpie_test contribute`, `autoplay`, `sim`, `rl` and `rlfr` pass.
- **End to end:** `scripts/e2e_magpie.py` against the freshly reset local stack (real input-data import, real release `magpie contribute`, NWL23, data-20251004). Every job type ran and its results were stored as the server reads them back:
  - **games** and **game_pairs**: 2 tasks each; games counted, and a pentanomial for pairs.
  - **opening_rack**, static player: a best move for every rack. Simming player: win%, blended utility and per-ply rows stored (F6).
  - **leave_generation**: 2 tasks folded into generation 1's full-rack rows (F1); nothing written into MAGPIE's data directory (F9). The first run found H1.
  - **A real generation transition**, forced by marking every rack at target: generation 1 closed on the stack in about 15 seconds (streaming 3.2 million rows, deriving leave values, building and uploading the KLV, copying the universe). Rerun from scratch it produced the same SHA-256. MAGPIE then played a generation-2 task with the server-derived KLV, and the result was accepted.
  - The backend logged no errors.
- **Not run locally:** the Docker frontend image build in CI's form (the compose build of both images succeeded), and the two new GitHub Actions workflows themselves, which run on their first push.
- **Left behind on purpose:** the integration harness keeps one template database (`birdtest_tpl_<hash>`) on the Postgres it runs against.

---

## A. Discrepancies between code and PLAN.md

### A.1 PLAN.md updated (code wins)

| # | What the code does | What PLAN.md said | Reasoning |
|---|---|---|---|
| A1 | Activation refuses a tier whose active allocations would exceed 100%; less is allowed. | "active jobs within a tier must have allocations summing to 100%". | An admin activating the first job in a tier at 50% has to be allowed; an exact-100% rule is unsatisfiable while rebalancing. PLAN.md's own Admin API section already said "at most 100%". |
| A2 | Every job type generates tasks on demand. A task returns to `available` when capacity reopens. | Task list of "analyze a single position / play a game"; a "Pre-populated jobs" strategy; on-demand tasks "never pass through `available`". | Stale since opening racks became range-addressed (PLAN.md itself says so later, in Creation Strategies). |
| A3 | The claim body is required (`magpie_version`, `unsupported_jobs`). | "Task Claim … the body is empty." | Contradicted by PLAN.md's own contract section; the body is load-bearing. |
| A4 | A worker record is never upserted on request; anonymous identities are minted by claims. | Request Handling step 1 "upserts the worker record". | Stale. |
| A5 | Request tables carry `variant`, `letter_distribution` and `board_layout`, and no lexicon. Tasks are inserted `available` and then claimed. Generation 1's `previous_artifact_key` is never NULL. | Low-level SQL steps inserted `lexicon` columns, inserted tasks as `claimed`, and said gen-1's artifact key is NULL. | Stale; PLAN.md's schema and contract sections already matched the code. |
| A6 | Leave-generation KLVs are built in Rust (`jobs/klv.rs`). | "Three details of MAGPIE's CLI shape this call" (the old `magpie convert csv2klv` subprocess). | Removed from the code before the audit; the paragraph was a leftover. |
| A7 | `JobStats` has no `ratings` field. | `ratings: [...]` "always present". | Ratings are pool-scoped by design (PLAN.md's own Ratings section). The code comment on `workers` also wrongly said "empty for every job type but game pairs"; fixed. |
| A8 | A rating pool names its anchor explicitly (`anchor_player_config_id`). | The anchor is "identified as the config whose `max_iterations` is NULL". | The explicit anchor is more general and is what the schema, API and ratings page use. |
| A9 | Worker limits are 1/s with **burst 5**. | Security section: 1/s, no burst, "(TBD)". | PLAN.md's API Conventions section already documented the burst; the Security table now points there. |
| A10 | `infra/ecs.tf` sets neither `DATA_PATH` nor `MAGPIE_BIN`. | "Not yet done: infra/ecs.tf still sets DATA_PATH and MAGPIE_BIN". | Already done; the note was stale. |
| A11 | An opening-rack result is `{"racks":[{"rack","moves"}]}`. | Contract example `{"moves":[...]}`. | The fixture test already pinned the real shape; a client written from PLAN.md would have been rejected. |
| A12 | There was no CI. | "The CI end-to-end test guards the two constants." | There was nothing to guard them. PLAN.md now says the MAGPIE end-to-end CI test is not built; basic CI was added (section E). |
| A13 | A player simulates iff `num_plies > 0` (MAGPIE autoplay reads `sim_args->num_plies`). | "A player with `max_iterations` null is static." | MAGPIE's rule is what workers execute. birdtest now refuses configs with simulation settings but no plies, so the two definitions can no longer disagree. |
| A14 | The shipped migration's `game_results` comment said SPRT reads the divergent subset. | PLAN.md's reproduced schema had the correct pentanomial comment; the migration was the stale copy. | The comment in the migration was fixed. PLAN.md's schema block was then re-synced verbatim from the migration, which also picks up every schema change in this audit. |
| A15 | Post-submission stats go out only when the job has an SSE subscriber. | "pushes an event after every accepted result". | Observationally identical to a subscriber; PLAN.md now says the payload is built only when someone is listening. |
| A16 | Redundant claims' aggregates are stored separately; nothing compares them. | Position Capture: "agreement between workers is still checked". | PLAN.md elsewhere says no agreement check exists and reconciliation is deferred. The sentence now says agreement *can* be checked later. |
| A17 | Leave-gen rack selection did not exclude racks already out with a worker. | Silent; the code's own `NoWorkYet` doc comment claimed it did. | Documented at first; F10 then chose to exclude them, which is now implemented. |
| A18 | Development docs: the worker for real work is MAGPIE; `fake_worker.py` is end-to-end-suite only. | Development "Prerequisites" implied `fake_worker.py` is how you get work done without MAGPIE, and step 6 said "the worker client from step 4" (the fake-worker profile) would complete "real tasks". | Python-worker correction; see section C. |
| A19 | — | Backups, "Verifying a restore": functional smoke via `worker/fake_worker.py --tasks 1` against the restored production stack. | That would write invented results into real jobs. Replaced with a one-task `magpie contribute` run; see section C. |
| A20 | `desired_count` must be 1 (now enforced). | Backups "Decisions settled": "If it ever exceeds 1 the backup task is unaffected…". | PLAN.md's single-instance assumption (imports, in-memory rate limits, per-process SSE) is what the code relies on; the settled decision now says it is enforced. |
| A21 | Worker Client Status table: the opening-rack executor now verified end to end (static player). | "Written; not verified end to end". | Status corrected from evidence gathered in the audit. |

### A.2 Code updated (plan wins)

| # | What the code did | What PLAN.md says | Reasoning |
|---|---|---|---|
| C1 | SPRT, progress, the job list and rating evidence summed every `game_results` row, so with redundancy X each seeded, deterministic batch counted X times. | The SPRT sample size is pairs (or games) *played*; `min`/`max` gate on the same number. | SPRT would see X times its real evidence and stop early on noise; ratings would weight redundant jobs X-fold. All four now read the first accepted result per task (`jobstats::FIRST_GAME_RESULT_PER_TASK`). Test: `redundant_results_for_one_task_count_once`. |
| C2 | Purge and the purge/delete census queried `player_config_ratings.job_id`, which does not exist. **Every purge and every job delete failed.** | Ratings are pool-scoped; purge deletes the job's data. | Ratings are no longer touched by purge (the sweep refits a pool whose evidence shrank). PLAN.md's API table, which said purge deletes "ratings", was corrected to match. Test: `a_job_can_be_purged_and_its_dispatch_counter_resets`. |
| C3 | `audit_log.job_id`/`actor_user_id` and `player_configs.created_by`/`worker_bans.banned_by` referenced `jobs`/`users` with no ON DELETE. **Deleting any job with history, any self-registered user, or any admin who had created a config or ban failed.** | Destructive endpoints write a census "that survives" the deletion; account deletion is supported. | `audit_log` has no foreign keys now (an append-only log must outlive what it describes). `created_by`/`banned_by` are nullable with SET NULL, like `jobs.created_by`. Tests: `a_job_with_history_can_be_deleted_and_its_census_survives`, `a_user_with_history_can_be_deleted`. |
| C4 | Deactivation was unconditional, so deactivate-then-activate restarted a completed job. | "A completed job cannot be reactivated." | The rule was sidestepped. Deactivation now refuses completed jobs (409); PLAN.md's "deactivation is unconditional" wording was refined. Test: `a_completed_job_cannot_be_deactivated`. |
| C5 | Every request with no identity header inserted an `anonymous_workers` row, on any endpoint, before rate limiting. | "`204` when there is no work… a request that arrived with no identity is not assigned a UUID here… gets one for keeps once a task is actually available." | A new contributor polling a quiet server minted an orphan identity every poll. Omitting the header also escaped the per-identity rate limit, and anyone could fill the table. The UUID is now persisted in the claim transaction only when a task is issued. Test: `a_worker_with_no_identity_is_persisted_only_when_given_a_task`. |
| C6 | The per-IP rate limits (register, reset) keyed on the TCP peer, which behind the ALB/Nginx is the proxy. | Limits are per client IP. | Every contributor shared one bucket: 10 registrations an hour site-wide. `clientip.rs` resolves the client behind `TRUSTED_PROXY_HOPS` (set to 1 in ECS and compose). |
| C7 | MAGPIE read the simulation settings as `plies` and `top_plays`. | Player mapping: `num_plies → -plN`, `num_plays → -npN`, `num_plies_recorded → shplies`. | birdtest sends the PLAN.md names. Every simming player silently ran on the worker's ambient plies and play count, so results were not comparable across workers. |
| C8 | MAGPIE ignored the request's `letter_distribution` and used whatever distribution was loaded. | "Letter distributions are stated, not inferred"; the name "is carried on the request the worker receives". | Now applied. Absent means MAGPIE's default for the lexicon, never the ambient one. |
| C9 | The job pins `layout_id` and workers verify the layout's digest, but the request never named a layout, so MAGPIE played on whatever board its settings last loaded. | Layout is pinned by the job like the distribution. | `board_layout` added to all three request types, stored on the request tables, and applied by MAGPIE. Fixtures updated in both repos. |
| C10 | MAGPIE's leave-generation executor *required* `target_rack_count`. | "The generation's rack target is **not** sent… the client passes `leavegen` a target it cannot reach". | birdtest follows PLAN.md and never sends it, so every task failed on a missing key. MAGPIE now uses an unreachable target (`INT_MAX`) and ends on `num_games`. (Other failures remain; see F1.) |
| C11 | MAGPIE ended the whole `contribute` run on any non-200 result submission, including a 400 for one bad result. It also counted `accepted: false` as completed. | "An error *claiming* or *submitting* is handled by the retry policy… and only stops the loop if it exhausts retries." | A 4xx now counts as one task failure (the consecutive-failure guard still applies). `accepted: false` is reported and not counted. |
| C12 | MAGPIE sent the server-supplied artifact key into a URL unchecked. | Client security: "reject artifact keys containing `..` or a leading `/`". | Validated against the server-minted key alphabet. |
| C13 | MAGPIE's half of the contract fixtures was not enforced. | "MAGPIE's half is not yet… or they pin only one side of a two-sided contract." | Fixtures copied to MAGPIE `test/birdtest_contract/`. `test/contribute_test.c` fails if any key the executors read is missing, which would have caught C7. PLAN.md updated to say so. |
| C14 | `infra/variables.tf` described `desired_count` > 1 as safe. | birdtest is single-instance; the import reaper is "load-bearing only here: if birdtest is ever replicated, the import is the first thing that breaks". | A validation now refuses > 1, and the description explains why. |
| C15 | Axum's default 2 MB JSON limit (and Nginx's 1 MB locally) silently capped submissions. | Position capture has "no per-task cap"; a 1,000-game batch is "on the order of 15 MB". | Capture batches would have been refused with 413. `/api/worker/result` now accepts 64 MiB, Nginx matches, and PLAN.md's open question 2 is recorded as settled. |
| C16 | The leave-generation executor named the downloaded KLV `birdtest_leavegen_previous`. MAGPIE infers a leaves file's letter distribution from its name's prefix, so the load failed on every task. | "`leave_generation` executor: Done" (Worker Client Status). | Renamed `<lexicon>_birdtest_previous`. This got past the load failure; the task still fails for the reason in F1, so the Status row now says the executor does not work end to end. |

### A.3 Unresolved at first — now decided

| # | Code | PLAN.md | Decision and outcome |
|---|---|---|---|
| F1 | MAGPIE's `leavegen` forces, counts and reports full 7-tile racks, and derives leave values itself. | birdtest enumerated leaves, forced leaves, and wrote per-leave means straight into the KLV. | **A.** The server now tracks full racks and derives leave values with a port of `rack_list_write_to_klv`. PLAN.md updated. |
| F5 | Public endpoints published anonymous workers' full UUIDs. | "Contributions are tracked and displayed per UUID." | **A.** Public endpoints publish a derived pseudonym; the UUIDs are admin-only. PLAN.md updated. |
| F6 | MAGPIE's opening-rack executor reported only `move`, `score` and `equity`. | Simulated ranking, win%, blended utility and per-ply statistics. | **A.** Implemented in MAGPIE, sharing the captured-position writer. |
| F7 | A null `num_plays_recorded` meant 10 moves to MAGPIE and "everything" to the server. | "A config that does not set it keeps everything." | **A.** Required, at least 1. PLAN.md updated. |
| F8 | Deleting a user destroyed shared captured positions and could not subtract leave occurrences. | Deletion "rolls back every counter". | **A.** Deletion anonymizes; contributions stay. PLAN.md updated. |

---

## B. Bugs and improvements fixed (not PLAN.md discrepancies)

References are to files on the audit branch.

**Claim and submit correctness** (`routes/worker.rs`, `scheduler.rs`, `jobs/registry.rs`)
1. **Submission race.** The claim was read outside the submit transaction and marked completed unconditionally. A timeout reclaiming it in between left it abandoned *and* completed, and the task's live count decremented twice, so another worker could over-fill the task. Now `SELECT … FOR UPDATE OF c` inside the transaction; a retried submission of an accepted result is `accepted: false`, not a duplicate-key 500. Test: `submissions_for_reclaimed_or_already_accepted_claims_change_nothing`.
2. **Decline race.** The same pattern, fixed the same way. `release_claim` now acts only on a still-`claimed` claim, so a double release cannot decrement twice.
3. **Redundancy starvation.** `next_available` offered a worker the oldest open task even when it already held a slot there. The per-identity unique index refused it, three retries hit the same task, and the worker got 204 while other work existed. Now excluded in SQL. Test: `redundancy_does_not_starve_a_worker_holding_a_slot`.
4. **One broken job failed every claim.** Any error in one candidate job (missing config row, missing gen-0 KLV) aborted the whole claim with a 500 that every client retries. Errors are now logged and the job skipped. Test: `a_job_that_cannot_dispatch_does_not_block_the_others`.
5. **Scheduler cost.** Deficit ordering ran `COUNT(*)` over each candidate job's entire claim history on every claim request. Replaced by `jobs.claims_issued`, incremented in the claim transaction and reset by purge. Test: `every_claim_advances_the_dispatch_counter`.
6. Unique-violation retries matched on the English message text (`lc_messages`-dependent). They now match SQLSTATE 23505 via `AppError::db_code`.
7. **Shutdown reason.** It reported `both` whenever the unsupported set was non-empty, even if it named only inactive jobs. It also took the "lowest floor" as a MIN over formatted text ("0.10.0" < "0.9.0"). Both fixed. Test: `a_stale_unsupported_entry_does_not_change_the_shutdown_reason`.
8. `JobFinished` could overwrite an admin's concurrent deactivation; the update is now guarded on `status = 'active'`.
9. Non-claim worker endpoints now return 401 without an identity. Unregistered claims are rate limited by client IP (5/s, burst 30) instead of escaping limits entirely. Test: `requests_other_than_a_claim_require_an_identity`.
10. Post-commit bookkeeping (finish check, SSE) no longer turns an accepted result into a 500 that invites a retry. The finish check computes only the aggregates it needs, not the full stats. The pushed payload now reflects an auto-completion. Full stats are built only for subscribers (`SseBroadcaster::has_subscribers`).

**Admin and API** (`routes/admin.rs`, `routes/auth.rs`, `error.rs`)

11. **Job-creation validation.** Previously accepted:
    - a zero or negative batch size, which retried a seed collision forever;
    - `elo_low` ≥ `elo_high`, which inverts the LLR's sign;
    - `alpha`/`beta` outside (0,1), or summing to 1 or more;
    - out-of-range rack and leave sizes;
    - an unknown variant;
    - `redundancy` < 1 (previously a 500 from the CHECK).

    All violations are now reported together as field errors. Unit tests are in `routes::admin::tests`.
12. **Player-config validation.** Non-positive counts, `stopping_pct` outside (0,100), negative margins and weights, an empty name, and simulation settings without plies (A13) are refused.
13. **Tier activation race.** Two concurrent activations could exceed 100%; now serialized with an advisory lock. Audit rows record the real previous status instead of a hard-coded one.
14. **Leave-gen activation.** Creation writes the gen-0 KLV after commit, so an object-store failure left a job that failed every claim once activated. Activation now builds it if missing.
15. **Reference checks.** Deleting input data or a player config referenced by a rating pool, rating history or clone returned a 500 FK violation; now a 409 naming the reason. More generally, unique and foreign-key violations map to 409, and database error text is no longer returned in 5xx bodies (PLAN.md's API conventions promise this).
16. **Login had no rate limit.** It was an online guessing oracle and an Argon2 CPU sink. Now 10/min per IP and per username.
17. **Configuration.** A malformed `HEARTBEAT_TIMEOUT_SECONDS`, `SESSION_TTL_SECONDS`, `SECURE_COOKIES` or `MIN_MAGPIE_VERSION` silently became the default; it now fails startup. Added `DATABASE_URL` assembly from `DB_*` parts (see F2).

**Performance and robustness**

18. Per-ply rows were inserted one statement each. They are now multi-row, and move inserts are chunked under Postgres's 65,535-parameter limit (a config keeping every move could exceed it).
19. `api_keys.last_used_at` and `anonymous_workers.last_seen_at` were written on every worker request; now at most once a minute.
20. Decline gap reports are capped (32 files, 128-char fields); they were unbounded inserts into `worker_data_gaps`.
21. `leave_gen_stats` counted the gen-0 artifact as a completed generation, so the dashboard showed generation 2 while generation 1 ran.

**Structure and tests**

22. `backend/src/lib.rs` now holds the module tree and router, so `backend/tests/` drives the exact router the binary serves.
23. The tier-2/3 harness (`backend/tests/common/mod.rs`) follows TESTING.md's settled design: template-clone database per test, SQL builders that validate nothing, and failure (not skipping) without `TEST_DATABASE_URL`.
24. Stale comments fixed: the scheduler's "shells out to MAGPIE", `JobStats.workers`, and `LeaveGenStep::NoWorkYet`.

## C. Python worker treated as a production client — corrections

The code already treated MAGPIE as the only production client. Nothing in the
backend special-cases `fake_worker.py`. The docs did not:

1. **RUNBOOK.md §4 "Verifying a restore"** told operators to run `python worker/fake_worker.py --server-url https://<host> --tasks 1` against the restored **production** stack. That writes an invented result into a real job, recorded as a genuine contribution. It is replaced by a one-task `magpie contribute` run, with an explicit warning never to point `fake_worker.py` at production.
2. **PLAN.md "Verifying a restore"** had the same instruction; same fix.
3. **PLAN.md Development:**
   - "Prerequisites" framed `fake_worker.py` as the way the stack does work without MAGPIE;
   - step 6 said the fake-worker profile's client would complete "real tasks".
   Both now say MAGPIE is the only worker client and `fake_worker.py` is an end-to-end-suite instrument.
4. **`worker/fake_worker.py` docstring** now opens with "Test tooling only", explaining why production use corrupts data. It also no longer describes "Glicko" and "anomaly detection", which do not exist (Bradley-Terry ratings and plausibility checks do).
5. PLAN.md's directory tree labels `fake_worker.py` "test stacks only, never production".

## D. MAGPIE `birdtest-contribute` changes

All on `birdtest-contribute`, in files `src/def/contribute_defs.h`,
`src/impl/config.c`, `src/impl/config.h`, `src/impl/contribute.c`,
`test/contribute_test.c` and `test/birdtest_contract/`.

| Change | What was missing | Why birdtest needs it |
|---|---|---|
| Read `num_plies`, `num_plays`, `num_plies_recorded` (were `plies`, `top_plays`, nothing) | C7 | Simming players ran on the worker's own settings; results were not comparable across workers. |
| Reset every per-player setting to MAGPIE's compile-time defaults before applying a request; defaults shared with `config_create` via `CONFIG_DEFAULT_*` | Absent or null fields kept the previous task's or `settings.txt`'s values | A static-player job claimed after a simming one simulated. Also reset: `eq_margin_movegen`, record/sort type, PlayChooser, and the leave-gen bot's settings. |
| Apply `letter_distribution` and `board_layout` from the request (defaults if absent) | C8, C9 | A worker could verify one file's digest and play with another. |
| Leave generation no longer requires `target_rack_count`; runs to `num_games` with an unreachable target | C10 | Every task failed on the missing key. |
| Leave generation names the fetched KLV `<lexicon>_birdtest_previous` | C16 | MAGPIE's lexicon/leaves compatibility check rejected the old name on every task. |
| Opening-rack simulation decided by plies; loads the request's win% model | Detection used `max_iterations`; no win% model was ever loaded | Keeps "simming" meaning the same thing in both job types. |
| A 4xx result submission counts as one task failure; `accepted: false` not counted as completed | C11 | One rejected result ended the contributor's whole session. |
| Artifact key validated before use | C12 | Untrusted input in a URL. |
| `test/birdtest_contract/` plus two tests: `test_contract_fixtures_carry_every_key_contribute_reads` and `test_player_settings_do_not_leak_between_tasks` | C13 | Enforces MAGPIE's half of the contract; pins the reset. |
| `config_get_player_sim_plies` / `_num_plays` / `_max_iterations` getters; `config_contribute_apply_player_settings` exported | — | For the tests above. |

Then, implementing the section F decisions:

| Change | Decision | Why birdtest needs it |
|---|---|---|
| `MAGPIE_VERSION` is `0.1.0`; the contract fixtures' floor matches | F3 | Builds from before the audit's fixes report `0.0.0` and produce wrong results; the server's floor is now `0.1.0`, which only a fixed build meets. |
| `autoplay_results_write_ranked_plays_json`: one writer for an analysed position's ranked plays, used by the opening-rack executor and the captured-position recorder | F6 | A simming opening-rack player now reports win%, blended utility and per-ply statistics up to `num_plies_recorded`. Captured in-game positions were also being listed in move-list (equity) order with simulation statistics attached; both now come in the simulation's ranking (H4). Test: `test_sim_ranked_plays_json`. |
| `AutoplayArgs.leavegen_write_files`, false in contribute mode | F9 | No per-generation KLV, CSV or report files in a contributor's data directory. |
| `leavegen`'s generation-file writes record a failure and return it as `ERROR_STATUS_RW_WRITE_ERROR` from the run, instead of `log_fatal` | F9 | A failed write in a hand-run `leavegen` no longer kills the process. |
| `rack_list_get_rack_equity_json` emits a JSON object | — | It emitted a bare `"racks":[...]`, so the server refused every leave-generation result (H1). Test: `test_rack_list_forced_racks` now parses the output. |

## E. Deployment blockers

### Resolved

| Blocker | Fix |
|---|---|
| **No HTTPS.** The ALB had only a port-80 forward while the backend sets `SECURE_COOKIES=true`; browsers drop Secure cookies over http, so **nobody could sign in**. | HTTPS listener on `var.acm_certificate_arn` (required), port 80 redirects, API rule moved to HTTPS. |
| Purge and job/user deletion broken | C2, C3 |
| Per-IP limits shared site-wide behind the ALB | C6, plus `TRUSTED_PROXY_HOPS=1` in ECS and compose |
| Capture submissions over 2 MB (1 MB locally) refused | C15 |
| MAGPIE contract gaps making simulation, distribution, layout and leave generation wrong or failing | C7–C11, C16 |
| `MIN_MAGPIE_VERSION` could not be set in ECS without editing the task definition; `GITHUB_TOKEN` not wired | `var.min_magpie_version` and optional `var.github_token_parameter_arn` |
| Multi-task deployment would break imports, rate limits and SSE | C14 |
| No CI | `.github/workflows/ci.yml`: backend tests with a Postgres service, frontend check and build, both image builds |
| `backend/.env.example` still set the removed `DATA_PATH` | Cleaned; documents `TRUSTED_PROXY_HOPS` and `DB_*` |

### Resolved by the section F decisions

| Blocker | Fix |
|---|---|
| Leave generation failed on every task | F1, plus H1 |
| RDS rotated the master password every 7 days under a fixed `DATABASE_URL` | F2: hand-set password |
| The shipped version floor refused every contributor | F3: MAGPIE `0.1.0`, floor `0.1.0` |
| `infra/variables.tf` did not parse, so `terraform init` failed | H2 |
| Leave-generation job creation and generation transitions outlast the ALB's 60-second idle timeout | H3 |
| Terraform, clippy and the MAGPIE contract were unchecked | F14, F15 |

---

## F. Decisions

Every question and concern the audit left open, with the options that were
offered. Each now ends with the option chosen and what was implemented.

### Decision index

| # | Question | Blocks | Options | Chosen | Status |
|---|---|---|---|---|---|
| F1 | Leave generation: track full racks or leaves? | Any leave-generation job | A–D | A | Implemented |
| F2 | Database password versus RDS rotation | Production deploy | A–D | C | Implemented |
| F3 | First MAGPIE version and the server floor | Production deploy | A–C | A | Implemented |
| F4 | Local dev database after the in-place schema edits | Running the audited backend locally | A–D | A | Done |
| F5 | Anonymous worker UUIDs are published | Public launch | A–D | A | Implemented |
| F6 | Simulated opening-rack statistics are not reported | Simming opening-rack jobs | A–C | A | Implemented |
| F7 | Captured moves when `num_plays_recorded` is null | Capture jobs without that setting | A–C | A | Implemented |
| F8 | What deleting a user destroys | Public launch (account deletion) | A–C | A | Implemented |
| F9 | MAGPIE `leavegen` writes into the data directory and can kill the process | Leave generation; the MAGPIE GUI | A–C | A | Implemented |
| F10 | Overlapping forced racks, and the generation-transition race | Leave generation at scale | A–D | A | Implemented (overlap only) |
| F11 | Sessions survive a password reset | Before launch (security) | A–C | A | Implemented |
| F12 | Login timing reveals which usernames exist | Before launch (security) | A–B | B | Decided; no change |
| F13 | Claim tokens are not bound to the worker that claimed | Hardening | A–B | A | Implemented |
| F14 | Verification gaps: Terraform, clippy, MAGPIE end-to-end CI, mail backend | Confidence before deploy | A–E (pick any) | A, B, C | Implemented |
| F15 | Contract fixtures are copied by hand between repositories | Keeping the protocol in sync | A–D | D | Implemented |
| F16 | Stats queries grow with each job's history | Scale | A–D | C | Measured |

---

### Blocking — decide before continuing

#### F1 — Leave generation: full racks or leaves?

**Concern.** The first end-to-end leave-generation run found three problems.
Two are fixed (C10, C16); the third is structural:

- MAGPIE's `RackList` forces and counts **full 7-tile racks**, rejecting
  anything else ("forced racks must all be full racks of 7 tiles, found: ?").
  It reports per-rack means and derives leave values itself
  (`rack_list_write_to_klv`).
- birdtest enumerates **leaves** of 1–`max_leave_size` tiles (914,624 for
  English at 6), forces leaves, and writes per-leave means straight into the KLV
  (`klv::build`).

Every leave-generation task fails today. Patching over the error only moves the
failure: 7-tile rows would land outside the server's universe, no generation
would reach its target, and the KLV would stay all zeros. Games, game pairs and
opening racks are unaffected.

**Options.**

- **A. The server tracks full racks.**
  - `seed_generation` enumerates 7-tile racks: 3,199,724 rows per generation for English.
  - Forced racks are full racks, so MAGPIE needs no change.
  - `klv.rs` gains a Rust port of `rack_list_write_to_klv`, pinned by a round-trip test against a real MAGPIE like the existing KLV tests.
  - *For:* MAGPIE's own algorithm stays authoritative; no protocol change.
  - *Against:* about 3.5× the progress rows, and every full rack must reach the target, which is the compute a hand-run `leavegen` does. A statistical derivation is ported, with drift risk; the size of the port has not been assessed.
- **B. The server keeps leaves; MAGPIE changes.**
  - MAGPIE accepts forced leaves (drawing racks that contain them).
  - It reports per-leave counts and means aggregated from what it played.
  - *For:* the server is unchanged; smaller tables.
  - *Against:* changes `leavegen`'s semantics. Per-leave means are not what `rack_list_write_to_klv` computes, so birdtest's KLVs would differ from a hand-run `leavegen`'s. New MAGPIE code on the release path.
- **C. Hybrid: the server coordinates full racks; a worker builds the KLV.**
  - Coverage is tracked on full racks, as in A.
  - When a generation closes, the server dispatches an "aggregate" task carrying the generation's per-rack means.
  - A worker runs `rack_list_write_to_klv` and uploads the KLV; two workers must produce identical bytes.
  - *For:* no algorithm port.
  - *Against:* a new task type and upload endpoint, and trust in a worker-built artifact that every later generation plays with.
- **D. Disable leave generation for now.** Refuse creating `leave_generation` jobs (and hide the type in the admin form) until A, B or C is built.
  - *For:* unblocks launching the other three job types immediately; small change.
  - *Against:* the feature is unavailable in the meantime.

**Recommendation:** D now, then A. D takes the broken path out of production at
once, and A keeps MAGPIE's algorithm as the single source of truth, verifiable
the same way `klv.rs` already is.

**Decision: A — implemented.**

- The universe is every full 7-tile rack. `seed_generation` unranks racks in chunks and `COPY`s them in (3,199,724 rows for English); later generations copy the universe in SQL. `max_leave_size` is gone from the schema, the API, the admin form and validation.
- A leave result must name full racks (plausibility), and is folded in with `UPDATE … FROM UNNEST`, so a rack outside the universe creates no row.
- `klv::FullRackLeaves` ports `rack_list_write_to_klv`/`generate_leaves`: combos-weighted full-rack means, per-leave weights `C(dist − L, R − L)`, value = weighted mean − average. `full_rack_derivation_matches_the_definition` checks it against a brute-force statement of the definition; identical rack means give zero leaves. English derives in about 13 seconds (release), streamed and on blocking threads.
- Tests: `leave_gen::the_universe_and_the_forced_racks_are_full_racks`, `a_result_folds_into_the_generation_and_creates_no_rows`, `a_leave_result_of_partial_racks_is_rejected`; the end-to-end run's leave-generation job (Verification).
- Cost, as predicted: 3.5× the rows, and every full rack must reach the target before a generation closes. Creating an English job takes tens of seconds (H3).

#### F2 — Database password versus RDS rotation

**Concern.** `infra/rds.tf` sets `manage_master_user_password`, so RDS keeps
the master password in Secrets Manager and **rotates it every 7 days by
default**. The service, backup and restore-drill tasks read a hand-written
`DATABASE_URL` from SSM. After the first rotation, new connections fail. The
backend can already assemble its URL from `DB_*` parts (section B, item 17);
nothing in Terraform uses them yet.

**Options.**

- **A. Inject the managed password and redeploy on rotation.**
  - ECS injects `DB_PASSWORD` from the secret's `password` key; `DB_HOST`/`DB_NAME`/`DB_USER` come from Terraform.
  - An EventBridge rule on successful rotation forces a new ECS deployment.
  - `backup.sh` and `restore-drill.sh` learn the same parts, and RUNBOOK §1's "repoint DATABASE_URL" step is rewritten.
  - *For:* rotation stays on; no secret handling in application code.
  - *Against:* the most infrastructure. There is a short window after each rotation where new connections fail until tasks restart.
- **B. As A, and the backend re-reads the secret itself on an authentication failure.**
  - *For:* no failure window.
  - *Against:* adds a Secrets Manager code path to the application, which PLAN.md deliberately avoids ("The process has no SSM code path of its own").
- **C. Turn off managed passwords.**
  - `manage_master_user_password = false`, with the password set by hand in SSM (Terraform never sees it, as for `SESSION_SIGNING_KEY`) and rotated by runbook.
  - *For:* matches the existing SSM design and the current runbook exactly; smallest change.
  - *Against:* no automatic rotation.
- **D. IAM database authentication.**
  - Tasks connect with short-lived IAM tokens instead of a password; sqlx needs a connect hook that refreshes them.
  - *For:* no long-lived database password at all.
  - *Against:* the most application code, and every tool (psql, `pg_dump` in the backup task) needs token handling too.

**Recommendation:** C for launch, since it fits how every other secret is
already handled. Revisit A if rotation becomes a requirement.

**Decision: C — implemented.** `infra/rds.tf` sets a placeholder `password` with `ignore_changes = [password]`; the AWS provider refuses `manage_master_user_password` alongside `password` even when false, which `terraform validate` caught, so that line is gone. The first deploy replaces the placeholder with `aws rds modify-db-instance` and writes `DATABASE_URL` to SSM (README, "Deploying"). RUNBOOK §1's repoint step keeps the password and swaps the host; a new "Rotating the database password" section covers rotation. PLAN.md's state table no longer lists a Secrets Manager password.

#### F3 — The first MAGPIE version and the server floor

**Concern.** `birdtest-contribute` reports `0.0.0`; the server's shipped floor
is `0.0.1`. A production deploy with defaults refuses every contributor. Builds
from before the audit's MAGPIE fixes also report `0.0.0`, and they produce
wrong results: ambient simulation settings, an unapplied distribution and
layout (section D). A floor that admits `0.0.0` admits them too.

**Options.**

- **A. Cut a real MAGPIE version.**
  - Give the contribution-capable build a version (e.g. `0.1.0`) when `birdtest-contribute` is released.
  - Set `min_magpie_version` in Terraform to match.
  - *For:* the floor excludes every pre-audit build, which is what it exists for.
  - *Against:* a MAGPIE release decision has to be made now.
- **B. Closed beta at floor `0.0.0`.**
  - Set `min_magpie_version = "0.0.0"` until public launch, then do A.
  - *For:* no release needed yet.
  - *Against:* any stale `0.0.0` build, including the buggy pre-audit ones, can contribute. Only tolerable with a hand-picked set of contributors.
- **C. A separate protocol version.**
  - The claim carries a `contribute_protocol` integer, and the server's floor applies to that rather than to MAGPIE's semver.
  - *For:* decouples birdtest compatibility from MAGPIE's release numbering.
  - *Against:* an additive contract change in both repositories. A second version number to keep straight.

**Recommendation:** A. The audit's MAGPIE fixes change results, so the floor
has to be able to shut out builds without them, and only a real version can.

**Decision: A — implemented.** MAGPIE's `MAGPIE_VERSION` is `0.1.0`. The server's default floor is `0.1.0` everywhere it is stated: `config.rs`, the migration's column defaults, `infra/variables.tf`, `docker-compose.yml`, both `.env.example` files, the contract fixtures in both repositories, the test harness, and the docs.

#### F4 — Local dev database after the in-place schema edits

**Concern.** Per the pre-release convention, the audit edited
`0001_initial.sql` in place: new columns, dropped foreign keys, and
`board_layout NOT NULL` on the request tables. The running compose database has
the old `0001` applied. A backend rebuilt from the audit branch will refuse to
start on the migration checksum mismatch.

**Options.**

- **A. Reset and reseed.** Drop the schema (README, "After a schema change") and run `scripts/dev.py`.
  - *For:* one command.
  - *Against:* local jobs, results and accounts are lost.
- **B. Dump, reset, restore selectively.**
  - `scripts/dev-dump.sh`, reset, then restore data.
  - Existing request rows need `board_layout` backfilled from their job's layout, or the restore fails on the new NOT NULL column.
  - *For:* keeps local data.
  - *Against:* hand work, for dev data.
- **C. End the single-migration convention now.**
  - Restore `0001` to its `main` form and move the audit's schema changes into `0002`.
  - Existing databases migrate forward.
  - *For:* no reset anywhere. Establishes the append-only discipline PLAN.md says starts at deployment.
  - *Against:* diff churn, and the convention's readability benefit ends before launch.
- **D. Keep running the pre-audit image locally until ready to switch.**
  - *For:* no work now.
  - *Against:* local development does not exercise the audited code.

**Recommendation:** A, unless the local data matters — then B. There is no deployed database, so C buys little yet.

**Decision: A — done.** `docker compose down -v` dropped the local volumes, the stack was rebuilt from this branch, and `scripts/e2e_magpie.py` seeded it through the real API (register, import, player configs, one job of each type).

---

### Before public launch

#### F5 — Anonymous worker UUIDs are published

**Concern.** An anonymous worker's UUID is its only credential, yet
`/api/workers`, job stats `workers` and `/api/jobs/:id/results` return it in
full. The frontend shows 8 characters, but the API returns the whole value.
Anyone reading the leaderboard can:

- claim tasks as that worker;
- submit garbage under its name and get it banned;
- take its attribution.

**Options.**

- **A. Publish a derived pseudonym.**
  - Public endpoints return `anon_id` = the first 16 hex characters of SHA-256(UUID), and `?worker=` accepts it.
  - Real UUIDs move to a new `GET /api/admin/workers`, which the ban page uses instead of the public list.
  - *For:* stable, and unlinkable back to the credential. No schema change.
  - *Against:* API shape change for the leaderboard, results filter and admin ban flow.
- **B. A separate random public id.**
  - A `public_id` column on `anonymous_workers`, minted with the UUID.
  - Otherwise as A.
  - *For:* no relationship to the credential at all.
  - *Against:* a schema change and a second identifier to keep straight.
- **C. Hide anonymous workers publicly.**
  - Public lists aggregate every anonymous worker into one "Anonymous" row.
  - Per-UUID views become admin-only.
  - *For:* smallest exposure.
  - *Against:* anonymous contributors lose any public recognition, which PLAN.md's "displayed per UUID" intends.
- **D. Accept the risk.** Document that anonymous identities are not protected, and point anyone who cares at API keys.
  - *For:* no work.
  - *Against:* bans and attribution for anonymous workers become meaningless.

**Recommendation:** A. It fixes the exposure while keeping PLAN.md's
per-worker display.

**Decision: A — implemented.** Public endpoints (`/api/workers`, job stats `workers`, `/api/jobs/:id/results`) return `anon_id` = `left(sha256(uuid text), 16)` and never the UUID; `?worker=` matches it. `GET /api/admin/workers` returns the same list with `anon_uuid`, and the ban page uses it. `auth::public_anon_id` is the Rust form. Test: `public_endpoints_name_anonymous_workers_by_pseudonym_only`.

#### F6 — Simulated opening-rack statistics are not reported

**Concern.** PLAN.md says the opening-rack executor reports, per move, the
simulated ranking, win%, blended utility and per-ply bingo% and average score.
MAGPIE reports only `move`, `score` and `equity`. A simming opening-rack job
would store analyses without that data, and there is no backfill.

**Options.**

- **A. Implement it in MAGPIE.**
  - After `impl_sim`, read `SimResults` per play: simulated order, win%, utility, and per-ply statistics up to `num_plies_recorded`.
  - *For:* the feature as designed.
  - *Against:* MAGPIE work on the release path.
- **B. Refuse simming player configs for opening-rack jobs** at job creation until A exists.
  - *For:* no job can silently store incomplete analyses; tiny change.
  - *Against:* simulated opening-rack analysis is unavailable meanwhile.
- **C. Drop it from the design.** Opening-rack analyses are static only; remove per-ply storage from the plan.
  - *For:* simplest.
  - *Against:* loses the most valuable use of opening-rack jobs.

**Recommendation:** B now, then A.

**Decision: A — implemented** (in MAGPIE; see section D).

On your question about positions: yes — an opening rack is treated as another position. On the server the two already share storage (`position_analysis_records` → `position_analysis_moves` → `position_analysis_plies`; an opening rack is a record with no CGP), so no server change was needed. The gap was only in MAGPIE's opening-rack executor, which had its own writer that emitted move, score and equity. It now calls the same writer the in-game positions recorder uses, so both report the same fields. Doing that exposed H4: the in-game recorder was listing simulated plays in equity order.

#### F7 — Captured moves when `num_plays_recorded` is null

**Concern.** PLAN.md says a config without `num_plays_recorded` "keeps
everything". For captured in-game positions, MAGPIE reports at most 10 moves
when it is null, so the server's "everything" is really 10. (Opening racks
report every ranked move, and the server truncates.)

**Options.**

- **A. Make it required.** `num_plays_recorded` must be set when a player config is created; the form defaults it to 10.
  - *For:* explicit; worker and server cannot disagree.
  - *Against:* one more required field.
- **B. Null really means everything.** MAGPIE reports every ranked move for a captured position.
  - *For:* matches PLAN.md.
  - *Against:* payloads grow by an order of magnitude or more per position, against the 64 MiB body limit.
- **C. A documented default.** The server fills a fixed default (e.g. 10) into the request when the config leaves it null, and PLAN.md says so.
  - *For:* no config change.
  - *Against:* an implicit value, which is what pinning was meant to eliminate.

**Recommendation:** A.

**Decision: A — implemented.** `player_configs.num_plays_recorded` is `NOT NULL CHECK (>= 1)`, required by the API (a missing field is refused, 0 is a field error), 10 by default on the form, and sent by `seed.py`. The server reads it as a plain integer; for games it reads player 1's, which is the one MAGPIE uses. Test: `a_player_config_must_say_how_many_plays_to_report`.

#### F8 — What deleting a user destroys

**Concern.** Account deletion removes the user's claims, and with them:

- captured in-game positions keyed to those claims, including positions other redundant claims deduplicated against, which are lost from the corpus entirely;
- nothing of what their leave-generation results added to `leave_rack_progress`, which cannot be subtracted, so those contributions stay counted after the account is gone.

**Options.**

- **A. Anonymize instead of delete.**
  - Strip the username, email, password and API keys, mark the account deleted, and keep claims and results under the tombstone id.
  - *For:* no donated compute is lost; personal data is still removed.
  - *Against:* "delete" no longer removes contributions. Counters and SPRT are unaffected, which may or may not be what an admin wants.
- **B. Keep deleting, but make it exact.**
  - Captured positions reference their claim `ON DELETE SET NULL`, so the corpus survives.
  - Per-claim leave occurrences are stored so they can be subtracted.
  - *For:* deletion removes exactly that user's influence.
  - *Against:* a schema change, and storing each leave submission in full (the largest per-claim data leave generation has).
- **C. Keep the current behaviour** (now documented in PLAN.md).
  - *For:* no work.
  - *Against:* both losses remain.

**Recommendation:** A. It preserves donated compute and still removes personal data.

**Decision: A — implemented.** `DELETE /api/admin/users/:id` sets `deleted_at`, replaces username and email with `deleted-<id>` / `<id>@deleted.invalid`, makes the password unusable, clears `is_admin`, deletes API keys, confirmation codes and reset tokens, and increments `session_generation`. Claims, results and counters are untouched. Login, password reset and `CurrentUser` refuse a deleted account; `/api/users` omits it; a second delete is a 404. Test: `a_user_with_history_can_be_deleted` (extended).

#### F9 — MAGPIE `leavegen` writes into the data directory

**Concern.** On every leave-generation task, `leavegen`'s post-generation step
writes a KLV, a CSV and a report into the data directory. If that directory is
not writable, it calls `log_fatal`, killing the contributor process instead of
failing one task. That is harmless-looking for a terminal user and bad for the
planned GUI.

**Options.**

- **A. Skip the writes in contribute mode**, and replace `log_fatal` with an error on the stack.
  - *For:* no stray files; a failure costs one task, not the process.
  - *Against:* a small `leavegen` change.
- **B. Redirect the writes to a temporary directory** in contribute mode.
  - *For:* keeps the files for debugging.
  - *Against:* still writes on every task; still needs the `log_fatal` fix.
- **C. Keep it.** Document that `./data` must be writable.
  - *For:* no work.
  - *Against:* the process-kill remains.

**Recommendation:** A, alongside whatever F1 decides.

**Decision: A — implemented** (in MAGPIE; see section D). The contribute executor sets `leavegen_write_files = false` for its run. The CLI still writes, and a failed write there is returned as an error instead of `log_fatal`. The end-to-end run checks that no `_gen_` or report file appears in MAGPIE's data directory.

---

### Can wait, but decide deliberately

#### F10 — Overlapping forced racks, and the generation-transition race

**Concern.**

- **Overlap.** Concurrent leave-generation claims can be handed the same lowest-count racks. That is correct, but wasteful at scale.
- **Transition race.**
  - A claim being issued (not yet committed) is invisible to another claim's "is anything in flight?" check, so a generation can close while a task for it is going out. That task's results then land in a closed generation and are ignored.
  - Two claims can also both run the transition. The results are idempotent, but the work is duplicated.

Both matter only once F1 is resolved.

**Options.**

- **A. Exclude racks already out.**
  - Skip racks named in `leave_requests.forced_racks` of open claims (array overlap, with a GIN index).
  - *For:* no duplicate coverage.
  - *Against:* a heavier query on every claim.
- **B. Randomize the selection.**
  - Draw `racks_per_task` at random from the lowest ~5× that many.
  - *For:* cheap; overlap falls sharply.
  - *Against:* statistical rather than exact.
- **C. Lock the transition.**
  - A per-job advisory lock serializes transitions and the in-flight check.
  - *For:* closes the race.
  - *Against:* does nothing for overlap. Combine with A or B.
- **D. Keep both as they are.**
  - *For:* no work.
  - *Against:* wasted compute and occasionally discarded results.

**Recommendation:** B + C, after F1.

**Decision: A — implemented.** Claim-time selection anti-joins the `forced_racks` of the job's open claims for the generation. Test: `racks_out_with_an_open_claim_are_not_handed_out_again`.

As noted when choosing, A does not address the transition race: a claim still being issued is invisible to the in-flight check, and two claims can both run a transition. Option C (a per-job advisory lock) would close both. The transition now costs more than when this was written — about 13 seconds of derivation plus copying 3.2 million rows — so duplicated transitions waste more, though they still produce the same result. Worth revisiting with C.

#### F11 — Sessions survive a password reset

**Concern.** Sessions are stateless Paseto tokens. A password reset clears only
the resetting browser's cookie; any other session, including an attacker's,
stays valid for up to 7 days. PLAN.md documents this as a v1 gap.

**Options.**

- **A. A per-user session generation.**
  - A `session_generation` integer on `users` is embedded in each token and compared on every request (the `users` row is already read per request).
  - Password reset and a new "sign out everywhere" bump it.
  - *For:* real revocation for one column and one comparison.
  - *Against:* a schema change.
- **B. A session table.**
  - *For:* per-session revocation and a list of active sessions.
  - *Against:* a lookup per request, and cleanup.
- **C. Keep the documented gap.**
  - *For:* no work.
  - *Against:* a reset does not evict someone who already has access.

**Recommendation:** A.

**Decision: A — implemented.** `users.session_generation` is embedded in every session token (`gen`) and compared in `CurrentUser`. A password reset, account deletion and the new `POST /api/auth/sign-out-everywhere` (an account-page button) increment it. A token without `gen` is refused, so sessions minted before this change end once. Tests: `bumping_the_session_generation_revokes_earlier_sessions`, `signing_out_everywhere_revokes_the_callers_own_session_too`.

#### F12 — Login timing reveals which usernames exist

**Concern.** An unknown username returns immediately; a known one pays an
Argon2 verify. The timing difference enumerates accounts. PLAN.md documents
this as accepted. Login is now rate limited, which slows but does not stop it.

**Options.**

- **A. Verify against a fixed dummy hash on a miss**, so both paths cost the same.
  - *For:* closes the leak in a few lines.
  - *Against:* an unknown username costs the same CPU as a known one.
- **B. Keep the documented gap.**
  - *For:* no work.
  - *Against:* usernames are enumerable. They are already public via `/api/users`, which is why PLAN.md accepted it.

**Recommendation:** B, since usernames are already published by `/api/users`. Choose A if that list ever stops being public.

**Decision: B — no change.** PLAN.md's accepted-risk text stands; usernames are published by `/api/users`.

#### F13 — Claim tokens are not bound to the worker that claimed

**Concern.** Heartbeat, decline and result accept any registered identity that
presents a valid claim token. The token is 122 random bits, so this is not
exploitable by guessing. But a banned worker can hand its tokens to another
identity, and a result is audit-logged under whoever submitted it while it is
credited to whoever claimed.

**Options.**

- **A. Require the submitting identity to match the claim's owner** on heartbeat, decline and result; a mismatch is treated like an unknown token.
  - *For:* bans and audit rows mean what they say. MAGPIE and `fake_worker.py` already always use one identity.
  - *Against:* a small change; a worker that switches API key mid-task (same account) still works, one that switches account does not.
- **B. Keep the token as the sole credential.**
  - *For:* no work.
  - *Against:* the gaps above.

**Recommendation:** A.

**Decision: A — implemented.** Heartbeat, decline and result look the claim up by token *and* owner (`claimed_by_user_id` / `claimed_by_anon_uuid IS NOT DISTINCT FROM` the caller). A mismatch behaves exactly like an unknown token: the heartbeat is a no-op, decline is 404, result is `accepted: false`. Test: `a_claim_token_works_only_for_the_identity_it_was_issued_to`.

#### F14 — Verification gaps (pick any)

**Concern.** Several checks exist on paper only:

- Terraform was never validated.
- `cargo clippy` is not run; TESTING.md specifies `-D warnings`.
- There is no nightly MAGPIE end-to-end test (TESTING.md tier 6). This audit's scratch script found two MAGPIE failures that no test caught.
- The file mail backend two tests need does not exist.
- The restore round trip is not scheduled.

**Options — independent, choose any combination.**

- **A.** Add `terraform fmt -check` and `terraform validate` (no AWS credentials needed) to CI.
- **B.** Add `cargo clippy --all-targets -- -D warnings` to CI, fixing whatever it reports first.
- **C.** Add a nightly workflow:
  - build MAGPIE `birdtest-contribute` and install data with `download_data.sh`;
  - run the backend;
  - run one real task each of `games`, `game_pairs` and `opening_rack` (and leave generation once F1 is done), asserting the results are accepted.

  The audit's scratch script is a starting point.
- **D.** Build the file mail backend and add the restore round trip to that nightly workflow.
- **E.** Defer all of them.

**Recommendation:** A, B and C. C is the check that would have caught the
MAGPIE contract bugs this audit found by hand.

**Decision: A, B and C — implemented.**

- **A.** CI's `terraform` job runs `fmt -check`, `init -backend=false` and `validate`. Running them locally first found H2 and the F2 provider conflict, both fixed; `ecs.tf` was reformatted.
- **B.** CI's backend job runs `cargo clippy --locked --all-targets -- -D warnings`. It reported three findings, fixed: a `large_enum_variant` on `registry::Acquired` (allowed, with the reason), `Matrix::len` without `is_empty`, and a `chunks_exact` in a test.
- **C.** `scripts/e2e_magpie.py` runs one real task per job type (games, game pairs, a static and a simming opening-rack player, leave generation) against the compose stack, seeding through `seed.py`, and asserts what the server stored. `.github/workflows/nightly.yml` runs it daily and on demand, building MAGPIE `birdtest-contribute` (or a chosen ref) with cached data. Its first run found H1.

#### F15 — Contract fixtures are copied by hand

**Concern.** The assignment fixtures now live in both
`birdtest/contract-fixtures/` and `MAGPIE/test/birdtest_contract/`. Each side
tests against its own copy, so a change in one repository that is not copied to
the other passes both test suites while breaking the protocol.

**Options.**

- **A. Manual copy** (current), with the rule in both READMEs.
  - *For:* no tooling.
  - *Against:* relies on memory; exactly the failure mode fixtures exist to prevent.
- **B. MAGPIE's CI fetches the fixtures** from birdtest at a pinned tag or commit before running `magpie_test contribute`.
  - *For:* one source of truth; bumping the pin is an explicit act.
  - *Against:* MAGPIE's CI gains a network dependency on birdtest.
- **C. A shared fixtures repository** included in both as a git submodule.
  - *For:* symmetric; versioned.
  - *Against:* submodule friction, and a third repository.
- **D. birdtest's CI checks out MAGPIE `birdtest-contribute`** and runs its contribute test against birdtest's fixtures.
  - *For:* catches a server-side change immediately.
  - *Against:* birdtest's CI builds MAGPIE, which is slow; it does not catch MAGPIE-side changes.

**Recommendation:** B, or D if F14's option C is adopted — that workflow builds MAGPIE anyway.

**Decision: D — implemented.** CI's `magpie-contract` job checks out MAGPIE `birdtest-contribute`, copies this branch's `contract-fixtures/*.json` over `test/birdtest_contract/`, restores MAGPIE's data from cache (downloading on a miss), builds `magpie_test` and runs `contribute`. As noted, it catches server-side fixture changes, not MAGPIE-side ones; the nightly end-to-end job covers those.

#### F16 — Stats queries grow with each job's history

**Concern.** Several queries aggregate over a job's entire history:

- **Every game-job submission** recomputes the SPRT aggregates over the whole job (one result per task).
- **The job list** sums games for every listed job.
- **The job detail page and SSE** aggregate every analysed position of an opening-rack job.
- **The rating sweep** rereads all paired results every two minutes.

These are fine at today's volume; PLAN.md chose "no pre-aggregation for v1". They grow without bound.

**Options.**

- **A. Running per-job aggregates.** Keep wins, losses, pentanomial and position counts in a per-job row, updated in the submit transaction.
  - *For:* constant-time reads.
  - *Against:* denormalized counters to keep correct under redundancy, purge, user deletion and restore. RUNBOOK §2.3 would need to recompute them.
- **B. Debounce.** Evaluate the finish condition and push SSE at most once every few seconds per job.
  - *For:* bounds cost under load with little code.
  - *Against:* a job can overshoot its SPRT stop by a few seconds of results.
- **C. Measure first.** Load-test with a realistic job (e.g. 400,000 games; a full English opening-rack job) and choose A or B from the numbers.
  - *For:* no premature complexity.
  - *Against:* time spent on the load test.
- **D. Keep as is** until it hurts.
  - *For:* no work.
  - *Against:* degradation arrives in production, under load.

**Recommendation:** C, then B, then A if still needed.

**Decision: C — measured.** Synthetic volume in a throwaway database on the local compose Postgres (16, default settings: 128 MB `shared_buffers`, 2 parallel workers; 12 cores), 2.7 GB in all:

- a `game_pairs` job of 400,000 pairs and a `games` job of 400,000 games, each task completed once and every tenth task twice (as under redundancy 2);
- 40 more paired jobs over 20 configs in one rating pool (600,000 paired results in all);
- an opening-rack job of 1,000,000 analysed racks at 10 moves each (10 million move rows; a full English job is 3.2× this);
- a leave-generation job's 3,199,724 progress rows.

The queries are copied verbatim from the code; the load and query scripts were run from a scratch directory and are not committed. Warm times, best of two:

| Query | Runs | Time |
|---|---|---|
| `game_pair_stats` over 400,000 pairs | every paired submission and SSE push | 54 ms |
| `game_stats` over 400,000 games | every game submission and SSE push | 50 ms |
| `list_jobs`, 42 game jobs | every job-list page view | **2,188 ms** |
| `opening_rack_stats`: racks analysed | job detail and every SSE push | **2,086 ms** |
| `opening_rack_stats`: best-move types | job detail and every SSE push | 541 ms |
| Rating sweep `build_matrix`, 600,000 paired results | every two minutes | 452 ms |
| `worker_contributions`, 44,000 claims | job detail and every SSE push | 136 ms |
| Public worker list, all claims | page view | 93 ms |
| Leave `next_step` rack selection | every leave claim | 47 ms |
| `leave_gen_stats` | job detail and every SSE push | 210 ms |
| Transition: stream generation 1 by rack | once per generation | 674 ms |
| Transition: copy the universe to the next generation | once per generation | **56–66 s** |

What the numbers say:

- **The SPRT path is fine.** The aggregates that run on every game submission take about 50 ms at 400,000 units, well within budget for the result rate a job actually sees. Neither A nor B is needed there yet.
- **The job list is the first real problem.** It recomputes one-result-per-task game totals for every listed job on every page view, and grows with total results across all jobs: 2.2 s here.
- **Opening-rack stats are the second.** `COUNT(DISTINCT rack)` over the job's records takes 2.1 s at a million racks, so about 7 s for a full English job, on the detail page and every SSE push.
- **Copying the universe is slow on this under-provisioned database** (a minute, against about 15 seconds for a whole transition on the smaller dev database in the end-to-end run). It runs once per generation on a detached task (H3), so it costs time, not correctness.

**Recommendation:**

1. Store running totals for the two expensive reads, not for SPRT: a per-job `units_completed` for games and pairs (maintained in the submit transaction, on the first accepted result per task), read by the job list, and a per-job analysed-rack count for opening-rack jobs. Purge and restore would reset or recompute them (RUNBOOK §2.3).
2. Debounce SSE stat pushes per job (B) once opening-rack jobs run at scale.
3. Leave the SPRT aggregates as they are until a job's submission rate makes 50 ms matter.
4. Re-measure the universe copy on the production instance class before running multi-generation English jobs.

## G. Clarifications against the audit brief

- The brief described a "priority-tier **weighted-random** task scheduler". Both PLAN.md and the code implement a **deterministic deficit-based** scheduler ("no randomness is involved", with a stated rationale), so nothing was changed.
- The brief described "**per-job** ELO ratings". Both implement **pool-scoped Bradley-Terry** ratings, siloed from job control flow. Per-job SPRT is the stopping rule. Nothing was changed.
- `is_admin` handling matches PLAN.md: re-read from the database on every request, settable by no endpoint, and enforced by the `AdminUser` extractor on every admin route (all state-changing admin routes also verify CSRF).

## H. Found while implementing the decisions

- **H1. MAGPIE's leave-generation result was not JSON.** `rack_list_get_rack_equity_json` wrote `"racks":[...]` without the enclosing object, so the server refused every leave result with 400 ("Failed to parse the request body as JSON"). Earlier runs never got that far. MAGPIE also exits 0 after giving up on five consecutive failures, so a run that checked only the exit code would have passed; the end-to-end script now checks the output and the stored rows. Fixed in MAGPIE, and its test now parses the output instead of matching a substring.
- **H2. `infra/variables.tf` did not parse.** A description contained unescaped double quotes, so `terraform init` failed before anything else could run. Fixed; CI now validates.
- **H3. Leave-generation work outlasted the load balancer.** Creating an English leave-generation job writes 3.2 million rows inside the request, and a generation transition runs inside a worker's claim request. The ALB's default idle timeout is 60 seconds. When a proxy gives up, axum drops the handler future, which would roll back a creation part-way, or abandon a transition part-way on every attempt, so a generation whose transition outlasts the timeout would never close. Fixes: seeding uses `COPY` (37 seconds for an English job on this machine, against 54 with batched `INSERT`s); the transition runs on its own task, awaited, so a dropped request cannot cancel it; and the ALB's `idle_timeout` is 300 seconds (MAGPIE's own request timeout is 120). SSE streams send keep-alives every 15 seconds, so the longer timeout does not change them.
- **H4. Captured in-game positions were not in simulation order.** The positions recorder read `sim_results_get_simmed_play(i)`, which is move-list (equity) order, and attached each play's simulation statistics. The "top N" stored for a simming player was the top N by static equity, not by simulation. It now reads the sorted display copies, as MAGPIE's own output does, through the writer F6 shares.

## I. Worth deciding next

Not blocking, but each is a decision rather than a bug, and the evidence for it
is above.

### I1. The generation-transition race (follow-up to F10)

F10's option A stopped concurrent claims being handed the same racks. As noted
when it was chosen, it does nothing for the transition itself:

- a claim still being issued is invisible to another claim's in-flight check, so a generation can close while a task for it is going out, and that task's results are ignored;
- two claims arriving together can both run the transition.

Both produce the right KLV, but with full racks (F1) a transition now costs about
15 seconds on the dev database: streaming 3.2 million rows, deriving leave
values, and copying the universe, which took about a minute on the F16 test
database. A duplicate wastes all of that.

- **A. Add F10's option C:** a per-job advisory lock taken around the in-flight check and the transition, so only one runs and no claim for the closing generation can be issued meanwhile.
  - *For:* closes both races; small change.
  - *Against:* claims for that job wait on the lock while a transition runs; claims for other jobs are unaffected.
- **B. Keep it.**
  - *For:* no work.
  - *Against:* occasional duplicated transitions and discarded task results as leave generation scales.

**Recommendation:** A, before running multi-generation English jobs.

### I2. The two expensive stats reads (follow-up to F16)

F16's measurements put the per-submission SPRT aggregates at about 50 ms for
400,000 games or pairs, which is fine. Two reads are not:

- the **job list** recomputes one-result-per-task game totals for every listed job on every page view: 2.2 s at the test volume, growing with total results across all jobs;
- **opening-rack stats** count distinct analysed racks on the detail page and every SSE push: 2.1 s at a million racks, about 7 s projected for a full English job.

- **A. Running totals for just those two:** a per-job `units_completed` for games and pairs (maintained in the submit transaction, on the first accepted result per task) and a per-job analysed-rack count for opening-rack jobs. Purge and restore would reset or recompute them (RUNBOOK §2.3).
  - *For:* both reads become constant-time; SPRT keeps reading the source rows.
  - *Against:* two denormalized counters to keep correct.
- **B. Debounce SSE stat pushes per job**, and cache the job list briefly (single instance, so in-process).
  - *For:* no schema change.
  - *Against:* the first view after the cache expires still pays the full cost, and stats lag by a few seconds.
- **C. Both.**
- **D. Keep as is** until opening-rack jobs run at full size.

**Recommendation:** A now, adding B once opening-rack jobs run at scale.

### I3. Re-measure the universe copy on the production instance

> **Superseded in part by K-D9.** The measurement is still wanted. The fallback
> this section proposes — treat a missing row as zero and anti-join instead of
> copying — does not work, for the reason given in K-D9, and is withdrawn.

Copying 3.2 million leave rows to the next generation took 56–66 seconds on the
F16 test database (the local compose Postgres at default settings, 2.7 GB of
data), against a whole transition of about 15 seconds on the smaller dev
database. The production instance class (`db_instance_class`) is not
benchmarked. Measure it before running multi-generation English jobs. If it is
slow there too, the copy can be avoided: treat a missing row as zero
occurrences, and select a generation's racks by anti-joining the previous
generation's rows instead of copying them.

---

## J. Second pass (2026-09-13)

### J.0 Starting state, and what this pass did with it

- **Branch.** This pass continued on `audit/birdtest-2026-09-11` instead of creating another branch. That branch was already made off `main` (at `baa4094`) for this audit and holds the first pass (3 commits, pushed). A new branch off `main` would have either dropped that work or duplicated it.
- **Uncommitted work was already in the tree.** It implemented I1 and I2 (J.1). It also removed every citation of this file and its decision IDs (`F8`, `AUDIT_FINDINGS.md F10`, …) from code, comments, workflows and PLAN.md, and staged this file for deletion.
  - **Kept:** the removal of citations from code. A comment should explain itself, not point at an audit record that will go stale.
  - **Reverted:** the deletion. The audit brief requires this file as the record of every decision. **Flag for a human:** if the deletion was intentional (the record meant to live in the pull request instead), drop the file at merge.
- Nothing uncommitted was discarded. It was reviewed and run (clippy and every test passed as found), and two bugs in it were fixed (J1, J2).

### J.1 Section I: status

| # | Chosen | What is implemented | Tests |
|---|---|---|---|
| I1 | A, extended | A per-job advisory lock (`leave_gen::lock_claim_decisions`, `pg_advisory_xact_lock`) held for every leave-generation claim decision. A `leave_generation_transitions` row (key: job, generation) is **committed** by the claim that finds a generation complete, so exactly one request runs the transition; the rest get 204. A transition not finished after 30 minutes is taken over and `attempts` records it. A transition that fails hands ownership back at once by backdating `started_at`. The lock is not held across the transition, which would hold a transaction open across an S3 upload. The lock alone (option A as written) would not have stopped a second transition, because the transition runs after the claim transaction commits; the committed row is what does. | I-LEAVE-11 to 14 |
| I2 | A only | `jobs.games_completed` (games and game pairs) and `jobs.racks_analyzed`, incremented in the submit transaction on a task's first accepted result. The job list and opening-rack stats read them, purge zeroes them, and RUNBOOK §2.3 recomputes them after a partial restore. SPRT still reads `game_results`, so a drifted counter cannot stop a job. Debouncing (option B) is not implemented and is recorded under PLAN.md's future improvements. | I-STATS-5, 5b, 5c, 5d |
| I3 | **Open** | `scripts/leave-gen-bench.sh` times the universe copy and the ordered stream against any database, inside a rolled-back transaction. It has not been run against the production instance class, which needs production access. | — |

I1 and I2 were implemented in the working tree before this pass began, and nothing records who chose the options. They are recorded here as implemented and verified, not as decisions this pass made.

### J.2 Discrepancies found this pass

| # | What the code did | What PLAN.md says | Decision | Reasoning |
|---|---|---|---|---|
| J1 | `registry::count_first_result` decided "first accepted result for this task" by counting the task's result rows, before anything locked the task row. Two redundant claims submitting together each counted only their own uncommitted rows, both concluded they were first, and the job's `games_completed` doubled (reproduced: 4 instead of 2). Its doc comment said the shared transaction made this safe. | Running totals are incremented "once per task, on its FIRST accepted result". | **Code updated** | A real race, and PLAN.md states the intent. `submit_result` now locks the task row (`SELECT … FOR UPDATE`) right after the claim row, before `store_result`, so submissions for one task serialize and the count sees earlier ones as committed. The lock order (claim, then task, then job) is the one the claim, decline and reclaim paths already use, so no deadlock cycle is introduced. Test: `worker_api::concurrent_redundant_results_count_once`. It is deterministic: an outside transaction holds the job row until both submissions are waiting on a lock. It failed before the fix and passes after. |
| J2 | `registry::acquire` re-dispatched `available` tasks for every job type before the type-specific path. For leave generation that was: **(a)** before `lock_claim_decisions`, so a reissued claim was invisible to a concurrent claim's in-flight check, the race I1 closed for new tasks left open for reissued ones; **(b)** for any generation, so once a generation closed, its reclaimed tasks were handed out again. The worker played them with an outdated KLV, the result could only be discarded, and the racks could overlap a fresh task's. | Leave claim step 2: "The whole of step 2 runs under a per-job advisory lock (… taken before anything is read …)". Request Handling: a task with capacity left "is re-dispatched before anything new is generated", stated for all job types. | **Code updated**; PLAN.md amended to match | The lock section states the invariant the rest of leave generation depends on. The generic re-dispatch rule predates generations and did not consider them. Leave jobs now reissue inside `generate_leave_gen`, after the lock, and only tasks whose `leave_requests.generation` is current: `next_available` gained a generation filter, and `leave_gen::current_generation` was extracted from `next_step`. PLAN.md step 2 gained a paragraph saying so. Test: `leave_gen::a_reclaimed_task_is_reissued_only_while_its_generation_is_open`. It failed before the fix (a generation-1 task handed out after generation 1 closed) and passes after. **Side effect**, also in PLAN.md: a task left over from a closed generation stays `available` and is never dispatched, so a leave job's task counts in the job list can include a few that never complete. |
| J3 | A timed-out claim is abandoned, and its late submission is refused (`c.state = 'claimed'` in `submit_result`). A generation closes only when none of its claims is still `claimed`. | "The lock above makes that rare rather than impossible: a claim that times out is reissued, and the original worker can still submit after its generation has closed." The leave fold's code comment and its test's docstring said the same. | **PLAN.md updated**, and both comments | The code was right; the scenario cannot happen. After J2 the closed-generation guard in `LeaveGenHandler::insert_record` cannot be reached through the claim flow. It is kept as a cheap guard against state the flow never writes (a partial restore, a hand edit), and its comment now says so. |

Also: `scripts/e2e_magpie.py` still cited `F1` and `F9`, and those citations were removed like the rest.

### J.3 Python worker as a production client

Every mention was re-checked:

- README, TESTING.md, RUNBOOK.md, PLAN.md;
- `docker-compose.yml`, `docker/Dockerfile`, `.env.example`;
- `scripts/dev.py`, the plausibility tests, and `worker/fake_worker.py` itself.

All of them describe `fake_worker.py` as end-to-end-suite tooling, and MAGPIE as the only production client. Nothing under `infra/` references the fake worker. **No corrections were needed this pass.** Section C lists the first pass's corrections.

### J.4 MAGPIE `birdtest-contribute`

Verified this pass; **no changes needed, and none were made.**

- The local branch is at `62fb6f37`, even with `origin/birdtest-contribute`, with a clean tree.
- The contract fixtures present in both repositories (`assignment-games.json`, `assignment-leave-generation.json`) are byte-identical.
- `make magpie_test` (the dev build, with ASan and UBSan) builds, and `magpie_test contribute` passes.
- Nothing this pass or the uncommitted work changed touches the wire format: J1, J2 and I1/I2 are all internal to the server.
- End-to-end with a real MAGPIE: see J.6.

### J.5 Deployment blockers

None new. One note that becomes a blocker for any existing database:

- **Migration checksum.** The uncommitted work added `jobs.games_completed`, `jobs.racks_analyzed` and `leave_generation_transitions` by editing `0001_initial.sql` in place, per the pre-release convention. **A database built from the earlier `0001`, including a local compose volume, will refuse to migrate.** No deployed database exists, so this is the F4 situation again: reset the local database (README, "After a schema change"). This pass did **not** reset the local compose database, because doing so deletes local data.

### J.6 Verification (this pass)

- `cargo clippy --locked --all-targets -- -D warnings`: clean.
- `cargo test`: 82 unit tests (3 ignored, as before) and 30 integration tests against a real Postgres (admin 4, auth 3, leave generation 9, worker 12). Both new tests were run against the code before their fixes and failed as described in J1 and J2.
- Frontend `npm run check`: 0 errors, 0 warnings.
- MAGPIE `magpie_test contribute`: passes.
- End-to-end with a real MAGPIE (`scripts/e2e_magpie.py`): **passes.** Run against this branch's backend in an isolated compose project (`COMPOSE_PROJECT_NAME=birdtest-e2e`, separate ports and volumes, so the local `birdtest` stack and its data were untouched), with a release build of MAGPIE `birdtest-contribute` at `62fb6f37`, NWL23, data-20251004.
  - Every job type got 2 accepted claims: games, game pairs, opening rack (static and simming), and leave generation. The English leave-generation job took 33.9 s to create.
  - **Not covered by this run:** a real generation transition. The script does not force one, so the commit-before-transition path (I1) is verified by the integration tests (I-LEAVE-11 to 14) and not yet against a real object store upload. The first pass's hand-forced transition predates I1. The nightly workflow is the next place it would run.
  - The isolated project was torn down afterwards.

---

## K. Third pass (2026-09-13)

### K.0 Starting state, and the branch

- **Branch: `audit/birdtest-2026-09-13`**, cut from `audit/birdtest-2026-09-11`
  at `f873986` rather than from `main`. That branch is itself off `main` (at
  `baa4094`) and carries the first two passes, none of which is merged yet.
  Branching from bare `main` would have either discarded that work or
  duplicated it, so the lineage to `main` is preserved and the date in the name
  is this pass's. **If the intent was a branch literally off `main`, this is the
  one thing to correct at merge time.**
- The tree was clean and everything in it passed as found: clippy, 82 unit
  tests (3 ignored), 30 integration tests, `npm run check`.
- Section I's open item, **I3** (benchmark the universe copy on the production
  instance class), is still open. It needs production access, which this pass
  did not have. `scripts/leave-gen-bench.sh` is still the tool for it.

### K.1 Race conditions found and fixed

Both are in leave generation, and both are in the window the second pass's I1
work opened rather than closed: I1 made exactly one request run a generation's
transition, but nothing said what the *rest* of the job may do while it runs.

| # | The race | Fix | Test |
|---|---|---|---|
| **K1** | A task left `available` by a lapsed claim was **reissued during its generation's transition**. A generation does not read as closed until the transition commits its artifact row, so for the tens of seconds a transition takes (streaming millions of rows, deriving leave values, uploading the KLV) `current_generation` still named the closing generation — and `registry::acquire`'s reissue step runs before `next_step`, which is the only thing that knew a transition was running. The worker played the task and its occurrences were folded into the very rows `generation_klv` was streaming, so the uploaded KLV no longer reproduced from the database. A hash mismatch is the one signal `rebuild_artifacts` reserves for a corrupted object, so this would have shown up later as a false corruption report, with no way to tell it from a real one. | `leave_gen::transition_in_progress`, checked under the claim lock before the reissue. A row past the takeover timeout deliberately does not count, so a transition whose process died is still taken over rather than stalling the job forever — the same bound `next_step` takes over on, so the two decisions cannot disagree. | `leave_gen::no_task_is_issued_while_a_generations_transition_runs`. Fails before the fix (200 with a generation-1 task), passes after. |
| **K2** | A **purge racing a running transition**. Purge deletes the `leave_generation_transitions` row, the artifacts, the progress rows, and reseeds generation 1 — all while a transition spawned before it may still be streaming. The transition then wrote a generation-1 artifact derived from results the job no longer had, and copied the freshly zeroed universe into generation 2, so the purged job believed generation 1 was done and would never redo its work. | `run_transition`'s closing transaction was split out as `leave_gen::close_generation`, and setting `completed_at` is now conditional on the row this request committed still being there and still open. If it is gone, nothing is written and the call fails loudly. The uploaded object is left behind: it is keyed by job and generation, a later transition overwrites it, and `/api/worker/artifact` serves no key no `leave_generation_artifacts` row names. | `leave_gen::a_transition_whose_job_was_purged_meanwhile_closes_nothing`. Fails before the fix, passes after. |

A third concurrency problem, **K3**, is a throughput failure rather than a
correctness one and is listed below with the other bugs.

**Checked and found sound**, so nothing was changed: the claim/submit lock order
(claim row, then task row, then job row, on every path that takes more than one);
`reclaim_expired` against a concurrent submission (the submit path holds the
claim row from lookup to commit and the reclaim re-checks `state = 'claimed'`
after waiting on it); `release_claim`'s single-decrement guard; `JobFinished`
guarded on `status = 'active'`; the tier-activation advisory lock; `next_available`'s
`FOR UPDATE SKIP LOCKED`; and `count_first_result`, whose correctness rests on
the task-row lock the second pass added (J1).

### K.2 Other bugs and oversights fixed

| # | What was wrong | Fix | Test |
|---|---|---|---|
| **K3** | **Concurrent claims against one job collided on the seed cursor and the losers were told there was no work.** The next seed is `MAX(seed)`, invisible to a concurrent claim's uncommitted task, so overlapping claims all compute the same one; the `(job_id, seed)` unique index catches it only by failing the loser, and `scheduler::claim` gives up after three attempts. Past three-way contention on one job — which the deficit scheduler actively produces, since it points every worker at the most-behind job — a worker got `204` while work existed. | The per-job advisory lock leave generation already used, generalised as `jobs::lock_job_dispatch` and taken by all four generators. It costs nothing that was not already being paid: `issue_claim` bumps `jobs.claims_issued`, holding that job's row lock until commit, so claims against one job already serialize. It only moves the start of that window earlier, turning a lost race into a short wait. | `worker_api::concurrent_claims_tile_the_seed_space_instead_of_colliding` (8 concurrent claims). Fails before the fix — a worker gets `204` — passes after. |
| **K4** | **An opening-rack submission was never checked against the task it answered.** Every other job type has that rule. Too few racks and the task still *completed*, leaving a hole in the rack space nothing revisits, because the job's finish condition only asks whether every task completed. Racks from nowhere were stored as analyses of this job and added to `jobs.racks_analyzed`, the progress counter the dashboard reads. | `opening_rack::check_batch_against_task`, called from `store_result` alongside the game batch-size check. The request names the racks rather than only how many, so the set is compared rather than the size; order is not part of the contract. The range is re-expanded from `rack_start`/`rack_count`, which costs what dispatching it cost. | `worker_api::an_opening_rack_result_must_answer_the_racks_it_was_given`. Fails before the fix (`accepted: true` for a one-rack answer to a three-rack task), passes after. |
| **K5** | **One bad rating pool silenced the whole rating sweep.** `recompute_stale` propagated the first pool's error, so every pool ordered after it stopped being refit — silently, for as long as the misconfiguration lasted. A pool whose anchor is no longer a member is an admin-reachable way to produce exactly that. | Each pool is logged and skipped; the returned count is of the fits that actually ran. | — (the failure needs a hand-built pool; the change is a `match` around one call) |
| **K6** | **The rate-limit bucket maps grew without bound.** `governor`'s keyed limiters keep one entry per key forever, and every key is outside input: a worker UUID, a client address, a username typed at the login form, an address typed into password reset. Memory growth driven by unauthenticated input rather than by how many contributors there are. | `RateLimiters::retain_recent`, swept every ten minutes. Forgetting a full bucket changes no decision — the next request rebuilds it full. | — |
| **K7** | **No graceful shutdown.** `axum::serve` ran without `with_graceful_shutdown`, so a deployment or `docker stop` dropped everything in flight. A worker that had just uploaded a completed batch lost it: the claim stays `claimed` until the heartbeat timeout, so its retry is answered `accepted: false`. | `SIGTERM` (what ECS sends before escalating at the stop timeout) and `SIGINT` stop accepting connections and let in-flight requests finish. | — |
| **K8** | **Lifting a ban wrote no audit row**, while applying one did. A ban applied and then quietly removed is exactly the sequence an audit log exists to make visible, and the ban row is gone by the time anyone looks. | `worker.unbanned`, in the same transaction as the delete, naming the identity rather than the ban row. PLAN.md's audit table gained it, and `user.signed_out_everywhere`, which the code already wrote and the table had never listed. | — |

### K.3 Discrepancies between code and PLAN.md

| # | What the code does | What PLAN.md said | Decision | Reasoning |
|---|---|---|---|---|
| K-D1 | Account deletion anonymizes the user row and leaves `task_claims` exactly where they are. | The shipped migration's `task_claims` comment — reproduced verbatim in PLAN.md's schema block — still described the pre-F8 sequence: update task counters, delete the task records, delete the claim rows, delete the user row. | **PLAN.md updated** (and the migration comment it is copied from) | The comment describes code that no longer exists, in the two places a reader would most trust it. F8 chose anonymization and gave the reasons; the comment now gives them where the foreign key is declared. No behaviour change. |
| K-D2 | `scripts/restore-roundtrip.sh` seeds the round-trip database with plain SQL. | Phase 4: the round trip "seeds with `fake_worker.py`". | **PLAN.md updated** | The code wins, and it is also right: what the round trip tests is `pg_dump`/`pg_restore`, and going through the worker API would add a client to the failure surface without adding a row shape. Also a Python-worker correction — see K.4. |
| K-D3 | The claim path takes a per-job advisory lock for every job type (K3), and leave generation refuses to reissue during a transition (K1). | The lock was described as leave-generation-only; the reissue rule said nothing about transitions. | **PLAN.md updated to match the code change** | These are this pass's own fixes, not pre-existing disagreements. Recorded here so the sections stay in step. |
| K-D4 | Opening-rack submissions are checked against the dispatched rack set (K4). | The submission section said only a *game* batch is checked against its task, and the impossibility table listed only the game rule. | **PLAN.md updated to match the code change** | As K-D3. PLAN.md's own framing — "the only submission-time check that catches a worker reporting work it did not do" — was what made the gap visible. |
| K-D5 | `GET /api/jobs/:id/results/stream` streams a job's entire result table to anyone, unauthenticated and unpaginated. | PLAN.md describes it as the offline-analysis download, with no access note. | **Decided: rate limit and cap it (A), and add an admin export for completed jobs (C)**; neither built yet | See K.6. Raised as unresolved by this pass and decided in review; the options and the implied shape are recorded there. |

**Count for this pass: 2 code-wins (PLAN.md updated), 0 plan-wins (code updated
for a discrepancy), 1 unresolved.** K-D3 and K-D4 are documentation following
this pass's own fixes rather than discrepancies that existed beforehand. The
running total across all three passes is in the table at the top of this file,
plus these.

### K.4 Python worker as a production client

Every reference was re-checked across `README.md`, `TESTING.md`, `RUNBOOK.md`,
`PLAN.md`, `docker-compose.yml`, `docker/Dockerfile`, `.env.example`,
`scripts/`, `infra/` and `worker/fake_worker.py` itself.

**One correction, K-D2 above**: PLAN.md said the restore round trip seeds with
`fake_worker.py`. It does not — it writes the rows directly in SQL — and the
sentence was the last place in the repository still giving `fake_worker.py` a
job outside tier 5. Everything else already says MAGPIE is the only production
client; nothing in the backend special-cases the fake worker.

Separately, `fake_worker.py` was checked against K4's new rule: it answers with
one analysis per rack in `request["racks"]`, so it satisfies the check, as does
MAGPIE's opening-rack executor.

### K.5 MAGPIE `birdtest-contribute`

Branch: **`birdtest-contribute`**, committed directly on it as instructed. One
commit, `cac07a8a`, on top of `62fb6f37`.

**The gap.** Opening racks are the one job type whose request carries `racks`
and a single `player` rather than a player pair — and **no contract fixture
covered it on either side**. `contract-fixtures/` had `assignment-games.json`
and `assignment-leave-generation.json` and nothing else, so birdtest's
`assignments_carry_a_task_request_this_build_understands` and MAGPIE's
`test_contract_fixtures_carry_every_key_contribute_reads` both skipped the
shape. Renaming `racks` or `player` on either side would have passed both test
suites and broken every opening-rack contributor — the same failure as the
`plies` / `top_plays` mismatch (C7) that motivated the fixtures in the first
place.

| Change | Where | Why birdtest needs it |
|---|---|---|
| `contract-fixtures/assignment-opening-rack.json` — a simming player, so the win% model the executor loads itself is covered too | both repositories | Closes the gap above. Verified by deliberately renaming `racks` in MAGPIE's copy: `test_contract_fixtures_carry_every_key_contribute_reads` fails with the name it could not find. |
| The fixture added to birdtest's round-trip test | `backend/src/routes/worker.rs` | Parses it into the real `TaskRequest` and compares field structure, so the server's half is pinned as well. |
| Opening-rack assertions in `test_contract_fixtures_carry_every_key_contribute_reads` | MAGPIE `test/contribute_test.c` | Asserts the request keys, a non-empty `racks`, and the full player key set — the executor applies the player exactly as the games executor applies `player1`. |

**Verified, no changes needed:** `magpie_test contribute` passes (dev build,
ASan/UBSan). The contract fixtures present in both repositories are
byte-identical. `contribute.c` reads everything birdtest sends and nothing it
does not — the shutdown directive, `worker_uuid`, `min_magpie_version`, the
`expected_data` files, and the artifact key (validated before use). Nothing in
this pass's birdtest changes touches the wire format: K1, K2, K3, K5–K8 are
internal to the server, and K4 tightens a check on a payload MAGPIE already
produces correctly.

### K.6 Worth deciding next

Each of these is a decision rather than a bug, so each is recorded with the
options and a recommendation rather than acted on. **K-D6 through K-D11 were
worked through after the pass's code changes were committed**, in review; where
that changed a conclusion the pass had already written down, the correction is
stated rather than quietly swapped, since the point of this file is to be
second-guessable.

Carried forward from section I: **I3** is still open, and K-D9 replaces its
suggested fix, which does not work.

#### K-D5. `GET /api/jobs/:id/results/stream` is public, unauthenticated and unbounded

It streams every row of a job's result table as newline-delimited JSON, straight
from a cursor. That is the point — it is the offline-analysis download, and the
streaming is what keeps the server's memory flat. But it is reachable by anyone,
it has no pagination or rate limit, and for a full English opening-rack job it is
tens of millions of rows. One caller can hold a database connection open for the
length of that scan, and *n* callers can hold *n* of them, which is a denial of
service that costs the attacker one HTTP request.

- **A. Rate limit it** per IP, like the auth endpoints, and cap concurrent streams.
  - *For:* keeps it public, which matches the "crowdsourced, open data" intent; small change.
  - *Against:* an attacker with a few addresses still occupies the pool.
- **B. Require an account.** Any signed-in user may stream; anonymous callers get the paginated endpoint.
  - *For:* attaches a cost and an identity to the expensive read.
  - *Against:* the data is meant to be open, and registration is a real barrier for a researcher who only wants the numbers.
- **C. Make it an admin-triggered export** to the artifact bucket, with a signed URL.
  - *For:* the read happens once per export rather than once per caller, and object storage is what serves large files.
  - *Against:* the most work, and the download is no longer live.
- **D. Keep it.** Document that it is an unmetered public endpoint.

**Recommendation: A now, C if the corpus becomes something people actually
download.** Left unresolved because it is an access-policy decision about how
open the data is meant to be, which is not the audit's to make.

**Decision: A and C — both, with C scoped to completed jobs.** Not
either/or: they cover different traffic. A keeps the live stream safe for
ad-hoc and small-job use, which is what it is good at. C is the sanctioned path
for pulling a completed job's whole corpus, which is where the tens-of-millions
-of-rows problem actually lives. Neither is implemented; the shape below is
what the decision implies, not what is built.

##### How A could be done

Two mechanisms, and the second is the load-bearing one.

1. **Rate limit per client IP.** `RateLimiters` gains an `export` bucket
   alongside `register`/`login`/`reset`; `job_results_stream` takes the
   `ClientIp` extractor and calls `ratelimit::check`, which already answers
   `429` with `Retry-After`. This bounds how often a stream *starts*.
2. **Cap concurrent streams**, which is the part that matters. A limit on
   starts does not bound long-lived streams: one request a minute, each running
   ten minutes, still accumulates. And the resource being consumed is not CPU
   but **the connection pool** — `db::connect` sets `max_connections(20)`, and
   `sqlx::query(…).fetch(&pool)` holds one connection for the whole life of the
   stream. A handful of concurrent streams starves dispatch and submission,
   which is how a cheap public read turns into an outage. So: an
   `Arc<Semaphore>` with a small permit count (2 or 3 against 20 connections),
   `try_acquire_owned`, `429` when exhausted, and the **owned permit moved into
   the `async_stream::stream!` body** so it is released when the stream ends
   *and* when a client disconnects and the response body is dropped.

The permit count and `max_connections` are one decision, not two, and belong
next to each other in config so neither can be tuned without the other.

##### How C could be done

The shape is already established by `input_data_imports`, which is the same
problem — a long operation an admin starts, polls, and then acts on:

- **`job_exports`**: `id`, `job_id`, `state` (`running`/`ready`/`failed`),
  `artifact_key`, `bytes`, `sha256`, `row_count`, `error`, `requested_by`,
  `requested_at`, `completed_at`. Startup fails any row left `running`, exactly
  as `inputdata::fail_orphaned_imports` does and for the same reason: single
  instance, so such a row belongs to a process that is gone.
- **`POST /api/admin/jobs/:id/export`** → `202` and an id, spawning a task;
  **`GET`** to poll, and to fetch the URL once ready. The task runs the same
  per-job-type query `job_results_stream` runs, gzips it, and uploads.
- **Refuse a job that is not `completed`.** That is the constraint that makes
  the whole thing worth building: a completed job's results are immutable, so
  the export is a stable artifact — built once, reused by every later request,
  and safe to cache. An export of an active job is stale as it is written.

**Two things `ArtifactStore` cannot do today**, which are the bulk of the work
and should be costed as such:

- `put` takes a `Vec<u8>` — the entire object in memory. A full English
  opening-rack export is on the order of gigabytes. It needs either a
  **multipart upload streamed from the cursor** (the bucket already carries an
  `abort-incomplete-uploads` lifecycle rule, so the infrastructure anticipates
  this) or a spill to task-local disk plus `ByteStream::from_path`, which is
  simpler but couples the export size to Fargate ephemeral storage.
- `get` returns a `Vec<u8>` as well, so serving the download *through* the
  backend has the same problem — and would put the bytes back on the connection
  pool that A exists to protect. **A presigned GET**
  (`get_object().presigned(…)`, available in the pinned `aws-sdk-s3 1.x`) is
  what keeps the bytes out of the backend entirely, and is why the original
  option said "signed URL".

**Invalidation.** "Immutable once completed" holds except for purge and
restore. Purge should delete a job's exports and their objects, for the same
reason it already deletes `leave_generation_transitions` — a stale row that
says "ready" is worse than no row. Recording `row_count` on the export means a
later mismatch is visible rather than silent, which is the same
signal-not-silence principle as the KLV `sha256`.

**Lifecycle, and a difference from the existing artifacts.** Exports are
derived data: regenerable from the database, so they need neither backup nor
cross-region replication, and they *should* expire. That contradicts
`infra/s3.tf`'s current comment — "Objects here are only ever added, never
deleted" — so an `exports/` prefix wants its own expiry rule, exclusion from
replication, and an amended comment saying why this prefix is different from
the KLVs.

**Both sub-questions are now decided.**

- **Who may download a ready export? Admin-only.** The export endpoints sit
  entirely under `/api/admin`, so C adds no public surface at all: no public
  signed-URL policy, and the URL's expiry can be short because the only holder
  is an authenticated admin who just asked for it.
- **Should the stream defer to the export? Yes.** For a completed job with a
  ready export, `job_results_stream` answers `303` to it rather than scanning.
  That is the whole point of building C: a completed job's corpus should be read
  from a stable artifact once, not re-scanned per caller.

**What follows, and one thing to confirm.** These two together mean the bulk
corpus of a completed-and-exported job is reachable only by an admin — a public
caller hitting the stream is deferred to a URL it cannot fetch. So the
deferral has to answer a non-admin with something honest (`404`, naming the
export as the route) rather than a redirect into a wall. That leaves an odd
shape: the public could bulk-stream an *active* job — expensive, unbounded,
still growing — but not a *completed* one, which is the cheap stable case. That
is backwards on cost.

The way to square it, and the **recommendation**, is to make the stream
**admin-only outright** and leave the public the paginated
`GET /api/jobs/:id/results`. Then one rule covers everything: bulk is an admin
operation, browsing is public. It also **simplifies A considerably** — with no
anonymous caller able to start a scan, the per-IP rate limit stops earning its
keep and only the concurrency cap remains, which is the half that was
load-bearing anyway (it protects the connection pool from an admin or a script
holding several streams open, which a rate limit never bounded).

That is a change to A, which was originally chosen partly to *keep* the stream
public, so it is flagged rather than assumed: **if the stream is meant to stay
publicly reachable for active jobs, say so and A keeps its rate limit.**

**Sequencing: A first.** It is small, and it is the half that stops one
request from costing the site. C is a feature, and can follow.

#### K-D6. `opening_rack_stats` scans the job's whole history

> **Resolved by K-D11, which deletes both expensive queries rather than fixing
> them.** The analysis below is kept because it is what established that the
> cost lived entirely in the two fields K-D11 removes, and because the index
> finding falls out of it. Option A — the query rewrite — is **not needed**.

**Concern.** Two of its three queries aggregate over every stored move row, on
the job detail page and on every SSE push. I2 replaced the distinct-rack `COUNT`
with `jobs.racks_analyzed` and left these:

```sql
-- average_best_equity
SELECT AVG(m.equity) FROM position_analysis_records r
JOIN tasks t ON t.id = r.task_id
LEFT JOIN position_analysis_moves m ON m.record_id = r.id AND m.rank = 1
WHERE t.job_id = $1

-- best_move_types                              (F16: 541 ms at 1M racks)
SELECT ... FROM position_analysis_moves m
JOIN tasks t ON t.id = m.task_id
WHERE t.job_id = $1 AND m.rank = 1 GROUP BY 1
```

**Correction to this pass's first write-up of it.** It said a running sum and
count beside `racks_analyzed` was the fix, and called that "the trade-off I2
already weighed". That was wrong about the cost, because the schema already
carries a cheaper fix. `position_analysis_moves.task_id` is denormalized with
the comment *"so job-wide aggregates need not join through the record"*, and
`position_analysis_moves_best_idx` is `ON (task_id) INCLUDE (move, equity)
WHERE rank = 1` — a covering partial index holding exactly one row per position,
with `equity` already in it. The second query uses it. **The first does not**,
because it joins on `record_id` rather than `task_id`, so it cannot reach the
index that was built for it. It reads like a query that predates the index.

**Options.**

- **A. Rewrite the average to go through `task_id`**, the same shape as the
  move-types query. The result is identical: the current `LEFT JOIN` yields
  `NULL` for a record with no rank-1 move and `AVG` ignores `NULL`s, so
  selecting from the moves directly covers the same set.
  - *For:* turns a million-row nested join into an index-only scan of the index
    added for it; no schema change, no counter, no staleness.
  - *Against:* none identified.
- **B. Running sum and count** beside `jobs.racks_analyzed`.
  - *For:* the average becomes constant-time.
  - *Against:* a third denormalized counter to keep correct through purge,
    restore and RUNBOOK §2.3 — and it fixes only one of the two scans, since a
    distribution over move types cannot be a scalar counter.
- **C. Debounce the SSE push and cache the detail payload** per job (F16's
  option B, never implemented). Superseded by K-D8, which is the same idea
  done properly.
- **D. Keep.**

**Recommendation: A, unconditionally — it is a query rewrite with no
identified downside — and then K-D8 for the general case.** Even after A the
page still does two index-only scans over the job's whole history: roughly a
second combined at a million racks, about 3.5 s projected for a full English
job, on every view and every push. A makes that tolerable; only K-D8 bounds it.
**B is withdrawn** in favour of K-D8, which is less machinery and covers more.

**One thing to settle while there:** both queries count claims rather than
racks, which is a correctness question rather than a cost one. It is **K-D10**.

#### K-D7. A locally-failed task's claim is left to time out

**Concern.** When a MAGPIE executor fails, `contribute_submit_result` stops the
heartbeat and drops the claim token without telling the server, so the task
stays `claimed` for the whole heartbeat timeout (300 s by default) before anyone
else can have it. `contribute_decline_task` would release it at once, but it
also calls `remember_unsupported`, which blacklists the entire job for the rest
of that run over what may be one bad task.

**Correction to this pass's first write-up of it.** It called separating those
two "the real fix". On reflection that is probably wrong, because of *why* an
executor fails:

- If the failure is a property of the **task or the job** — an unusable rack, a
  batch size of zero, leavegen producing nothing — the task is poison and will
  fail for every worker. Reissuing it in seconds rather than five minutes burns
  the fleet through it *faster*. The timeout is accidentally rate-limiting
  poison work.
- If it is a property of the **worker** — a failed KLV write, a transient OOM —
  MAGPIE's `MAX_CONSECUTIVE_FAILURES = 5` ends the run anyway, and the five
  minutes cost one task's latency on a job that has other work.

The case where fast reissue helps — a transient, worker-local failure on a job
with nothing else to hand out — is narrow, and the change carries a real cost in
the common case.

**The gap that does look real is observability.** `routes::worker::decline_task`
validates `reason` against `missing_data` / `magpie_version` /
`unknown_job_type` and then **never stores it**. `worker_data_gaps` records
*what* was missing; the audit row is a bare `task.declined` (`audit::log` has no
`reason` parameter — only `log_ban` and `log_detail` do). So nothing anywhere
records *why* claims are declined, and a local execution failure is invisible
server-side, indistinguishable from a worker that vanished.

**Options.**

- **A. Add a `task_failed` reason and decline without blacklisting.**
  - *For:* the task returns in seconds; the failure becomes visible.
  - *Against:* the poison-task churn above. It needs a reissue guard — stop
    re-dispatching a task after N declines — to be safe, and that guard is the
    larger half of the work. Additive protocol change across both repositories
    plus the fixtures.
- **B. Record the decline reason server-side** — on the audit row
  (`log_detail` already takes `reason`) or as a column on `task_claims` — and
  leave the timeout behaviour alone.
  - *For:* small, one-sided, no protocol change. Turns "is this happening at
    all?" into a query, and is the prerequisite for judging A on evidence
    instead of speculation.
  - *Against:* does not speed up reclamation.
- **C. Keep both as they are.**
- **D. Shorten `HEARTBEAT_TIMEOUT_SECONDS`.**
  - *For:* cheapens every lost claim, not just failed ones.
  - *Against:* a legitimately slow batch gets reclaimed and duplicated. It
    trades a rare cost for a common one; wrong lever.

**Recommendation: B now, and A only if B shows that local failures are common
and transient.** The five-minute delay is worth paying for the rate limiting it
provides, and there is currently no evidence either way — which is the part
actually worth fixing.

#### K-D8. Job statistics are display-only, and could be refreshed in the background

**What is load-bearing, checked rather than assumed.** `jobstats` has three
entry points outside itself:

| Call | Where | On what path |
|---|---|---|
| `jobstats::compute` | `public.rs` job detail, `public.rs` SSE connect, `worker.rs` SSE push (already gated on `has_subscribers`) | **Display only** |
| `jobstats::game_stats` | `worker.rs`, inside `finish_condition_met` | **Job completion** |
| `jobstats::load_job` | single-row reads | trivial |

`scheduler.rs` does not reference `jobstats` at all, and neither does
`registry::acquire`: the claim path reads `jobs`, `tasks`, `task_claims`, the
per-type config rows and `leave_rack_progress`, and nothing else. **No worker
anywhere waits on a statistic.** `opening_rack_stats`, `worker_contributions`
(F16: 136 ms at 44,000 claims, and growing), `estimate_eta`, `leave_gen_stats`
and the task-state counts all exist so a person can see how a job is doing.

The one exception is not dispatch but **stopping**: `game_stats` runs on every
submission to a games or game-pairs job and is what auto-completes it when the
LLR crosses. PLAN.md commits to that explicitly ("SPRT is evaluated inline on
every result submission (no background sweep)"), so backgrounding it would let a
job overshoot its stopping point by the refresh interval. F16 put it at about
50 ms for 400,000 units, so there is no pressure to move it and it should stay
where it is.

**Why it is not already this way** is history rather than a reason: the SSE
design came first, and PLAN.md's promise is "one SSE event per accepted result,
carrying the same payload `GET /api/jobs/:id` would return". Byte-identical live
and reload payloads is a property the code went out of its way to keep. A
background refresher gives that up — the stream becomes an event every *T*
rather than one per result. That is the real cost, and it is a product decision.

**Options.**

- **A. Refresh every active job on a fixed interval; all reads serve the cache.**
  - *For:* constant-time reads always; cost bounded and independent of traffic;
    no cold-view penalty.
  - *Against:* burns work on jobs nobody is looking at; every viewer sees stats
    lagging by the interval.
- **B. Refresh only jobs under attention** — a live SSE subscriber, or viewed
  recently — on an interval.
  - *For:* cost follows attention, which is the pattern `SseBroadcaster::has_subscribers`
    already establishes here; no waste on idle jobs; a watched job still updates
    continuously.
  - *Against:* the first view of a cold job pays full cost; one more piece of
    in-process state.
- **C. Lazy TTL cache, no background task.**
  - *For:* simplest; nothing to schedule.
  - *Against:* whoever misses the cache pays the full cost — the very thing this
    is meant to avoid — and a popular job stampedes on every expiry unless the
    computation is single-flighted.
- **D. Split the payload by freshness.** Progress and SPRT stay live (they are
  counters already); the descriptive aggregates — analysed racks, the move-type
  distribution, the worker table — come from the background and carry an
  `as_of`.
  - *For:* honest about which numbers are live; leaves the completion path
    untouched; the page can say "analyses as of 30s ago" rather than being
    quietly stale.
  - *Against:* two update paths in one payload, and the frontend has to render
    staleness.

**K-D11 shrinks the motivating case.** With `average_best_equity` and
`best_move_types` gone, `opening_rack_stats` becomes two single-row reads and
opening-rack jobs stop being the expensive ones. What still grows with a job's
history inside `jobstats::compute` is `worker_contributions` (F16: 136 ms at
44,000 claims), `estimate_eta`'s count over recent claims, the task-state counts
over every task of the job, and `leave_gen_stats`. None of those is near the
seconds the removed queries reached, so this is **no longer urgent** — but it is
still the right shape, and the argument for it (nothing in the claim path reads
a statistic) is unchanged.

**Recommendation: B, shaped like D** — refresh under attention, and mark the
backgrounded parts with an `as_of` so the dashboard is not silently stale.
`ratings::recompute_stale` is already a two-minute sweep in `main.rs`, so there
is both a place to put it and a precedent for how its failures should be
handled (K5: log and skip, never abort the sweep).

Two things to settle first:

- **Do K-D6's option A regardless.** A cache over a 3.5-second query still has a
  3.5-second cold path, and every option above has one. Making the underlying
  query an index-only scan makes all of them cheaper, and makes C viable at all.
- **This deepens the single-instance assumption.** An in-process cache is
  coherent only because `desired_count` is pinned to 1 (C14). PLAN.md already
  lists the import reaper, the rate limits and SSE as single-instance-dependent;
  this would make four, and it belongs on that list rather than being discovered
  during a future attempt to replicate.

#### K-D9. The leave-generation universe copy (supersedes I3's suggested fix)

**What it is.** When a generation closes, `copy_universe` inserts one row per
rack into the next generation — 3,199,724 rows for English, since F1 made the
tracked universe every full 7-tile rack. Three things need those rows
materialized: claim-time selection is a single indexed
`ORDER BY occurrence_count ASC LIMIT racks_per_task` and "a rack with no row
counts as 0" needs a known universe (F16: 47 ms); result folding is an `UPDATE`
rather than an upsert, so a rack with no row creates nothing, which is the only
thing validating rack strings from workers; and `generation_klv` requires every
rack to have a row, since a rack that never occurred still contributes mean 0 to
the weighted average.

**Why I3 cares.** F16 measured the copy at **56–66 s** on the loaded local
compose Postgres against about a second on a small dev database — a spread wide
enough that neither predicts the production `db_instance_class`.
`scripts/leave-gen-bench.sh` answers it and is safe against production
(everything is rolled back).

**What the problem actually is**, worst last:

1. It is on the critical path: claims for the job are refused while it runs, so
   every worker on that job idles.
2. It is a long write transaction — 3.2 M rows, a heap row plus two index
   entries each, roughly half a gigabyte of writes and WAL, in the same
   transaction as the artifact row. On RDS that is replication lag and an IOPS
   burst; on a burstable class it can drain the I/O credit balance and leave the
   whole database slow afterwards. That is the specific reason the instance class
   matters.
3. It is multiplied by `generation_count`: a ten-generation English job copies
   32 M rows over its life and retains them all.
4. It is all-or-nothing and retried. The copy is inside `close_generation`'s
   transaction, so if it cannot finish, `completed_at` never commits and the
   takeover path re-runs the **entire** transition 30 minutes later, re-deriving
   and re-uploading as well. An instance slow enough not to finish does not
   merely slow the job down — it wedges it in a retry loop.

**I3's suggested fix does not work.** I3 says to "treat a missing row as zero
occurrences, and select a generation's racks by anti-joining the previous
generation's rows instead of copying them". It saves nothing: a generation closes
only when *no* rack is below target, so every rack must reach the target, so
every rack ends up with a row regardless. Sparse storage only changes *when* rows
are created — trading one bulk `INSERT … SELECT` for 3.2 M lazy upserts spread
across the generation's submissions, which is more total work — while turning the
47 ms selection into an anti-join against the universe and giving up the
"no row means not a real rack" validation. Steady-state storage is identical.
**Withdrawn.**

**Options.**

- **A. Generate rather than copy.** `seed_generation` already builds a universe
  from `RackIndex` via `COPY`, is idempotent and is already tested; calling it
  for generation N+1 instead of `copy_universe` is close to a one-line change.
  - *For:* one code path instead of two implementations of the same invariant,
    derived from the pinned letter distribution rather than from prior rows.
  - *Against:* it is a **trade, not a strict improvement**. The SQL copy keeps
    all 3.2 M rows server-side, which is PLAN.md's stated rationale ("later
    generations copy it in SQL instead of re-sending it"); generating pays
    roughly 75 MB of client-to-server traffic per generation plus the CPU to
    unrank 3.2 M racks. The two numbers available (H3's 37 s to `COPY`-generate,
    F16's 56–66 s to SQL-copy) are from different machines in different states
    and do not settle it.
  - *Checked and rejected as an argument for A:* that copying could propagate a
    damaged universe forward. `klv::FullRackLeaves::add_rack` rejects unknown
    letters, wrong tile counts and over-drawn letters; `generation_klv` pins the
    total against `RackIndex::total()`; and the primary key forbids duplicates.
    Right count, all individually valid, all distinct is exactly the right set
    for full racks, so the check is complete, and it fires at the transition —
    which is when the copy happens anyway. A is a simplification, not a safety
    fix.
- **B. Move it off the critical path**: seed generation N+1's universe when
  generation N *opens* rather than when it closes.
  - *For:* same total work, done while workers are busy instead of while they
    are idle. Storage is retained either way, so nothing extra is held — it just
    exists earlier. The transition is then only the artifact row and
    `completed_at`: a small, fast transaction, which also removes problem 4.
  - *Against:* a generation's universe exists before anything can use it, which
    is mildly confusing to read.
- **C. Overlap it with the derivation.** Weaker than B but simpler: the next
  universe depends only on generation N's rack *strings*, fixed from the moment
  that generation was seeded, so the copy can run concurrently with the ~13 s
  derivation and the S3 upload.
- **D. Take it out of the transition transaction**, whenever it runs. It is
  `ON CONFLICT DO NOTHING`, so it is independently retryable, and a failed copy
  should not cost a re-derive and a re-upload.
- **E. Stop retaining every generation's rows.** The structural one. Retention
  exists because "Artifacts: back up, or rebuild?" chose rebuild; if the KLVs
  were backed up instead — they already live in a cross-region-replicated bucket
  — generation N's rows could be dropped once its artifact is committed and
  verified, giving 3.2 M rows in total rather than 3.2 M per generation.
  - *For:* fixes storage and vacuum pressure.
  - *Against:* does **not** fix the copy time, and reverses a documented
    decision.
- **F. Keep it in proportion.** The copy is tens of seconds; driving 3.2 M racks
  to `target_rack_count` is the job. None of the above changes that.

**Recommendation: B plus D now** — both are unconditional and need no
measurement, B because moving work off the critical path is right however fast
the work is, D because a failure should be independently retryable however often
it happens. **A is measurement-dependent**, so rather than guessing, extend
`scripts/leave-gen-bench.sh` to time generate-versus-copy on the same machine in
the same run; it currently times only the SQL copy. Then A answers itself, and
answers it for the production instance class rather than for a laptop — which is
a better use of I3's benchmark than running it and still having to guess.

#### K-D10. The opening-rack aggregates count claims, not racks

> **Resolved by K-D11.** The inconsistency was entirely between the two removed
> fields and `racks_analyzed`; with them gone, the only number left is already
> one-result-per-task. None of the four options below needs choosing. The
> analysis is kept because it is the reason the removal is safe rather than
> merely convenient — and because it records that opening racks are the one
> place the stored corpus is genuinely per-claim, which stays true.

**Concern.** `OpeningRackStats` carries three numbers, and at `redundancy > 1`
they do not share a denominator:

| Field | Counts | Source |
|---|---|---|
| `racks_analyzed` | **one result per task** | `jobs.racks_analyzed`, incremented by `registry::count_first_result` only on a task's first accepted result |
| `average_best_equity` | **every accepted claim** | `AVG(m.equity)` over all rank-1 moves |
| `best_move_types` | **every accepted claim** | `COUNT(*)` over all rank-1 moves, grouped |

Opening-rack records are keyed `(task_claim_id, rack)`, so each redundant claim
records its own analysis of the same rack — deliberately: the migration says
"redundant claims each record their own and can be compared", and PLAN.md says
the same. In-game captured positions are the opposite, keyed
`(task_id, game_index, turn_number)` with `ON CONFLICT DO NOTHING`, so only the
first claim's land. Opening racks are the one place the corpus is genuinely
per-claim.

**This is visible on the page, not just in the API.** The job detail view
renders all three in one `<dl>`: "Racks analyzed 1,000,000 / 3,199,724" beside
"Best move types: placement 1,842,000 · exchange 158,000" — raw counts, not
proportions. At `redundancy = 2` the move-type counts sum to twice the racks
stated next to them. Nothing is wrong at `redundancy = 1`, which is why this has
not been seen; redundancy is a per-job setting an admin can raise without
touching any of this code.

**Why this is not simply "make it consistent".** PLAN.md's rule is that every
aggregate treating results as observations reads one result per task, and its
stated reason is determinism: "games are seeded and deterministic, so the other
copies replay the same games, and counting them would multiply the evidence by
the redundancy." That premise does not hold here. PLAN.md also says
"opening-rack analysis by a simming player is non-deterministic by
construction, so honest repeat runs disagree" — so for a simming opening-rack
job the repeat analyses are real extra samples, not duplicates, and discarding
them throws away exactly what redundancy bought. For a static player they are
identical and it makes no difference either way.

**Options.**

- **A. One result per task**, matching `racks_analyzed` and every other
  aggregate: restrict both queries to the first accepted claim per task.
  - *For:* one rule across the whole system, easy to state and to keep. The
    numbers stop depending on a job's redundancy. Discards nothing from
    *storage* — the per-claim rows stay, and stay comparable.
  - *Against:* the headline average ignores the repeat simming samples. Picking
    "first" among genuinely different samples is arbitrary.
- **B. Average per rack, then over racks.** Mean the analyses of each rack, then
  mean those; count each rack's best-move type once.
  - *For:* redundancy-independent *and* uses every sample. The number then means
    what its label claims — the mean best equity of a rack — rather than of an
    analysis.
  - *Against:* it needs `rack`, which lives on `position_analysis_records` and
    **not** in the covering index `(task_id) INCLUDE (move, equity)`. So it
    either joins back to the records, giving up K-D6's option A speedup, or
    `rack` is added to that index's `INCLUDE` list — cheap, but a schema change
    that widens an index carrying one row per position. Move types also need a
    tie-break when a rack's claims disagree.
- **C. Keep per-claim and make the payload say so.** Add `analyses_count`
  beside `racks_analyzed` so both denominators are present, and render move
  types as proportions rather than bare counts.
  - *For:* no query change; keeps every sample; fixes the *visible* error, which
    is as much a display bug as a query one.
  - *Against:* two denominators in one panel is still something a reader has to
    hold, and the headline average still moves when an admin changes redundancy.
- **D. Defer** until an opening-rack job actually runs at `redundancy > 1`.
  - *For:* no work, and nothing is wrong today.
  - *Against:* it is silently wrong the first time one does, and the setting that
    triggers it is one field on the job creation form.

**Recommendation: A, plus C's display fix.** One rule across the system is worth
more than a marginally better estimator on a job type nobody has yet run
redundantly, and A costs nothing that B would later need — the per-claim rows
survive, so B remains available if those repeat samples ever get a consumer.
Render move types as proportions regardless, since bare counts beside a
different denominator is the part a reader actually misreads. **B is the right
answer if simming opening-rack jobs at redundancy > 1 become a real workload**,
and the index change it needs is small enough to make then rather than now.

Whichever is chosen, it should be stated in PLAN.md next to the one-result-per-
task rule, which currently reads as universal and is not.

#### K-D11. `OpeningRackStats` keeps only `racks_analyzed`

**Decision.** `average_best_equity` and `best_move_types` are removed from the
job stats payload. `racks_analyzed` and `racks_total` — the progress pair — are
the display the page actually needs; the other two were carrying most of the
cost and all of the ambiguity in this area.

**What this resolves outright.**

- **K-D6.** The two removed fields *are* the two full-history scans. What is
  left of `opening_rack_stats` is a read of `jobs.racks_analyzed` (a counter on
  the `jobs` row) and `total_racks` from the config row: two single-row reads,
  constant time, no index scan at any job size. The query rewrite K-D6
  recommended is not needed, and neither is a counter.
- **K-D10.** The denominator inconsistency was entirely between those two
  per-claim aggregates and the per-task `racks_analyzed`. With them gone the
  payload carries one number with one meaning, and the redundancy question does
  not arise. **The per-claim rows are untouched** — nothing is deleted from
  `position_analysis_records` or `position_analysis_moves`, so redundant
  analyses remain stored and comparable, which is what the per-claim key exists
  for. Only the aggregate over them goes.

**What this unlocks, which is the part worth noticing.**
`position_analysis_moves_best_idx` — `ON (task_id) INCLUDE (move, equity) WHERE
rank = 1`, added because "the dashboard's aggregates are all over best moves" —
has **exactly one reader**, and it is `best_move_types`. Checked: the other two
rank-1 queries (`jobstats`'s average, and the opening-rack listing in
`routes::public::job_results`) both join on `record_id`, so they use
`position_analysis_moves_record_idx (record_id, rank)` instead. So removing
these fields orphans that index, and dropping it takes maintenance work off
**every move insert** on a table that runs to tens of millions of rows, plus its
storage. That is a submission-path saving, not just a read-path one.

`position_analysis_moves.task_id` loses its stated reason at the same time — the
comment on it is "denormalized so job-wide aggregates need not join through the
record", and there will be no job-wide aggregates. It is *not* recommended for
removal: it is also a `REFERENCES tasks(id) ON DELETE CASCADE`, a second cascade
path, and a column is cheap. But the comment becomes wrong and should say what
the column is actually for now.

**What has to change with it.**

- `jobstats::OpeningRackStats` loses two fields; `MoveTypeCount` goes entirely,
  and with it the move-type classification (the `ILIKE '(exch%'` /
  `'(pass%'` logic and the comment explaining MAGPIE's rendering) becomes dead
  code rather than something to keep working.
- The job detail page's "Opening racks" panel drops two of its three cells,
  leaving the progress pair. This also removes the visibly wrong number K-D10
  found, rather than fixing it.
- PLAN.md's *"A partial index on rank 1 keeps the dashboard's aggregates
  cheap"* stops being true and should go with the index. The neighbouring
  reasoning — that the record does not duplicate the best move because it is
  the rank 1 row of `position_analysis_moves` — **still holds**: that row is
  still read per-result by the results listing and by the rack lookup, just
  never aggregated.
- K-D8's motivating example shrinks; see the note there.

**Cost of the decision**, stated so it is a choice rather than an oversight: the
dashboard stops showing what the analysed racks *say* — the average best equity
and how often the best opening is a placement, an exchange or a pass — and shows
only how far through the space the job is. That information is not lost, only
un-summarised: every ranked move is still stored, `GET /api/jobs/:id/results`
still returns the best move, score and equity per rack, `?rack=` still returns a
rack's full ranked list, and the admin export (K-D5) is the path for analysing
the corpus properly. Summarising three million racks in two numbers on a
progress page was arguably never where that analysis belonged.


### K.7 Verification (this pass)

- `cargo clippy --locked --all-targets -- -D warnings`: clean.
- `cargo test --locked`: **82 unit and contract tests** (3 ignored, as before)
  and **32 integration tests** against a real Postgres 16 — admin 4, auth 3,
  leave generation 11, worker 14. The four new tests were each run against the
  code *before* their fix and observed to fail in the way described, then
  against the code after and observed to pass.
- Frontend `npm run check`: 0 errors, 0 warnings.
- MAGPIE `birdtest-contribute`: `make magpie_test` builds (dev build, `-Werror`,
  ASan, UBSan) and `./bin/magpie_test contribute` passes. The new contract
  assertion was verified to fail on a deliberately renamed key.
- **Not run this pass:** Terraform `fmt`/`validate` (no Terraform binary on this
  machine; CI runs both and nothing here touched `infra/`), the Docker image
  builds, and `scripts/e2e_magpie.py` against a live stack. Nothing in this pass
  changes the wire format, the images or the infrastructure; the four behaviour
  changes that could affect a real run (K1–K4) are covered by integration tests
  driving the real router against a real database.
- **Migration checksum, again.** K-D1 edits a comment in `0001_initial.sql`
  in place, per the pre-release convention. A database built from the earlier
  `0001` — including any local compose volume — will refuse to migrate. No
  deployed database exists; reset locally (README, "After a schema change").
  This is the same situation J.5 recorded, now for one more edit.
