# birdtest audit — findings, version 6

Branch: `audit/birdtest-2026-09-17-pass2`, off `main` at `a061fa3` (the merge of
PR #8, which brought the sixth to ninth audits and their decisions into `main`).
MAGPIE: `birdtest-contribute`, **changed by this audit** — section 9 — one
commit, `51cc3ffc`, on top of `d93dacaf`; committed locally and **not pushed**.
Date: 2026-09-17.

**This is the tenth audit.** It builds on [AUDIT_FINDINGS_5.md](AUDIT_FINDINGS_5.md)
(the ninth, branch `audit/birdtest-2026-09-17`), [AUDIT_FINDINGS_4.md](AUDIT_FINDINGS_4.md)
(the eighth, `audit/birdtest-2026-09-16-pass2`), [AUDIT_FINDINGS_3.md](AUDIT_FINDINGS_3.md)
(the seventh, `audit/birdtest-2026-09-16`), [AUDIT_FINDINGS_2.md](AUDIT_FINDINGS_2.md)
(the sixth, `audit/birdtest-2026-09-15`), [AUDIT_FINDINGS_1.md](AUDIT_FINDINGS_1.md)
(the fifth) and, through them, the four unnumbered records before those
(`AUDIT_FINDINGS.md`). Every audit branch is now merged: each of
`origin/audit/birdtest-*` is zero commits ahead of `main`. The highest numbered
file anywhere in the history was `AUDIT_FINDINGS_5.md`, so this is
`AUDIT_FINDINGS_6.md`; the earlier files are untouched. A branch named for
today's date already existed (the ninth audit's), so this one is `-pass2`. Where
an entry revisits a prior item it names it ("ninth audit, U2").

This file is the authoritative record of every code-versus-`PLAN.md` decision
made in this audit, and of the bugs, races, MAGPIE argument trace, critical-path
analysis, performance and storage findings behind them.

**Counts: 4 code-wins (PLAN.md or TESTING.md updated to match the code), 7
plan-wins or code changes with the plan brought level (K1–K7), 2 items left for
human input (U1, U2).**

The default bias is that the code wins and `PLAN.md` is brought level with it.
The code was changed only where it was wrong, or where the plan described the
behaviour the rest of the system needs.

**A note on the brief.** The audit brief describes the scheduler as a
"priority-tier weighted-random task scheduler". Neither `PLAN.md` nor the code
has been that since the eighth audit's follow-up (`2a6fe2c`, "Drop job
priority"): selection is a deterministic deficit order on
`(claims_issued - claims_baseline) / allocation`, with no priority and no
randomness, and PLAN.md says so in four places. That was a recorded human
decision, the plan and the code agree, and it was not re-litigated.

---

## The headline

Nine audits read what the server does while it is running. This one asked what
happens when it **stops** — which, for a single instance whose old task must be
gone before the new one starts, is every deployment. The answer was four
independent defects in three places that compound:

| | Where | What happened on a routine deploy |
|---|---|---|
| D1 | Terraform | Nothing served for **seven or eight minutes**, not "a few seconds": ECS waits out the target group's 300-second default deregistration delay before it even sends `SIGTERM`, and the new task then needs three health checks thirty seconds apart |
| D2 | Backend | With one dashboard open anywhere, `SIGTERM` was followed by nothing until the runtime's `SIGKILL` thirty seconds later: graceful shutdown waits for open responses, and an SSE stream never ends |
| D3 | MAGPIE | `contribute` retried a refused or `5xx` request for **31 seconds** and then **ended the run**. Every contributor that asked for a task inside the gap stopped contributing until a person restarted it; one that was *submitting* lost the finished task too |
| R1 | Backend | The gap was longer than the 300-second heartbeat timeout, so the first claim request after the restart **abandoned every claim in flight across the fleet** — the workers had been heartbeating to a server that was not there — and every result then being computed came back `accepted: false` |

None of it is visible to a test suite, a single-machine end-to-end run, or a
reading of any one repository. All four are fixed, and the combination was
exercised live with real MAGPIE workers through a real outage (section 13).

---

## 0. What the prior audits left open

| Prior item | Still true? | How it was checked |
|---|---|---|
| Ninth audit — "nothing since `main` is merged" | **Resolved.** PR #8 merged `audit/birdtest-2026-09-17` (and with it the sixth to ninth audits) at `a061fa3`; its CI run and the push run on `main` both passed (`gh run list`) | `git log main`, `gh run list` |
| Ninth audit — "after the merge, dispatch the nightly once" | **Now possible, not done by this audit.** The last nightly (10:30 UTC, before the merge) failed on the old `main`'s `BUILD=release`; the merged workflow builds `portable_release`. It runs on its own schedule (cron 06:00 UTC; GitHub started the last two around 10:30). This audit ran the same script natively instead (section 13) and did not dispatch a workflow on the owner's account | `gh run list`; `.github/workflows/nightly.yml` |
| Ninth audit, U1 (a) parity with the jobs being served — implemented in `c46ff73` | Holds. `join_at_parity`'s statement, both callers, RUNBOOK §2.3's copy and the test were re-read; the minority-fleet limit is recorded in PLAN.md as decided | Section 3, "checked and found sound" |
| Ninth audit, U2 (b) the selection sweep — implemented in `c46ff73`, the newest and least-read code in the repository | Sound: every interleaving of sweep, reissue, reclaim, merge, purge and the sweep→tail switch was re-derived (section 4). One consequence it did not follow through (**B3**), and one cost its record understates (**U1**) | Sections 3, 4, 11 |
| Ninth audit, U3 (b) the narrow selection index | Holds; migration and PLAN.md's schema block are byte-identical (`diff`) | — |
| Ninth audit, "to do after merge" 2 — pin the claim, decline and shutdown shapes in MAGPIE's tests | Still open, still not a gap; unchanged | Section 9 |
| Ninth audit, "smaller, noted" — sweep order, validation under the locks, `seed_generation`'s COPY | Unchanged | — |
| Seventh audit, U1 — lapsed claims of jobs nobody claims from | Stands. This audit's R1 adds a grace to the same reclamation and does not change who triggers it | Not re-litigated |
| Sixth audit — `tasks_claimed_idx` unused, left alone as tiny | Left alone again | — |
| Every K-item of the prior five records | Hold | Spot-checked while reading |

