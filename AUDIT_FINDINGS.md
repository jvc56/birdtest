# birdtest audit — findings

**Date:** 2026-09-11.
**Branches:** birdtest `audit/birdtest-2026-09-11` (off `main` at `baa4094`). MAGPIE
`birdtest-contribute`, committed directly on that branch as instructed.

This is the record of every decision the audit made, complete enough to
second-guess each one without reading the diffs. Where this file and PLAN.md
disagree about what the code does, this file describes the audited code.

## How to read the decisions

Each discrepancy between the code and PLAN.md got exactly one of three
decisions:

| Decision | Meaning | Count |
|---|---|---|
| **PLAN.md updated** ("code wins") | The code's behaviour was right, or at least deliberate, and PLAN.md was a stale or inaccurate summary of it. | **21** |
| **Code updated** ("plan wins") | The code was wrong — a bug, or a clear mismatch with what the rest of the system needs — and PLAN.md described the intended behaviour. PLAN.md was also touched where its wording needed to follow the fix. | **16** |
| **Unresolved** | Reasonable arguments on both sides, or a real design decision. Both left as they were, apart from correcting factual status lines. **Needs human input.** | **5** |

Section B lists fixes that were not discrepancies (PLAN.md and code agreed and
were both wrong, or PLAN.md was silent), and section F the open questions.

## Verification

- **birdtest backend:**
  - `cargo test` passes: 78 unit and contract tests in the library.
  - 12 new integration and API tests pass (`backend/tests/`) against a real Postgres (`TEST_DATABASE_URL`), using the tier-2 harness TESTING.md specified, which is now built.
  - Each new test names the bug it catches.
- **Frontend:** `npm run check` passes (0 errors, 0 warnings).
- **MAGPIE:**
  - `make magpie magpie_test` (dev build: `-Werror`, ASan, UBSan) succeeds.
  - `magpie_test contribute`, `config`, `rl`, `rlfr` and `layout` pass.
- **End to end:** the audited backend ran against a throwaway database, with the real rebuilt `magpie contribute` (NWL23, data-20251004), via a scratch script, not committed.
  - **game_pairs:** 9 tasks accepted. The pentanomial agreed with the game counts. One anonymous identity was minted and appended to `contribute.txt` exactly once.
  - **opening_rack:** 2 tasks, 100 racks analysed and stored.
  - **Admin validation:** a zero-batch job was refused with a field error.
  - **leave_generation:** fails. See F1.
  - The backend logged no warnings or errors.
- **Not run:**
  - `terraform validate`: Terraform is not installed here. The infra edits are syntactically conventional HCL, but unvalidated.
  - The Docker image builds: the new CI job does them.
  - `cargo clippy`.

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
| A17 | Leave-gen rack selection does not exclude racks already out with a worker. | Silent; the code's own `NoWorkYet` doc comment claimed it did. | Documented as a trade-off in PLAN.md and in the code comment; the choice itself is F4. |
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

### A.3 Unresolved (needs human input)

| # | Code | PLAN.md | Why not decided |
|---|---|---|---|
| F1 | MAGPIE's `leavegen` forces, counts and reports **full 7-tile racks**, and derives leave values itself (`rack_list_write_to_klv`). | birdtest enumerates **leaves** (1–`max_leave_size` tiles; 914,624 for English at 6), forces leaves, stores per-leave counts, and writes those means straight into the KLV. | See F1 below. Every leave-generation task fails; picking a model is a design decision. |
| F2 | The Worker Client status table says the opening-rack executor reads per-ply `bingo_percentage`/`average_score` and the simulated ranking from `SimResults`. MAGPIE reports only `move`, `score` and `equity`. | As described. | A missing MAGPIE feature (reading `SimResults` per move), not a mismatch that can be fixed by renaming. Static-player jobs work; simming opening-rack jobs store no win%, utility or ply data. |
| F3 | Public endpoints (`/api/workers`, job stats `workers`, `/api/jobs/:id/results`) publish anonymous workers' full UUIDs. | "Contributions are tracked and displayed per UUID, shown under the label 'Anonymous'." | See F3 below. |
| F4 | Captured positions: with `num_plays_recorded` null, MAGPIE reports at most 10 moves. | "A config that does not set it keeps everything." | Small, but "everything" is unbounded per position. Choosing a cap is a product call. |
| F5 | Deleting a user cascades to captured in-game positions keyed to their claim, including ones other redundant claims deduplicated against. It also cannot subtract their leave-gen occurrences. | Deletion "rolls back every counter that user's claims contributed". | PLAN.md now states both limits. Fixing them means re-keying positions on redundancy or keeping per-claim leave records; an archive-versus-delete decision. |

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
17. **Configuration.** A malformed `HEARTBEAT_TIMEOUT_SECONDS`, `SESSION_TTL_SECONDS`, `SECURE_COOKIES` or `MIN_MAGPIE_VERSION` silently became the default; it now fails startup. Added `DATABASE_URL` assembly from `DB_*` parts (see F6).

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

