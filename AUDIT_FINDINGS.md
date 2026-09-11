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
| **Unresolved at first** | Reasonable arguments on both sides, or a real design decision, left for a human. **All five are now decided and implemented** (A.3, section F). | **5** |

Section B lists fixes that were not discrepancies (PLAN.md and code agreed and
were both wrong, or PLAN.md was silent). Section F lists every question the
audit left open, the options offered, the option chosen, and what was
implemented. Section H lists what implementing those decisions turned up.

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