---

## 1. How this audit was run

- Read `PLAN.md`'s design, low-level, worker-client, API and schema sections in
  full; diffed its schema block mechanically against
  `backend/migrations/0001_initial.sql` (identical before and after); read the
  ninth audit's record in full, then `c46ff73` — the implementation of its three
  decisions, which no audit had read — line by line.
- Backend, read in full: `scheduler.rs`, `jobs/leave_gen.rs`, `jobs/registry.rs`,
  `jobs/mod.rs`, `jobs/dispatch.rs`, `jobs/handler.rs`, `jobs/plausibility.rs`,
  `routes/worker.rs`, `auth/mod.rs`, `sse.rs`, `state.rs`, `main.rs`,
  `magpie.rs`, the migration; and the parts of `routes/admin.rs` (import,
  activate, deactivate, complete, census, purge, delete), `routes/public.rs`
  (stream, leave feed), `exports.rs` (start, build, purge) and `jobstats.rs`
  that this audit's findings touch. `infra/ecs.tf`, both workflows, the compose
  file, `scripts/e2e_magpie.py` and `frontend/src/lib/sse.ts`.
- MAGPIE `birdtest-contribute` at `d93dacaf`: the three contribute executors,
  both resets, `config_fill_autoplay_args`, `config_fill_game_args`,
  `config_fill_sim_args`, `autoplay.c`'s use of `use_game_pairs` and of the
  play chooser, the whole `Config` struct, and — new to this audit —
  `src/util/http_client.c`, the heartbeat thread, `contribute_claim_task`,
  `contribute_submit_result` and `impl_contribute`'s loop, read for what they
  do when the server is not there.
- Backend: `cargo clippy --locked --all-targets -- -D warnings` clean; `cargo
  test --locked` against Postgres 16: **191 tests before, 196 after**, all
  passing. Frontend: `npm run check`, 0 errors, 0 warnings. Terraform:
  `terraform fmt -check` and `terraform validate` (1.9.8, the version CI pins,
  run from the stock `hashicorp/terraform` image).
- MAGPIE: `make magpie_test` (the dev build: `-Werror`, ASan, UBSan, LSan) and
  `./bin/magpie_test contribute` pass; `python3 format.py` reports no
  difference; `find_circ_deps.py` reports the same seven cycles before and
  after. Per a standing instruction for this machine, `cppcheck`/`clang-tidy`
  were not run, and every build was capped (`-j 2`, `nice`).
- End to end: `scripts/e2e_magpie.py`, unmodified, natively, with the changed
  client; then two live outage runs with real workers (section 13). No Docker
  image was built.

---

## 2. MAGPIE arguments that change a task's outcome

The prior tables were not reused. The trace was re-run from both ends against
`d93dacaf`. Everything the prior five records list still holds. Checked this
time and not named before:

| Setting | Changes results? | How it is covered | Change |
|---|---|---|---|
| `use_game_pairs` on a leave-generation task | It could — `config_contribute_games` sets it per task and `config_contribute_leave_gen` does not reset it, so a `game_pairs` task (or a contributor's `-gp true`) leaks into the next leave task's `AutoplayArgs` | Inert there: `autoplay_leave_gen` always passes a NULL second game runner, `show_divergent_results` is forced false in leavegen mode, and the option check only refuses pairing with recorders a contribute leave task never requests. `autoplay.c` has exactly three reads of the flag and all three were read | None |
| `max_num_display_plays`, `shplies` on an opening-rack task | No | The opening-rack executor does not set them (the games executor does), and `config_fill_sim_args` passes them into `SimArgs` — where they reach only printing (`random_variable.c`, `analyze.c`, `autoplay.c`'s board print). What is *reported* is capped by arguments the executor passes to the writer explicitly, from the request | None |
| Endgame and pre-endgame solvers inside autoplay | Would | Reached only through a player's PlayChooser (`game_runner_create_play_choosers`), which requires `pN_play_chooser_time_ms >= 0`; both are reset to `-1` before every task | None |
| `num_threads` | For a simming player and for leave generation, yes — a documented, decided exception (fourth audit, U3/U4): simming jobs are excluded from equality cross-checks and leave generation runs at redundancy 1 | `contribute.txt` `threads`, by design the contributor's | None |
| `config->game_history` on an opening-rack task | Would, through inference | `config_contribute_use_player_settings_for_analysis` forces `sim_with_inference` off | None |

**No gap found**, for the sixth consecutive record. The server half is pinned by
the contract tests in both repositories, and this audit's end-to-end run put
every request shape through `config_contribute_validate_common` and the
required-key checks again.

One thing the trace did turn up is not an argument but a document: PLAN.md's
example requests showed a static player with `num_plies`, `num_plays` and
`num_plays_recorded` **null** — a request MAGPIE refuses — and omitted
`bingo_bonus` and `sim_cutoff`. K8.

---

## 3. Bugs and footguns

### B1 — one impossible count could stop a leave generation from ever closing

*Code changed; PLAN.md updated (K6). Older than staging; made permanent by it.*

- **What the code did.** `plausibility::check_rack_occurrences` holds a leave
  result's counts to **at least 1** and nothing held them from above. Accepted
  occurrences are *summed* — into `bigint` columns, by `merge_staged`, which
  folds every staged result of the generation in **one statement**
  (`SUM(count)::bigint`, then `occurrence_count + count`).
- **Why it matters.** The plausibility rules exist for the broken client, whose
  signature is garbage rather than bias, and a count out of an uninitialised
  buffer is as likely to be near 2^63 as anywhere. Such a result was accepted
  and staged. From then on the merge statement failed with `bigint out of
  range` — and a failed merge leaves the row staged, so it failed again: the
  half-hourly sweep, the merge a claim asks for, and the *drain*
  `run_transition` will not close a generation without. `next_step` answered
  `NeedsMerge` for ever. The job was wedged until someone deleted the staged row
  by hand, and nothing said why beyond a log line per attempt. The quieter
  version is as permanent: a rack reported a million times is at target for
  good on coverage nobody played, and PLAN.md is explicit that a fold "cannot be
  subtracted back out".
- **Fix.** `plausibility::check_rack_occurrence_total`, run from
  `registry::store_result` beside the two other rules that need the task: the
  submission's total is held to `num_games` × 1,000. MAGPIE's leave generation
  records at most two racks a turn (the player's, and a forced rare rack —
  `autoplay.c`, the two `rack_list_add_rack` calls), so at the 400-turn ceiling
  captured positions are already held to, no game reaches it; a real game is
  some twenty turns. The sum saturates rather than wrapping. It is an
  *impossibility*, which is the only kind of rule PLAN.md allows here.
- **Tests.** `plausibility::tests::a_leave_batch_cannot_report_more_occurrences_than_its_games_drew`;
  `leave_gen::a_count_no_game_could_produce_is_refused_before_it_can_wedge_the_merge`,
  which also stages the same number by hand and shows `merge_staged` fail twice
  running — the wedge, demonstrated rather than inferred.
- **Not done.** Making the merge itself survive a poisoned row already staged
  (skip it, quarantine it). With the door closed nothing can stage one, and a
  merge that silently drops results is its own kind of wrong.

### B2 — every deployment left open dashboards dead, and looking live

*Code changed (frontend).*

- **What the code did.** `lib/sse.ts` relied on `EventSource`'s own reconnect,
  with a comment saying so. `EventSource` reconnects after a *dropped
  connection*; a reconnect answered with anything but `200` **fails the
  connection for good** (`readyState` `CLOSED`), by specification. A deployment
  produces exactly that: the stream drops, the browser reconnects three seconds
  later, and the load balancer answers `503` until the new task is in service.
- **Effect.** Every job page open across a deploy stopped updating and gave no
  sign of it. Not a data problem; a page that says something untrue.
- **Fix.** On `error` with `readyState === CLOSED`, subscribe afresh after five
  seconds (one timer at a time, cancelled by the unsubscribe function). The new
  stream's first event is the current stats, so the page is level at once.
  `npm run check`: 0 errors.

### B3 — a claim that asked for a merge threw away what its sweep had just learned

*Code changed. From `c46ff73`.*

- **What the code did.** `c46ff73` made a claim answering `NoWork` **commit**,
  for one reason: a sweep that finds its lap finished deletes the cursor, and
  rolled back, every later claim re-read from the cursor to the end of the
  universe to find that out again. But the same sweep, one line further on, can
  find the lap's results *staged* and return `NeedsMerge` — and that arm still
  rolled back, under a comment reading "Nothing was written".
- **Effect.** Small, and exactly the cost the commit exists to avoid: for the up
  to a minute a full-size merge takes, every claim on the job repeated the
  end-of-universe read inside the dispatch lock. Late in a generation, with
  most racks at target, that read is most of the universe.
- **Fix.** `NeedsLeaveMerge` commits too. The only possible write is that
  delete, which is knowledge, not a decision.

### B4 — checked and found sound (no change)

- `join_at_parity` with `served_within`: the `COALESCE` fallbacks (served →
  offering → zero), the job's exclusion of itself, a 0% activation, both
  callers inside their transactions, RUNBOOK §2.3's copy.
- The sweep and the lowest-count-first tail; the switch between them; lap
  start, lap end and generation close: section 4.
- The leave feed's two-part cursor; a foreign cursor reads as "start at the
  beginning"; the database's collation orders `rack >` and `ORDER BY rack`
  identically, and `''` sorts first under any collation.
- A purge of a running export: the purge deletes the `running` row, so the
  export's guarded terminal `UPDATE` matches nothing and no `ready` row
  describes a purged job.
- `Magpie::run`: a timeout and `kill_on_drop` on every subprocess.
- The template and derived-file caches: bounded by jobs per process lifetime,
  forgotten on delete.
- Python worker: section 8a.

---

## 4. Race conditions

### R1 — a restarted server abandoned every claim in flight

*A bug under objective 2: **fixed** (K5). Older than every audit.*

- **The assumption that does not hold.** A claim is reclaimed when
  `COALESCE(last_heartbeat_at, claimed_at)` is older than the heartbeat timeout
  — "no heartbeat for that long means the worker is gone". That is only true of
  a server that was there to receive one. After an outage longer than the
  timeout, *every* open claim in the fleet is that old, however alive its
  worker: the workers went on heartbeating, to nothing.
- **The interleaving.** Server returns → the first claim request from anyone
  runs `reclaim_expired_for` over the candidate jobs → one statement flips every
  in-flight claim to `abandoned` and returns every task to `available` → those
  tasks are handed out again → each original worker's heartbeat now matches
  nothing (`state = 'claimed'` fails) and its finished result is answered
  `accepted: false`. Hours of fleet compute, discarded by the first request
  after a restart, with every row looking correct.
- **How often.** On every deploy before D1 was fixed (the gap was 7–8 minutes
  against a 5-minute timeout), and after it on anything longer than five
  minutes: a failed deploy rolled back, an RDS maintenance window, RUNBOOK §1's
  full restore, which scales the service to zero for "under an hour".
- **Fix.** `scheduler::reclaim_lapsed`: a process reclaims nothing until it has
  been up for the heartbeat timeout itself (`AppState.reclaim_from`, set at
  startup). A live worker heartbeats every thirty seconds and refreshes its
  claim inside the first minute; a claim still silent a full timeout after
  startup has had the same chance to speak as any other and is reclaimed as
  before. Both callers go through it — the claim path and `exports::start`.
- **Cost, stated.** A worker that really died during the outage is noticed up
  to one timeout later than it might have been. A crash-looping server never
  reclaims; it is not dispatching either.
- **Why this does not open a new race.** The grace only ever *delays* a
  reclamation; every path that depends on a claim being gone — a generation's
  close, an export's "settled", the finish check's "nothing in flight" —
  already treats `claimed` as in flight and waits.
- **Tests.** `worker_api::a_restarted_server_does_not_abandon_claims_it_could_not_have_heard_from`
  (an hour-old claim survives a fresh process's first claim request, is heard
  from, and its result is accepted);
  `worker_api::a_claim_still_silent_after_the_grace_is_reclaimed`. Live: section 13,
  where a claim 96 seconds old against a 70-second timeout, with **no**
  heartbeat ever received, was not reclaimed by a claim request one second
  after the restart, and its result was accepted.

### R2 — a claim could lapse while its own result was on the way

*Fixed, in MAGPIE (K3). PLAN.md already described the right order.*

- **The race.** `contribute_submit_result` called `heartbeat_stop` *first* and
  submitted second. From that instant the claim's clock was running with
  nothing to reset it, and a submission is not instant: the server does not look
  the claim up until the whole body has arrived, a batch with captured positions
  is tens of megabytes on a contributor's uplink (64 MiB at 1 Mbit/s is over
  eight minutes, against a five-minute timeout), and with D3's fix a submission
  is retried for a quarter of an hour. Any other worker's claim request in that
  window reclaimed the claim; the task was handed out again and the finished
  result answered `accepted: false`.
- **What PLAN.md says.** The contribute loop, steps 7 and 8: "`POST
  /api/worker/result`", then "Stop the heartbeat". The code had them the other
  way round.
- **Fix.** The heartbeat runs through the submission and stops after it
  (`contribute.c`). An executor that produced no result stops at once, as
  before. A heartbeat for a claim that has just completed is a no-op on the
  server (`state = 'claimed'` fails). The heartbeat and the submission already
  shared the client concurrently with artifact fetches; each request has its own
  libcurl easy handle, and `load_curl` has run on the main thread before any
  heartbeat thread exists.

### Examined and found sound

The sweep (`c46ff73`) is new state on the path that has needed the most race
fixes, so each of these was derived, not assumed:

- **Reissue vs lap start.** A lapsed task is reissued under the dispatch lock
  before anything is selected, so its racks stay behind the cursor. At
  redundancy 1 (enforced for leave jobs) an `available` task is always takeable
  by the present claimant, and under the dispatch lock nothing else holds a
  task row of the job, so `SKIP LOCKED` does not skip one.
- **Reclaim between two statements of one claim.** READ COMMITTED gives each
  statement its own snapshot, so a reclaim committing between `next_available`
  and `claims_in_flight` yields "not available, and not in flight": a lap can
  start with one lapsed task unissued, which the next claim reissues. The cost
  is that task's racks forced twice — duplicate coverage on different seeds,
  never a closed generation missing a result, because closing reads in flight
  and staged afresh and an `available` task is neither. Pre-existing in the
  tail selection too; harmless; left.
- **Merge in flight vs lap start.** `merge_staged`'s `DELETE … RETURNING` is
  invisible until it commits with the `UPDATE` it feeds, so `anything_staged`
  sees the rows until the counts hold them. There is no moment at which a
  result is neither staged nor counted.
- **Sweep → tail.** `racks_total - racks_at_target` only falls within a
  generation (counts only grow), so the switch happens once; the tail excludes
  everything out, including racks *ahead* of an abandoned cursor; `close_generation`
  deletes the cursor.
- **Rollback paths.** `issue_claim` refusing (job no longer active), a unique
  violation on a drawn seed or on the per-identity index: each rolls the cursor
  advance back with the task it belonged to.
- **Lock timeout then commit.** A claim whose dispatch-lock wait timed out is in
  an aborted transaction; `COMMIT` of one is a rollback and sqlx reports
  success. Correct as commented.
- **Heartbeat vs reclaim**, re-derived because R1 touches it: whichever
  `UPDATE` commits second re-evaluates its predicate against the new row
  version and does nothing.
- **Activation vs purge vs claims.** `join_at_parity` reads the other jobs
  without locks, by design; an approximation of a ratio that is itself moving.
- Everything the prior five records examined — claim vs reclaim vs decline vs
  heartbeat vs submit, redundant submissions serializing on the task row,
  purge's lock order (merge → dispatch → claims → job), the key limit, the fit
  lock — was re-read where this audit's changes came near it, and holds.

---

## 5. Critical path

Traced statement by statement again. **Claim:** identity (one statement) →
rate limit (memory) → body parse → `candidate_jobs` → reclaim (one statement,
now skipped during the grace) → per candidate: derived and template caches
(memory) → `BEGIN`, lock timeout, dispatch lock → re-dispatch probe → seed
cursor or leave selection → task, request and claim inserts → task update →
guarded job update → `COMMIT`. **Submit:** identity → claim `FOR UPDATE` → task
`FOR UPDATE` → job → template (memory) → decode and validate on the blocking
pool → record → claim, task, job and identity counters → `COMMIT` → finish check
(every eighth, or when nothing is in flight) → spawned SSE push.

Every statement on both paths is one the decision needs, as the prior audits
found; nothing display-only is left on either. **Nothing was moved this time**,
and that is the finding.

### What changed on it

| Change | Path | Why it was safe |
|---|---|---|
| **C1 — `NeedsLeaveMerge` commits** (B3) | A leave job's claims between a lap's end and its merge landing no longer repeat an end-of-universe read inside the dispatch lock | The only possible write is the cursor delete, which is true whether or not the merge has landed |
| **C2 — the new occurrence bound** (B1) | One pass over a list already in memory, on the blocking-pool side's output; no statement | It decides acceptance, so it belongs on the path |
| **C3 — MAGPIE's heartbeat is one attempt** (D3) | Client side: a heartbeat backing off through an outage held `heartbeat_stop`, and so the task's submission, for as long as it retried | Its own thirty-second schedule is the retry |

### Examined and left on the path

| Kept | Why |
|---|---|
| The finish check inline, every eighth submission (~50 ms at 400,000 units) | Decided by the fourth audit; spawning it would let the submitting worker's next claim race the completion it just caused. Milliseconds amortised, out of this pass's scope |
| The reclaim statement on every claim request | It is bounded by claims in flight across the fleet, not by history; throttling it to once every few seconds is a real option worth a millisecond or two per claim at a thousand workers — marginal, so noted rather than built |
| Everything the prior records list | Decided |

---

## 6. Performance — most severe first

1. **Every deployment cost the fleet its work in flight and most of its
   members.** *Fixed (D1, D3, R1, R2).* Expected impact before: 7–8 minutes with
   nothing serving per deploy; every task in flight at the time — each minutes
   to hours of contributed compute — abandoned and redone; every contributor
   that made a request inside the gap gone until restarted by hand, which for an
   unattended volunteer fleet means days. After: a gap of a minute or two
   (Fargate's provisioning, which Terraform cannot shorten), ridden out by
   every worker, with no claim lost. Measured live on a 90-second outage
   (section 13): both workers exit 0, both results accepted, zero abandoned.
2. **`SIGTERM` to exit took thirty seconds whenever a dashboard was open.**
   *Fixed (D2).* Thirty seconds of every deploy's gap spent waiting for a
   `SIGKILL`. After: the stream ends on the signal; the process exits when the
   last real request does.
3. **A generation near its end re-read most of the universe on every claim
   while its merge ran.** *Fixed (B3/C1).* Up to a second a claim, inside the
   dispatch lock, for up to a minute per lap end; and only late in a
   generation.
4. **A lap's end idles a leave job for a dead worker's full timeout plus a
   replay.** *Flagged, U1.* The ninth audit's record costs the pause at "one
   task's duration and one merge per lap". That is the live case. When the
   worker holding one of a lap's last tasks dies, the job hands out nothing for
   the heartbeat timeout (5 minutes) plus a full task's replay by whoever is
   reissued it — for every worker on the job, some 6,400 tasks apart.
5. **Re-dispatch at redundancy above 1 walks every task the claimant has
   already filled.** *Noted, not built; no job runs above redundancy 1 today.*
   `next_available` takes the oldest `available` task the identity holds no slot
   on. With workers of unequal speed the fast one's completed-once tasks pile up
   waiting for the slow one, and every claim by the fast worker probes all of
   them first — linear in the backlog, inside the dispatch lock, and the
   backlog grows without bound because generation never waits for redundancy to
   catch up. PLAN.md calls redundancy "the natural next step"; whoever takes it
   will need a bound on how far generation may run ahead of acceptance.
6. **`GET /api/admin/fleet`, the evidence sweep, `worker_contributions`** —
   unchanged, decided by prior audits, on the display pool or a background
   sweep.

---

## 7. Storage

Re-read against the schema as merged. **Nothing changed and nothing new was
found**: this audit adds no table, column or index. `c46ff73`'s additions are
bounded — `leave_selection_cursors` is a row per generation with a lap under
way, deleted at lap end, generation close and purge; `jobs.last_claimed_at` is
eight bytes a job. The narrow `leave_rack_progress_pick_idx` is as the ninth
audit measured it.

B1 is, among other things, a storage-integrity fix: an unbounded count written
into `leave_rack_staging` was a row that could not be merged and could not be
removed except by hand.

Unchanged from prior records, and still the human's to decide:
`leave_rack_progress` kept for the life of the job (432 MB a generation);
captured CGPs as `TEXT`; `audit_log` and `worker_data_gaps` growing with
declines; `tasks_claimed_idx` unused; no index on `audit_log`'s filters or
`task_claims.claimed_at`; what a merge rewrites ("What a merge costs").

---

## 8. PLAN.md reconciliation

"Code wins" means PLAN.md (or TESTING.md) was updated to match the code. "Plan
wins" means the code was changed (and the document updated wherever its wording
also needed it).

| # | Subject | Code | PLAN.md said | Decision | Reasoning |
|---|---|---|---|---|---|
| K1 | What a deployment costs | `infra/ecs.tf` took the target groups' defaults: 300 s of draining before `SIGTERM`, then 3 × 30 s of health checks | "Stopping first costs a few seconds with nothing serving" (Decisions settled, 4), and the same sentence in `ecs.tf` | **Plan wins in intent, code changed**: 30 s draining, two checks ten seconds apart. **PLAN.md corrected too** — "a minute or two", and what rides it out | D1. The plan's figure was never true; the fix gets as close as Fargate allows and the document now says the real number |
| K2 | MAGPIE's retry budget | Five retries, 31 s, then the run ends | The same, in the retry table — and, of deploys, "Worker claims retry" (`ecs.tf`) | **Code changed (MAGPIE); PLAN.md's table and a new paragraph updated** | D3. The documented number was the defect: it cannot coexist with a stop-then-start deployment |
| K3 | When the heartbeat stops | Before the submission | Loop steps 7–8: submit, then stop the heartbeat | **Plan wins, code changed (MAGPIE)**; PLAN.md now says why the order matters | R2 |
| K4 | Graceful shutdown | Waited for every open response, SSE streams included | "stop it accepting new connections and let in-flight requests finish" — silent about requests that never do | **Code changed; PLAN.md's Health and startup says how streams end** | D2 |
| K5 | When a claim lapses | Any time its last heartbeat is older than the timeout | The same (Task States; Request Handling step 3) | **Code changed; both passages updated** | R1. Not a contradiction — an assumption both shared |
| K6 | What a leave submission must satisfy | Counts ≥ 1 | The plausibility table and "What a submission has to satisfy" list the same | **Code changed; table row, the "three of them" paragraph and the contract's validation list added** | B1 |
| K7 | An `EventSource` after a deploy | Left to the browser, which gives up on a `503` | "the dashboard's stream reconnects" (`ecs.tf`) | **Code changed (frontend)** | B2 |
| K8 | The example requests in the Worker API Contract | Every player states `num_plies`, `num_plays`, `num_plies_recorded`, `num_plays_recorded`, `movegen_margin`; every request states `bingo_bonus` (and `sim_cutoff` where it can simulate); forced racks and reported racks are full 7-tile racks | A static player with `num_plies`, `num_plays`, `num_plays_recorded` **null**; no `bingo_bonus` or `sim_cutoff` anywhere; `forced_racks: ["AA", "AB"]`; a leave result of `"rack": "AA"` | **Code wins** | Stale since the sixth audit materialised defaults and the leave jobs moved to full racks. As written the examples are requests MAGPIE refuses and a result the server refuses. The fixtures were right throughout; the examples now say they are the authority |
| K9 | `recorder_type` in the player table | `best` \| `equity` \| `all`; an opening-rack job keeping more than one move refuses `best` | "`best` for all birdtest jobs" | **Code wins** | Contradicted by PLAN.md's own "How much of an analysis is kept" |
| K10 | TESTING.md, plausibility coverage | Thirteen rules | "Twelve rules are covered" | **Code wins** (after B1) | — |
| K11 | TESTING.md, the test lists | `I-SCHED-20`, `A-PUBLIC-6a`, `I-LEAVE-17` exist | Absent | **Code wins** | New tests recorded where the tiers list theirs |

Counted: K8–K11 are code-wins (4). K1–K7 changed code (7). U1–U2 are
unresolved (2).

**Also updated, not discrepancies:** RUNBOOK §1 gained a paragraph on what the
fleet does during a restore-length outage — which is the case R1's grace and
D3's budget were built for, and the one place an operator deliberately causes
it.

**Checked and found in agreement:** the schema block (byte-identical); the
Workflow and Request Handling sections against `candidate_jobs`,
`join_at_parity`, `claim` and `try_claim_from_job`; the leave-generation claim
steps against `next_step`, `sweep`, `furthest_below_target` and
`nothing_to_hand_out`; Result Submission against `submit_result` and
`after_submission`; the Worker and Admin route tables against the routers; the
configuration table against `config.rs`; the contract fixtures in both
repositories (identical).

---

## 8a. The Python worker

Looked for, in every document, the compose file, the Dockerfile, the scripts,
the Terraform, CI, the frontend and the backend: any description or treatment of
`worker/fake_worker.py` as a production client. **None found; nothing to
correct** — the same result as the prior five records. The script's docstring
opens "Test tooling only. The one production client is MAGPIE itself"; the
compose service is behind a `fake-worker` profile marked end-to-end-suite only;
the Dockerfile target says it needs no MAGPIE because it invents its results;
RUNBOOK says "Never use `worker/fake_worker.py` for this"; README and TESTING.md
confine it to tier 5; PLAN.md's Worker Client section says MAGPIE is the only
production client there is; the landing page tells contributors they need "only
MAGPIE — no Python, no Docker".

One thing this audit's B1 touches: the fake worker's leave results report 1–12
occurrences a rack, far inside the new bound.

---

## 9. MAGPIE `birdtest-contribute`

Checked directly rather than assumed — and this time **something was missing**.
The branch held everything birdtest needs to *run a task*. It did not hold what
a production client needs to stay a client across the server's ordinary life.

### Changed on `birdtest-contribute` (and on no other branch)

| # | What was missing | Change | Why it was needed |
|---|---|---|---|
| M1 | A retry budget that outlasts a deployment | `src/util/http_client.c`: a transport failure or `5xx` is retried `HTTP_CLIENT_MAX_TRANSIENT_RETRIES` (20) times, backing off 1, 2, 4, … seconds to a ceiling of `HTTP_CLIENT_MAX_BACKOFF_SECONDS` (60) — about fifteen minutes — where it was five retries and 31 seconds. The constants live in `src/def/contribute_defs.h` (AGENTS.md: shared constants go in `src/def/`); `http_client_backoff_seconds` is exposed for the test | D3. Giving up ends the run. A stop-then-start deployment is a minute or two of refused connections and `503`s, so every deploy stopped whichever contributors asked for anything during it, and lost a submitter's finished task |
| M2 | A heartbeat that does not hold the task up | `http_client_post_json_once` (no retry of a transient failure), used by the heartbeat thread | With M1 alone a heartbeat would back off for up to fifteen minutes, and `heartbeat_stop` joins the thread — so the task's submission would wait behind it. Thirty seconds on, the next heartbeat is the retry |
| M3 | A claim kept alive until its result has landed | `contribute_submit_result` stops the heartbeat **after** `submit_result_over_http`, not before | R2; PLAN.md's loop already had this order |
| — | A test | `test_http_retries_outlast_a_server_deployment` in `test/contribute_test.c`: the schedule is monotone, capped at a minute, totals at least ten minutes, and does not overflow far past its end | The budget is a number with a purpose; the test holds it to the purpose |

`MAGPIE_VERSION` stays `0.1.0`: PLAN.md's rule is that it moves "only when a
release changes what a task computes", and none of this does. `docker/Dockerfile`'s
`MAGPIE_COMMIT` does not need to move either — the server runs MAGPIE for
conversions only, never as a client.

Verification: the dev build (`-Werror`, ASan, UBSan, LSan) compiles clean and
`./bin/magpie_test contribute` passes; `python3 format.py` (include order plus
`clang-format-20`) reports no differences; `find_circ_deps.py` reports the same
seven cycles with and without the change (all pre-existing, through `config` and
`json`); the `portable_release` build reports `0.1.0`/`nehalem`, builders 1, and
ran every job type end to end and both outage runs (section 13).

**Not pushed.** The commits are on the local `birdtest-contribute` only. CI's
`magpie-contract` job and the nightly both check out
`origin/birdtest-contribute`, so neither sees M1–M3 until it is pushed; nothing
in birdtest's build depends on them.

### Verified, unchanged

- **It runs every job type against this branch** (section 13).
- **Its half of the contract.** `./bin/magpie_test contribute` passes; the three
  assignment fixtures are byte-identical in the two repositories; the claim,
  decline and shutdown shapes were traced by hand against `routes/worker.rs` and
  the five non-assignment fixtures, as the ninth audit did, and agree.
- **What the server runs.** `magpie builders`; `help convert` lists
  `dawg2wordmap`, `klvwmp2rit`, `rackequity2klv`; `createdata klv` built the
  generation-0 KLV in the end-to-end run.
- **The argument trace**: section 2.

Noticed and left: `find_circ_deps.py` reports seven include cycles on the
branch, and MAGPIE's CI lists circular-dependencies among its required checks.
They predate this audit and do not affect birdtest; whoever merges the branch
upstream will meet them.

---

## 10. Deployment blockers

### Resolved in this audit

| # | Blocker | Resolution |
|---|---|---|
| D1 | **A deployment is seven or eight minutes with nothing serving.** ECS deregisters a stopping task's targets and waits out the target group's deregistration delay — 300 s by default — before `SIGTERM`; with one task and `deployment_minimum_healthy_percent = 0`, that is five minutes of `503`s before the old process is asked to stop. The new task then waits for three health checks thirty seconds apart | `infra/ecs.tf`: `deregistration_delay = 30` and `interval 10 / healthy_threshold 2` on both target groups; the service's comment and PLAN.md say what the gap really is. `terraform fmt -check` and `validate` pass |
| D2 | **`SIGTERM` waits for a `SIGKILL` whenever a dashboard is open.** Demonstrated: with one `curl -N` on a job's stream, a build without the fix was still running **41 s** after `SIGTERM` and had to be killed | `state::Shutdown`; `main.rs` triggers it with the signal; `job_stream` ends on it (`take_until`). The same check with the fix: the process exits **0.1 s** after `SIGTERM` and the client sees a clean end of stream. Test: `worker_api::a_live_stats_stream_ends_when_the_server_is_told_to_stop` |
| D3 | **The production client cannot ride out a deployment** | MAGPIE M1, M2 |
| D4 | **A restart abandons the fleet's work in flight** | R1 |
| D5 | **A slow or retried submission can outlive its own claim** | R2 / MAGPIE M3 |
| D6 | **One broken client can wedge a leave job permanently** | B1 |
| D7 | Dashboards dead after every deploy | B2 |

### Not resolvable from here

- **Push `birdtest-contribute`** (section 9), then the nightly will exercise
  M1–M3 on GitHub's runners.
- **Fargate's provisioning time** is the floor on a deployment's gap. Getting
  under it means two tasks alive at once, which is PLAN.md's "primary/secondary
  split" — a design the single-instance assumptions (imports, exports,
  transitions, rate limits, SSE) were written against. Not proposed.

### Checked, no change needed

Migrations run before bind; required secrets fail startup when absent; CSRF on
every cookie-backed mutation; the server refuses to start without a working
MAGPIE at or above its floor; CI on `main` is green after the merge; the MAGPIE
pin is fetchable; the nightly workflow builds `portable_release`.

---

## 11. Left for human input

### U1 — a lap's end can idle a leave job for the heartbeat timeout plus a full task

*Builds on the ninth audit's U2 (b), as built in `c46ff73`.*

A sweep's lap starts only with nothing in flight and nothing staged — that rule
is what lets a claim carry no exclusion list. So when a lap's racks run out, the
job hands out **nothing** until the lap's last results are in and merged. The
ninth audit's record costs that at "one task's duration and one merge per lap",
which is the case where every worker holding a last task is alive.

When one is not — a contributor closes a laptop with one of the lap's final
tasks — the wait is the heartbeat timeout (five minutes; ten after a restart,
with R1's grace) for the claim to lapse, **plus a whole task's duration** for
whoever is reissued it, and the entire fleet on that job idles or goes elsewhere
for all of it. A lap is about 6,400 tasks for English, so this is a per-lap tax
rather than a per-task one, and a fleet with other jobs to run loses nothing but
the leave job's share. It is left alone because every repair gives back some of
what the decision bought:

- **(a)** Start the next lap while stragglers are out, carrying an exclusion
  list of *their* racks only. Bounded by the stragglers (a handful of tasks),
  not by what is staged — so U2's original cost does not return — but it
  reintroduces the list, and the next lap selects on counts that are missing
  the stragglers' own results.
- **(b)** Shorten the wait rather than remove it: near a lap's end, reissue a
  task redundantly once its claim has been silent for, say, two heartbeat
  intervals rather than ten. Needs a notion of "suspect" claims the design does
  not have, and burns a duplicate task when the worker was only slow.
- **(c)** Leave it. (The real cost is now recorded in PLAN.md next to the pause
  it already describes — a fact, not a decision.)

**Recommendation: (c) now** — it is a throughput tax on one job type, not a
correctness problem — **and (a) if leave generation is ever the only job a large
fleet is running**, where an idle lap end is the whole fleet idle.

### U2 — should a worker be told how long the server expects to be away?

*New; follows from D3.*

M1's fifteen minutes is a guess at "longer than any routine outage". RUNBOOK
§1's full restore is "under an hour", and a contributor whose client gave up at
minute fifteen is gone until a person notices. Two ways to close that:

- **(a)** Never give up on a claim: treat an unreachable server like `204` and
  poll at the ceiling for ever (keeping the budget for *submissions*, whose
  claim will have lapsed anyway). A fleet that survives any outage; a client
  that spins quietly against a server that has been decommissioned, or a typo'd
  `server` line, until someone looks.
- **(b)** Keep a finite budget and make it a `contribute.txt` setting
  (`retryminutes`), so an operator planning a long window can ask contributors
  to raise it.
- **(c)** Leave it at fifteen minutes.

**Recommendation: (a) for claims only, with a log line per retry at the
ceiling**, because the asymmetry is large — an idle poll a minute costs nothing,
and a lost contributor costs their machine for as long as they do not notice. It
is a behavioural choice about someone else's computer, so it was not made here.

### Smaller, noted rather than asked

- **The reclaim statement runs on every claim request.** Bounded by claims in
  flight, so milliseconds; throttling it per process is available when a fleet
  is large enough to care (section 5).
- **Re-dispatch at redundancy above 1** (section 6, item 5).
- **`reclaim_expired` (single job) is now called only by tests.** Kept: six
  tests use it to reclaim deliberately, without the grace.
- The ninth audit's smaller notes stand.

### To do after merge (not decisions)

1. Push `birdtest-contribute`.
2. `terraform apply` the target-group change before the next deploy that
   matters; it is in-place on both groups.
3. The nightly runs at 06:00 UTC on the merged `main`; read it.

---

## 12. Verification

- Backend: `cargo clippy --locked --all-targets -- -D warnings` clean; `cargo
  test --locked` against Postgres 16: **196 tests** (97 unit and contract, 99
  integration), all passing (191 before this audit). The five `magpie_smoke`
  tests are `#[ignore]` by design.
- Frontend: `npm run check` — 0 errors, 0 warnings.
- Terraform 1.9.8: `fmt -check -recursive` clean; `init -backend=false` and
  `validate`: "Success! The configuration is valid."
- MAGPIE: section 9.
- B1's integration test demonstrates the wedge it prevents (a hand-staged
  `i64::MAX` fails `merge_staged` twice running). R1's test fails against the
  old claim path by construction: the second worker is handed the first
  worker's seed.