Not changed, and flagged:
- `MAGPIE_VERSION` is `"0.0.0"` (F7).
- Leave generation still fails (F1).
- Opening-rack simulation statistics are not reported (F2).
- `leavegen`'s post-generation step writes KLV, CSV and report files into the data directory on every task, and calls `log_fatal` — killing the process — if the directory is unwritable. PLAN.md already assumes a writable `./data`; worth revisiting for the GUI use case.

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

### Unresolved — need a decision before launch

See F1, F6 and F7. F3 should be settled before public launch too.

---

## F. Open questions in detail

**F1 — Leave generation: racks or leaves?**
The first end-to-end run of `leave_generation` found three independent problems. Two are fixed (C10, C16). The third is structural:
- MAGPIE's `RackList` only accepts full 7-tile forced racks ("forced racks must all be full racks of 7 tiles, found: ?"), and `rack_list_get_rack_equity_json` reports full racks.
- birdtest's `seed_generation` enumerates leaves, `next_step` forces leaves, and `klv::build` maps each key straight to a leave value.

Patching over the error would upsert 7-tile rows outside the server's universe, so no generation would ever reach target and the KLV would be all zeros. Options:
- **(a)** birdtest tracks full racks: a 3.2M-row universe per generation, and a Rust port of `rack_list_write_to_klv`'s rack-to-leave derivation into `klv.rs`.
- **(b)** MAGPIE accepts forced leaves and reports per-leave aggregates, changing `leavegen` semantics.

Until decided, `leave_generation` jobs should not be activated; games, game pairs and opening racks are unaffected.

**F2 — Simulated opening-rack statistics.**
Worth doing before anyone runs a simming opening-rack job: the rows would be stored without win%, utility or ply data, and there is no way to backfill.

**F3 — Anonymous UUIDs are published.**
The UUID is an anonymous worker's only credential. Anyone who reads `/api/workers` can:
- claim as that worker;
- submit garbage under its name, or get it banned;
- farm its attribution.

The frontend shows only the first 8 characters, but the API returns the full value. The fix is clear in principle — publish a derived pseudonym (e.g. a truncated SHA-256) and keep real UUIDs to admin endpoints — but the choice of public identifier affects the leaderboard, the `?worker=` filter and the admin ban flow (which currently takes the UUID from the public list). Pre-launch, so nothing is exposed yet.

**F4 — Overlapping forced racks (throughput).**
Concurrent leave-generation claims can receive the same lowest-count racks: correct, but wasteful at scale. Tracking in-flight racks costs a table or an array scan per claim. Moot until F1 is resolved.

**F5** — see A.3.

**F6 — Database credentials versus RDS rotation.**
`rds.tf` uses `manage_master_user_password`, which rotates the master password (every 7 days by default). The service reads a hand-written `DATABASE_URL` from SSM, so **the backend, backup and restore-drill tasks lose database access within a week of deploy**. The audit added `DB_*` assembly to `config.rs` so the password can be injected from the managed secret. Wiring it is an infrastructure decision:
- inject `DB_PASSWORD` from the secret's `password` key and `DB_HOST` from Terraform;
- redeploy tasks on rotation, since open pool connections survive but new ones fail;
- teach `backup.sh`/`restore-drill.sh` the same parts;
- rewrite RUNBOOK §1's repointing step.

The alternatives (turn off rotation, or unmanaged passwords) trade security for simplicity.

**F7 — The first MAGPIE version number.**
`birdtest-contribute` reports `0.0.0`; the server's shipped floor is `0.0.1` (PLAN.md: "a placeholder… to be raised to that release's real number before launch"). A production deploy with defaults refuses every contributor. Needs:
- a MAGPIE release version for the contribution-capable build;
- `min_magpie_version` in Terraform set to match.

**Also noted, not changed:**
- **Session revocation.** PLAN.md documents that sessions survive a password reset.
- **Login timing enumeration.** PLAN.md documents that a login attempt's timing reveals whether an account exists.
- **Terraform unvalidated.** Validate with `terraform init && terraform validate` before applying.
- **Harness template database.** The integration harness leaves one template database (`birdtest_tpl_<hash>`) on the Postgres it runs against, by design, so later runs clone it instead of re-migrating.

## G. Clarifications against the audit brief

- The brief described a "priority-tier **weighted-random** task scheduler". Both PLAN.md and the code implement a **deterministic deficit-based** scheduler ("no randomness is involved", with a stated rationale), so nothing was changed.
- The brief described "**per-job** ELO ratings". Both implement **pool-scoped Bradley-Terry** ratings, siloed from job control flow. Per-job SPRT is the stopping rule. Nothing was changed.
- `is_admin` handling matches PLAN.md: re-read from the database on every request, settable by no endpoint, and enforced by the `AdminUser` extractor on every admin route (all state-changing admin routes also verify CSRF).