- The schema block in PLAN.md is byte-identical to the migration.

---

## 13. End-to-end runs

`scripts/e2e_magpie.py`, **unmodified**, against a native stack: a throwaway
`postgres:16` (2 CPUs, 1 GB) and MinIO (stock images, nothing built), the
backend and `build-derived` run from this branch's debug build with `MAGPIE_BIN`
pointing at the checkout's `portable_release` MAGPIE **with M1 and M2 in it**,
and a PATH shim that turns the script's three `docker compose` calls into their
native equivalents.

| Job | Result |
|---|---|
| `games` | 2 accepted claims |
| `game_pairs` | 2 accepted claims, pentanomial stored |
| `opening_rack`, static, with a wordmap | derived files built natively; the worker's own wordmap agreed; 2 accepted claims |
| `opening_rack`, simming | 2 accepted claims, simulated statistics stored |
| `leave_generation` | 2 accepted claims by sweep over a real 3,199,724-rack universe; results staged, merged on demand, nothing left staged |

The backend log held **no** `ERROR` or `WARN` line. Run **twice**: once with M1
and M2 in the client, and again at the end on a fresh database with the final
builds of both repositories (M3, and B1's bound on the server). The second run
is also B1's false-positive check against a real client: its two leave tasks of
20 games reported **1,390** occurrences over 779 distinct racks — about 35 a
game, against a ceiling of 1,000 a game.

### Two live outages

Then, against the same stack and with M3 built in as well, a script that does to
the server what a deployment does (kept out of the repository: it kills and
restarts a process by PID). A `games` job of static players; `HEARTBEAT_TIMEOUT_SECONDS=70`
so that "an outage longer than the heartbeat timeout" takes a minute and a half
rather than six, keeping the production relationship that matters (the grace is
longer than the client's longest back-off). Worker 1 claims; the server is
stopped with `SIGTERM` fifteen seconds later; after the outage it is started
again and worker 2 claims **at once** — the request that used to abandon
everything.

| Run | Outage | Worker 1's task | What happened |
|---|---|---|---|
| 1 | 91 s | 40,000 games, still playing when the server returned | Worker 2, one second after the restart, was given seed 40001 — not worker 1's task. Worker 1's first heartbeat in two minutes landed fourteen seconds later; its result was accepted. Both exit 0; two `completed` claims, none abandoned |
| 2 | 80 s | 20,000 games, **finished during the outage** | Worker 1 had its result ready some fifty seconds before there was a server to give it to — past the old client's 31-second budget, at which it would have ended its run and lost the task — and went on retrying; accepted three seconds after the restart. Its claim was by then 96 s old against a 70 s timeout, with **no heartbeat ever received**; worker 2's claim request, one second after the restart, did not reclaim it and was given seed 20001. Both exit 0; two `completed` claims, none abandoned |

### Shutdown with a stream open

One `curl -N` on `/api/jobs/:id/stream`, then `SIGTERM` to the backend. With the
fix: exit after **0.1 s**, and the client's stream ends cleanly. With
`take_until` removed for the experiment and put back: still running **41 s**
later, which is past ECS's default stop timeout.

Both containers and the scratch database were removed afterwards.

---

## 14. Tests added

| Test | What it pins |
|---|---|
| `worker_api::a_restarted_server_does_not_abandon_claims_it_could_not_have_heard_from` | R1: a fresh process's first claim request reclaims nothing; the surviving worker is heard from and its result accepted |
| `worker_api::a_claim_still_silent_after_the_grace_is_reclaimed` | R1's other half: the grace is a delay, not an amnesty |
| `worker_api::a_live_stats_stream_ends_when_the_server_is_told_to_stop` | D2: the stream stays open until the shutdown signal, then ends, having sent its first payload |
| `plausibility::tests::a_leave_batch_cannot_report_more_occurrences_than_its_games_drew` | B1: the bound, its edge, garbage near `i64::MAX`, and a total that must not wrap while being added up |
| `leave_gen::a_count_no_game_could_produce_is_refused_before_it_can_wedge_the_merge` | B1 end to end: `400`, nothing staged, an honest result then accepted and merged — and the wedge itself, staged by hand |
| MAGPIE `test_http_retries_outlast_a_server_deployment` | M1: the schedule is monotone, capped at a minute, and at least ten minutes in all |
