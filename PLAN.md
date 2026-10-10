# birdtest — Project Plan

## High-Level Design

### Overview

birdtest is a crowdsourced crossword game research platform, modeled after Fishnet (which crowdsources chess game analysis for Lichess). It runs MAGPIE, a crossword board game engine, on volunteers' computers to play test matches between versions, tune its settings and study openings. Users contribute compute by running tasks locally and submitting results back to the site. Admins define jobs and allocate work; the site aggregates results and presents them on a polished dashboard.

---

### Jobs

Jobs are long-running research goals defined and managed by admins. The following job types are supported:

- **Analyze all possible opening racks**
- **Run games** — autoplay using any player configuration; supports pure static players (no simulation), simming players, or any mix.
- **Run game pairs** — same as games but run as matched pairs (same seed, players swapped) to reduce variance.
- **Leave generation**

Each job has a **percentage allocation**, and nothing else decides who gets work: every claim goes to the active job furthest behind its share (measured from when the job last joined the jobs on offer, not over its whole life — see [Workflow](#workflow), step 2), and the active jobs may allocate at most 100% between them (enforced at the application layer, not via a DB constraint, and serialized so two concurrent allocation changes cannot jointly exceed it). There is no priority. A job that should get nothing for now is set to **0%**, which *is* the inactive state, and a job that should get everything is the only one above 0% — so allocation alone expresses every ordering an admin needs, without a second axis to keep coherent with it. Jobs and their allocations are managed by admins only.

#### Job Lifecycle Controls

Jobs are created by admins **inactive at 0%** and only start receiving work once given an allocation. **The allocation is the only on/off switch**: for a job that is not completed, `active` exactly when its allocation is above 0% (the `jobs_allocation_is_status` check in `0001`, with `allocation NOT NULL DEFAULT 0`). The following states are supported:

- **active** — above 0%; workers are assigned tasks from this job normally.
- **inactive** — at 0%; the job exists and retains all its tasks and results, but workers are not assigned tasks from it. Giving it an allocation again reactivates it. The server switches a job off itself in one case, the same way: three of its tasks in a row hit the [time limit](#task-time-limit), and the job is *set aside*, inactive at 0% with the reason on its page. There used to be a separate activate and deactivate besides the allocation, and with them an active job at 0% — offered to nobody while every page called it running — and an inactive job that remembered a share; they are gone, and deactivated now just means 0%.
- **completed** — all tasks have been completed, either automatically when the finish condition is met or manually by an admin. A completed job holds **0%**: every completion, the server's or an admin's, writes the allocation to 0 with the status, so its share goes back to the fleet and nothing of it is remembered.

Allocation is not set at creation time. The admin sets it on **`/admin/allocation`** (`PUT /api/admin/jobs/allocations`, the only endpoint that activates or deactivates a job), which shows every active and inactive job together and checks the shares as a whole. This keeps the allocation budget coherent: an admin reviews the full set of active jobs, decides the new job's share, and activates it — moving shares from other jobs in the same save if need be. The admin job page shows the job's allocation read-only, with a link there. A games or game-pairs request naming several player configs creates a **round robin** — a job per pairing, all inactive at 0% ([Admin API semantics](#admin-api-semantics)) — and the form lands on `/admin/allocation` with them marked, to be started together.

Admins can set a job's allocation (activating or deactivating it), purge, force-complete, or delete a job at any time. **Purge deletes tasks outright** rather than returning them to `available`: every job type generates its tasks on demand, so a purged job regenerates them from the start of its space at the next claim. Leaving the rows behind would advance the seed cursor past work that was never done. Purging also rebuilds the generation-0 zeroed KLV a leave-generation job needs before it can dispatch; the generation-1 rack universe it deleted is seeded again by the first claim, as every generation's is. It takes the job's **merge lock** before anything else — a merge of a leave job's staged results takes the staged rows and then the per-rack rows, a purge deletes them the other way round, and run together the two deadlocked, with the purge the likelier victim: a `500` and nothing deleted (`leave_gen::lock_merges`; it is first in the lock order everywhere, because nothing takes it while holding another lock). Then the job's dispatch lock, the same one every claim takes: a claim in flight has already read the seed cursor and is about to insert its task, which the purge's deletes cannot see, so without the lock the purge finishes and the claim then commits a task into the job it just emptied. It then waits out every open claim's in-flight submission before taking the job's row, which is the order every submission locks in (claim, then task, then job): taken the other way round, a submission arriving mid-purge deadlocked against it, and one that committed between the purge counting contributions and deleting claims was never handed back. Delete takes the same three locks for the same reasons.

A completed job cannot be given an allocation, not even 0%: setting it inactive and then raising it would otherwise restart it. Force-completion is unconditional, and sets the allocation to 0 as every completion does. (An opening-rack job is the exception: a [consensus edit](#opening-rack-consensus) that unsettles its racks reopens it, force-completed or not — inactive at 0%, for the admin to give it an allocation.) **A purge leaves every job inactive at 0%**, as a new one is: a completed job has nothing left to be complete about, and one left `completed` was an empty job nothing could ever run again — where the purge is meant to start it over; an active one stops too, so the emptied job takes no claims until the admin gives it an allocation again, which joins it at parity as any activation does.

**While a purge or delete runs, claims skip the job** (and while an opening-rack job's [consensus edit](#opening-rack-consensus) runs, which holds the same locks) without asking the database (`jobs::DispatchHolds`, an in-process set; the service is a single instance), and nothing else waits on what it holds: a heartbeat skips a locked claim (`FOR UPDATE SKIP LOCKED`), reclamation skips one, a submission or decline for one of the job's claims is answered `503` with `Retry-After` at once (and any other submission waits at most five seconds for its claim's row, `lock_timeout`), and a worker request's `last_seen_at`/`last_used_at` touch skips a locked identity row. What the job's contributors earned is counted before the claims go and given back as the purge's last statement, so their rows are held for milliseconds — given back first, as the eleventh audit left it, every contributor's row was held for the whole purge and every request those identities made waited on it, stalling the fleet the same way through a different table. A purge that ends without committing (the request dropped at the load balancer's timeout, a deadlock) spares the job's claims from reclamation for a heartbeat timeout afterwards: their heartbeats were skipped while it held them, not missed. A consensus edit spares them however it ends, since they outlive it.

**Closing a leave generation takes the job's merge lock first**, as purge and delete do, so a close racing a purge waits holding nothing and then finds its transition gone. It took the transition row and then waited on the job's row (the artifact insert's foreign key) while the purge, holding the job's row, waited on the transition row — a deadlock Postgres resolved by aborting the purge after it had run for its whole length. The final generation's close completes the job only if it is still active. A purge of a large job holds every open claim of the job for as long as its deletes run, minutes for a full opening-rack job; before the eleventh audit each of those waited with a pool connection held, the fleet's heartbeats filled the twenty-connection pool within one thirty-second cycle, and every claim and submission on the server failed until the purge committed.

---

### Tasks

Tasks are the atomic units of work that workers execute. The following task types are supported:

- Analyze a contiguous range of opening racks
- Play a batch of games from a given starting seed
- Play a batch of game pairs from a given starting seed
- Play a batch of leave-generation games over a forced subset of racks

Every task carries a **seed** — a `uint64` value — because every job type plays games, and what a game samples must be a function of the task and never of the worker. The combination of `(job_id, seed)` must be unique; duplicate tasks are prevented at the database level. Games and game pairs seed their batch from it — MAGPIE draws each of the batch's game seeds from a stream seeded with it — and consecutive tasks' seeds step by the batch size, so the seed is also the job's cursor. An opening-rack task's seed is the index of its first rack in the job's rack space — rack `i` of the batch is analysed from `seed + i`, the same number on every worker — so the same column and the same index tile that space too. A leave-generation task's seed is drawn when the task is created and stored with it, so a reissued task replays it; two leave tasks are otherwise kept apart by the rule that a claim is never handed a rack another open claim is already forcing (see claim step 2 under Leave Generation), and a collision between two random draws is a unique violation the claim path retries.

#### Task States

Tasks move through an explicit state machine:

```
available → claimed → completed
               ↑          
         (heartbeat timeout or decline: the claim ends; the task returns to available)
```

A task has **one slot**: one worker holds it at a time, and its one accepted result completes it. State follows from denormalized counters (`accepted_count`, `active_claim_count`), each 0 or 1:

- **available**: no live claim and no accepted result — made, then given back (its claim lapsed or was declined); the next worker gets it before anything new is generated.
- **claimed**: a live claim; waiting on its result.
- **completed**: its result has been submitted and accepted.

A job used to state a `redundancy` X, and each task waited for X independent results. Nothing compared them (reconciliation was deferred), every aggregate had to read one result per task, and it cost X times the compute for the same answer; it was removed in October 2026. Opening-rack jobs instead re-analyse a rack until its analyses agree ([consensus](#opening-rack-consensus)), each analysis a task of its own.

Individual claims are rows in `task_claims`. When a claim's heartbeat times out, that claim row is flipped to `abandoned`, `active_claim_count` is decremented, and the task returns to **available**. Reclamation is lazy — it runs at the moment the next task is requested and the job is a candidate, not via a background process — **and a process reclaims nothing until it has been up for the heartbeat timeout itself** (`scheduler::reclaim_lapsed`). A heartbeat can only arrive at a server that is there to receive it: after an outage longer than the timeout, every open claim in the fleet looks that old however alive its worker, and the first claim request after the restart abandoned all of them — every task in flight handed out again, every result being computed answered `accepted: false`. A live worker's heartbeat arrives within thirty seconds of the server's return, so a claim still silent a full timeout after startup is reclaimed as before; what it costs is that a worker which really did die during the outage is noticed up to one timeout later. A job nobody asks for work from — one that is inactive, completed, or parked at 0% — therefore keeps a lapsed claim on its books until something reclaims it: activation above 0% makes it a candidate again, and starting an [export](#exports) reclaims the job's lapsed claims first, since a completed job is never claimed from again.

#### Task time limit

A task may run for at most **`settings.max_task_seconds`** — one hour by default, ten minutes to a day, set on `/admin/settings` (`PUT /api/admin/settings`) and read at each claim. Every assignment states it (`max_task_seconds`) beside the job's name (`job_name`, never empty: a job created without one is named for its type and id), and every claim stores its own deadline, `task_claims.deadline_at` = claim time plus the limit as it stood then, so a change applies to the claims made after it. A heartbeat is not enough to keep a task: a job whose one unit outlasts any machine — a game pair of deep simmers — had its tasks held for as long as their workers lived, and handed out again when they gave up. The floor is ten minutes, not one, because a task's first claim on a machine may build the job's rack info table first — a minute to three, which cannot be stopped part-way and is kept for every task after it — and a limit near that would stop that task every time on every new machine. The column's CHECK holds the same floor as the API, so nothing gets under it; a test that wants a claim past its deadline moves the claim's `deadline_at` and `claimed_at` back instead.

- **The worker stops at the limit.** MAGPIE stops a task that reaches `max_task_seconds` by its usual stop path, hands it back unfinished and declines it `time_limit`. The decline releases the claim at once, like any other, and is counted against the job (`jobs.time_limit_declines`): the job's page says "N tasks hit the time limit — lower the batch size". A `time_limit` decline is a task the worker could run, so it does not undo the job's settling, as `task_failed` does not.
- **The server stops waiting a minute later** (`scheduler::DEADLINE_GRACE`). Reclamation lapses a claim past its deadline and the grace whether or not its worker still heartbeats — a build that ignores the limit, a solve hung but heartbeating — in the same statement that lapses a silent one, so the two can never both release one claim. A result for such a claim is answered `accepted: false`, as a lapsed claim's is, whether or not reclamation got to it first (a job nobody claims from is never swept, KL-1), and the claim is released in that transaction, its task back out at once. Neither happens while the process is in its startup grace, nor while a job's claims are in the grace after a purge or delete let go of them without committing (`scheduler::deadlines_enforced`): those are the spells in which a worker could not reach its claim — an outage, a purge's `503`s — and a result that could not land sooner is not late by its worker's doing.
- **A lapse with the worker alive is an overrun.** A solve or a build that overruns MAGPIE's stop declines only after the claim has lapsed, and that decline is a `404`; a build that ignores the limit never declines. Either way the task hit the limit, and a job whose tasks always do would be handed out for ever. So reclamation, lapsing a claim at its deadline, marks it an overrun (`task_claims.overrun = 'pending'`) when its worker was alive when it lapsed: its last heartbeat within the heartbeat timeout of the deadline and grace, however late the sweep comes round. One whose worker had gone silent by then is what a dead worker leaves, and is not marked. A result refused past its deadline marks its claim too — that worker is plainly alive. An overrun counts toward setting the job aside as a `time_limit` decline does.
- **Three in a row set the job aside.** `jobs.time_limit_streak` counts the job's `time_limit` declines and overruns since a task of it was last completed; every accepted result zeroes it (in the `UPDATE jobs` every submission already makes). `jobs.time_limit_declines`, the page's "N tasks hit the time limit", counts both. The third in a row sets the job aside: inactive at 0% in one write, as `set_allocations` leaves a job at 0%, so `jobs_allocation_is_status` holds; a `job.set_aside` audit row with no actor, from its share ("50% -> 0%: …"); and the reason on the job (`jobs.set_aside_reason`), which its page shows in place of "Paused". Three rather than one: one is a slow machine or an unlucky batch, and a declined task goes to another worker (a worker is not handed back a task it declined within the hour); three with nothing completed between is a batch too big for the limit. A job's batch is fixed at its creation, so the cure is a new job with a smaller batch, or a longer limit — and an allocation, which clears the reason and starts the run afresh. A purge zeroes all three columns, and deletes the claims an uncounted overrun is marked on.
- **Overruns are counted lazily, in order.** Reclamation runs over every candidate job in one statement on the claim path, and taking their rows there would serialize every claim behind it, so it only marks an overrun. It is counted (`pending` → `counted`, `worker::record_time_limit`) by whoever next holds the job's row for it: the job's next claim (`scheduler::count_overruns`, after reclamation and before anything is handed out), a `time_limit` decline, or a result refused past its deadline. Counted late, an overrun joins the run only if its deadline came after the run began — `jobs.time_limit_streak_since`, set by every accepted result and by the allocation that puts an inactive job back — and otherwise counts toward the total alone: an overrun from before a completion is not part of the run that completion ended. A job the claim's count sets aside is dropped from that claim's candidates, so the claim that found it is handed another job's task, or none.
- **What a claim pays.** One statement per claim request, asking which candidate jobs have an uncounted overrun, through `task_claims_overrun_idx` — a partial index on `job_id` holding only `pending` overruns, each from the reclamation that marks it to its job's next claim, decline or late result: all but always empty, and never more than the claims that were in flight. Only a job that has one pays more: a short transaction of its own under the job's dispatch lock (`try_lock_job_dispatch`'s bounded wait; a job a purge or delete holds, or whose lock does not come, is left to the next claim), and a failure is logged, not fatal.
- **Lock order.** The decline holds its claim (`FOR UPDATE`, under the five-second claim lock wait, and answered `503` while a purge or delete holds the job, as before), then its task (`release_claim`), then the job's row (`record_time_limit`): claim, then task, then job, the order every submission and purge takes. The deadline's refusal of a result takes the same three, its claim, task and job. Counting overruns takes the job's row and then the job's `pending` claims — the job's row first, by every caller, so two counts queue on the row rather than lock the same claims in two orders. Nothing else locks an abandoned claim (a submission, a decline, a heartbeat and reclamation take only open ones) but a purge or a delete, which holds the job's dispatch lock throughout — the claim path's count takes that lock first, and a decline or a refusal holds an open claim the purge waits out before it takes the job's row. Reclamation still skips a locked claim, deadline or not, and takes no job's row.

A fleet on a MAGPIE that honours the limit declines `time_limit`; a fleet that ignores it, or a task that overruns the stop, is taken back a minute past the deadline and counted as an overrun; either way three in a row set the job aside. Only a worker that was silent when its claim lapsed counts toward nothing.

#### Task Generation

Every job type generates its tasks **on demand**: the next task request is generated, inserted and claimed in one transaction at claim time. A task whose claim lapsed or was declined returns to `available` and is re-dispatched before anything new is generated. There is no pre-populated strategy (see [Creation Strategies](#creation-strategies)).

---

### Workflow

1. The worker sends a **task claim** to the server — a minimal message identifying itself and signaling it is ready for work.
2. The system selects the active job **most behind its configured allocation share** — specifically, among active jobs with an allocation above 0%, the one with the lowest ratio of `(claims_issued - claims_baseline) / allocation`, where `jobs.claims_issued` counts every claim ever issued for that job, **including abandoned and declined ones** — a claim consumed real dispatch capacity at the moment it was issued regardless of what happened to it afterward, so the count only ever goes up (a purge, which deletes the claims it counts, resets it). It is a counter rather than a `COUNT(*)` over `task_claims` because selection runs on every claim request, and a count grows with each job's whole history. Excluding abandoned claims would let a job with flaky or slow workers accumulate a disproportionate share by having its timeouts discounted, and would make the count non-monotonic — the opposite of what the deficit-based scheduler needs. Ties are broken by job creation order (oldest first). This is a deterministic deficit-based selection; no randomness is involved.

   **A job's share is measured from when it joined, not from when it was created.** `jobs.claims_baseline` is reset — on an allocation change, which is also how a job is activated (a purge zeroes it with the job left inactive, to rejoin when activated again) — so that the job's ratio equals the **lowest ratio among the other jobs being served** (`scheduler::join_at_parity`), and it takes its share from then on. *Being served* means a claim issued within the heartbeat timeout of the most recent claim of any other job on offer — `jobs.last_claimed_at`, which rides the `UPDATE jobs` every claim already makes — measured from the latest claim rather than from now, so after a quiet spell (a deployment gap, a quiet night) the jobs that were being served when the fleet stopped still set the pace; with no other job ever claimed, the highest ratio on offer, or zero. Joining is then **settled**: for an hour after it joined (`scheduler::JOIN_SETTLE`, from `jobs.activated_at`, which every join sets), each claim of the job lifts it level with the lowest of the claiming worker's other candidates — the ones it just passed over included — less one claim of that job and one of this one, the slack the turn check allows (`scheduler::issue_claim`, `pace_for`). A worker that declines the job because it cannot run it at all (`missing_data`, `magpie_version`, `unknown_job_type`, `derived_mismatch`) undoes the settling its claim gave (`scheduler::unsettle`): the server cannot filter a data gap, so every worker without the job's data is issued one claim of it, and that claim had settled a job only a minority could run at the majority's pace — past the minority's own lagging job, where the minority never reached it (none of 400 claims). Two more rules keep a job with nothing to hand out from banking debt: **a job passed over for want of a task is lifted to where the job the worker claims stood before its claim** (`scheduler::lift_passed_over`, after the claim commits, skipping a row somebody holds) — a games job whose every game is in flight, a generation being built, a dispatch hold (a seeding, a purge) — and **a job not served for a heartbeat timeout rejoins at parity on its first claim back, which starts its settling** (`scheduler::issue_claim`) — one whose MAGPIE floor was above the fleet's, whose data was not out. Every one of these only ever raises a job's ratio. The reason for the two steps is that a fleet split by capability — a release rolling out, data some workers lack — has no single pace: each class of workers runs its jobs at its own rate, and a job only some can run lags the rest for as long as the split lasts. That lag is the scheduler working (the minority's job gets all of the minority), but it means no one join point is right. What each single point did (the thirty-second audit, passes 19 and 20): level with the lowest of *every* job on offer, a newcomer was level with a job nobody could run and took twelve of the next twelve claims; level with the *leader*, a newcomer only the lagging class could run starved behind that class's job — none of the next 1,000 claims; bounding every job's lag behind the job just claimed to make up for that scrambled the jobs that lagged together (two at 45% split 978 : 22) and made a concurrent burst to a small job permanent (64 to 87 claims where 30 is fair); and level with the lowest *served*, a newcomer everyone could run was level with a job only a minority could run and took the majority's claims until it had caught theirs — the majority job's first claim came 331st, and an allocation changed from 20% to 19% did the same. Settled against the next candidate only, a job that paused for one claim let a newcomer be settled past it; and settling forgave the payback of a concurrent burst (307 to 324 claims where 30 is fair) until claims were checked for their turn. Joined at the lowest served, a job is below no class's pace, so it is never starved; settled, the first claim from each class that runs faster puts it level with that class's jobs, so it takes nothing over; and a structural lag is never lifted, because within the class that runs it a lagging job keeps pace. The limits of the hour and of the lift are KL-89. This is start-time fair queuing's rule — a flow that (re)joins starts at the current virtual time, of the workers that will serve it, and a flow with nothing to send earns no credit — and it is what makes "no starvation" true. Measured over a job's whole life, as it was, every change to the set of jobs was a takeover: a job activated beside one that had issued two million claims had a ratio of zero, so it was first in every candidate list until it had issued two million of its own, and the older job — at the same 50% — got *nothing* for as long as that took. A purge (which zeroes `claims_issued`), a reactivation after a week switched off, and an allocation raised from 10% to 50% (which cuts the ratio to a fifth) all did the same. With the baseline, the shares an admin sets are the shares the fleet sees from that moment, selection is still one deterministic statement, and the long-run ratios still converge on the allocations, because every job's numerator counts from the same point in the fleet's history. The baseline can be negative (a job with no claims joining a busy fleet is credited the claims that put it level).
3. Expired claims for the candidate jobs are lazily reclaimed, in one statement: each timed-out `task_claims` row — silent for the heartbeat timeout, or past its [deadline](#task-time-limit) and a minute's grace however recently it heartbeat — is flipped to `abandoned`, `active_claim_count` is decremented, and their tasks return to `available`. A claim somebody holds locked — a submission, a decline, a purge — is skipped rather than waited on (`FOR UPDATE SKIP LOCKED`): it is not lapsed in any sense that matters, and the next claim request reclaims it if it still needs to be. One lapsed at its deadline with its worker alive is marked an overrun, and the overruns not yet counted against the candidate jobs are counted then; a job they set aside is not a candidate for this claim ([Task time limit](#task-time-limit)).
4. The system acquires the next task (one being re-dispatched, or else one generated on demand), inserts a `task_claims` row, increments `active_claim_count`, and issues a claim token (UUID) to the worker.
5. The server responds with the **task request** for that job type.
6. The worker performs the task and submits a **task response** along with the claim token.
7. If the claim token matches a `task_claims` row that is not abandoned and was issued to the identity presenting it (a token presented by any other identity is treated as unknown, so bans and audit rows mean what they say), the task response is accepted, a **task record** is stored keyed to the `task_claim_id`, `accepted_count` is incremented, `active_claim_count` is decremented, and the task is marked **completed**. If the token is stale (the claim was abandoned due to timeout, or this result was already accepted), the submission is answered `{"accepted": false}` and changes nothing. The claim row is locked from lookup to commit, so a timeout reclaiming it concurrently cannot count it as well.

---

### Workers

Workers are the clients that perform tasks and submit results. Two types are supported:

- **Anonymous workers**: Identified by a UUID **the server mints**, not one the client invents. A worker with no credentials sends no identity header at all on its first claim; the server draws a UUID, and only when that claim actually hands out a task does it insert the `anonymous_workers` row (in the claim's own transaction) and return the UUID in the response body. A claim answered `204` writes nothing — otherwise every idle poll from a new contributor would mint and orphan an identity — and every worker endpoint other than the claim answers `401` without an identity. The client persists it and sends it as `X-Worker-UUID` from then on. A UUID that does not already exist in `anonymous_workers` is rejected with `401` and a message naming the fix, because a client-invented identity is one the server never got to validate — it would let anyone manufacture contributors, attribute work to identities that never claimed anything, and hand per-worker anomaly detection a population it does not control. Contributions are tracked per UUID but **displayed under a pseudonym**: the first 16 hex characters of the UUID's SHA-256, labelled "Anonymous". The UUID is the worker's only credential, so no public endpoint returns it (`/api/workers`, job stats `workers` and `/api/jobs/:id/results` all carry `anon_id`, and `?worker=` accepts it); only `GET /api/admin/workers` returns real UUIDs, for banning. A request from a known UUID refreshes `last_seen_at`, but at most once a minute — it answers "is this worker still around", which a per-minute resolution answers just as well as a write on every request would (`api_keys.last_used_at` is throttled the same way).
- **Authenticated workers**: Identified by an API key tied to a user account. Contributions are tracked per user.

#### Worker Integrity and Anomaly Detection

- **Plausibility checks at submission time** — every submission is checked against what is *possible*, not against what is usual: a negative standard deviation, a play scoring negative points, a rack with eight tiles (counted as tiles: a multi-character letter such as Catalan's `[L·L]` is written bracketed and is one), a negative count of moves generated, a per-ply bingo percentage outside [0, 100] or plies out of order, a pentanomial whose pairs need more draws than the games report, a batch reporting a different number of games than the task dispatched. The shared checks are in [`backend/src/jobs/plausibility.rs`](backend/src/jobs/plausibility.rs); the per-job-type ones are in each handler under `backend/src/jobs/` — the pentanomial cross-check (counts in range, pairs = games / 2, the score and draw identities) in `game_pair.rs`, a batch's racks against its task in `opening_rack.rs` (`check_batch_against_task`), and so on. This is the only active integrity mechanism at submission time, and the rest of this section explains why it is the only one that can be.
- **Worker ban list** — a persistent table of banned worker identities; banned workers cannot claim or submit tasks. Meaningful for authenticated workers; for anonymous workers, banning targets the UUID. Applied by an admin; nothing bans automatically. A ban binds an identity, not a person: a client that sends none mints a new one on every claim, so it cannot be banned, and a banned account's owner can go on contributing without a key (KL-56). **One row per identity**, enforced by a partial unique index: enforcement is an `EXISTS`, so a second row's reason is never read, while unban deletes by row id — so a duplicate would leave an identity banned after an admin had lifted the ban, with nothing to say why. Banning again with a different reason is unban-then-ban, which the audit log records as both halves.
- **No redundant task execution** — a task has one slot and one result. Jobs used to state a redundancy X, with X workers completing each task, but no consensus or agreement check was ever built, so the copies cost X times the compute for nothing an aggregate could use; it was removed in October 2026. The cross-check below is what would make replication worth its cost.

#### Why impossibility, and not per-worker anomaly detection

The obvious design — the one fishnet uses — is a per-worker statistical test
against the population: flag the worker whose results deviate, without needing
to trust any individual submission. **That does not transfer to birdtest**, and
building it would be worse than building nothing.

It works for fishnet because chess analysis is **replicated**: two honest
clients at the same depth on the same position return the same evaluation, so
disagreement is proof. birdtest has no ground truth to compare against. Workers
are handed *different* seeds — that is how the seed space tiles without gaps or
overlaps — so no two workers ever play the same games. The only cross-worker
statistic available is the win rate, and that is precisely what the match
test is measuring. A test on it cannot separate "this worker is broken" from "these
seeds favoured player 2", so it would flag honest contributors at its own alpha
rate while an attacker biasing results by a percent passed straight through.
Two further problems compound it: opening-rack analysis by a simming player is
non-deterministic by construction, so honest repeat runs disagree; and anonymous
identities are free, so a per-worker score is defeated by requesting a new UUID.

What is left is the failure that actually happens: a **broken client**. Those do
not produce subtly shifted distributions — they produce garbage. So the checks
that ship are hard rules with no false-positive rate to trade against, and every
one of them rejects an arithmetic or physical impossibility:

| Check | Why it cannot be a false positive |
|---|---|
| Score means and standard deviations are finite | `NaN`/`Inf` is what an uninitialised or corrupted buffer serialises to |
| A standard deviation is not negative | Arithmetically impossible; the number did not come from a variance calculation |
| Mean scores lie within generous absolute bounds | A word game cannot average a negative or four-figure score |
| A play scores between 0 and 100,000 | A pass scores 0; the theoretical maximum play is a little over 1,700 on the 15×15 board, and the 21×21 board's quadruple-word corners multiply an edge-long word by 144, so the bound sits far above both (it was 2,000 until the thirty-second audit) |
| Win percentages and blended utilities are inside their ranges | A probability is bounded by definition |
| A rack has 1–7 tiles | More tiles than a rack holds cannot be dealt |
| `num_moves` is at least the number of moves reported | A worker cannot report more moves than it says it generated |
| A leave submission lists no rack twice | Occurrences are **summed** on receipt, so a duplicate silently inflates a generation's coverage |
| A leave submission reports no more rack occurrences than its games could draw | At most two racks are recorded a turn, so `num_games` × 1,000 is far past any game; and the counts are summed into `bigint` columns, where a garbage count near 2^63 made every later merge of the generation fail and the generation impossible to close |
| A batch reports exactly the games the task dispatched | The size was fixed when the task was handed out |
| An opening-rack batch analyses exactly the racks the task dispatched | The racks themselves were named when the task was handed out |
| A position's analysis is one its task's players could run: a simulation or an inference only from a player that simulates (and infers), a solve only from one that solves, an endgame at no depth past its `endgame_plies` and a pre-endgame at none past its schedule's | MAGPIE decides each turn from the mover's own settings (`num_plies`, `use_inference`, `endgame_plies`, `peg_max_bag`), and an endgame solve searches no deeper than it was asked to. A pre-endgame ranks stage `s` of its `peg_stage_top_k` at `s + 1` plies, and the one stage `[2147483647]` (MAGPIE's exhaustive mode) at 40, deeper than any endgame. Checked against either player, so the rule never needs to know which seat moved |
| An unsimulated move carries no iterations and no per-ply statistics | MAGPIE writes 0 iterations and no plies for every move it did not simulate, static or solved |
| A pair outside `divergent_games` is a win and a loss, or two draws | MAGPIE counts a pair as not divergent only when both games played the same moves throughout, with the first mover swapped: the same game from both seats |

The pentanomial cross-check ([MAGPIE reports the
pentanomial](#magpie-reports-the-pentanomial)) belongs to the same family and is
the sharpest instance of it: the two views of a batch are individually plausible
and only wrong *in relation to each other*.

Two rules were considered and deliberately left out, because they cannot meet
the no-false-positives bar:

- **A minimum time per batch.** Elapsed time is measured server-side, from
  `claimed_at`, so no client clock is involved — but the bound would have to
  encode a maximum plausible throughput, and that depends on the contributor's
  hardware, thread count and whether a wordmap is loaded. There is no
  hardware-independent figure, so any threshold risks banning a fast honest
  worker.
- **Identical submissions across different seeds.** Tempting, but two different
  seeds producing the same aggregate is ordinary for a small batch — a one-game
  batch has three possible results.

The natural next step, if replication is ever brought back (as sampled audit
tasks, say), is **cross-checking replicated tasks**: N workers running the same
seed with the same configs, each claim's row stored separately, and games
deterministic, so disagreement becomes proof rather than evidence. That is where
detection with real teeth lives, and it needs no population statistics at all.
It applies to jobs whose players are all static and solve nothing: a simulation
samples, and its thread count alone makes two honest workers' rankings differ,
and an endgame or pre-endgame solve is multithreaded and not reproducible run to
run either, so simming and solving jobs are excluded from any equality
cross-check.

---

### Worker Client

Contributors run a client program that loops continuously: it sends a **task claim** to the server, receives a **task request**, executes the work, and submits a **task response**. The client handles authentication (API key or anonymous UUID) and heartbeating automatically. See the [Worker Client](#worker-client-1) section for technical details.

---

#### Letter distributions are stated, not inferred

Every job config names its `letter_distribution` alongside its lexicon, and the
name is carried on the request the worker receives. MAGPIE derives a
distribution from the lexicon's prefix; birdtest used to mirror that inference,
which meant guessing at something the job can simply say.

Job creation parses the pinned distribution as every claim will, and refuses
one the server or MAGPIE cannot use: in particular one with more letters than
MAGPIE's `MAX_ALPHABET_SIZE` (50), which MAGPIE now refuses too and used to load
past the end of every per-letter array. It also refuses a job that needs a
wordmap or a rack info table on a distribution with more than two blanks
(`english_super`): MAGPIE aborts building either (`wmp_maker.c`), so such a
job's build failed three times and it never dispatched.

#### Two generate/report pairs on the player config

`player_configs` carries two pairs, each "how much to compute" against "how much
to report":

| Compute | Report | MAGPIE |
|---|---|---|
| `num_plays` | `num_plays_recorded` | `-np` / `maxnumdplays` |
| `plies` | `num_plies_recorded` | `-pl` / `shplies` |

A simmer may rank hundreds of candidates to order the top few correctly while
only the leaders are worth storing, and the same holds for plies. Because these
live on the player config rather than the job, one setting governs both opening
rack analysis and positions captured during games.

#### Position analyses from games

`games` and `game_pairs` jobs can keep the position analyses their workers
produce while playing, by setting `capture_positions`. A worker analyses a
position on every turn regardless; this decides whether those are recorded
rather than discarded, turning a job run to settle which player is stronger
into a corpus of analysed positions as well.

A `game_pairs` job can keep less: with `capture_first_divergence` as well, only
each pair's **first divergence**. A pair's two games share their tiles and swap
who moves first, so until the players choose different moves they are one game
played from both seats; the turn they first disagree is the one position both
players faced, with each player's ranking of it. From each pair that diverges
the worker keeps both games' positions at that turn -- one board, one rack to
play from, each player to move in one game -- and from a pair played
identically, nothing. MAGPIE's positions recorder holds each turn's positions
until the pair's play says whether that turn is the divergence
(`divergentpositions`, `autoplay_results_commit_positions`), so nothing else
reaches the result. The server checks the shape strictly: every pair with
positions has exactly two, at one turn, on one board with one rack, and there
are as many such pairs as `divergent_games` says diverged. The job's page shows
every saved position of a pairs job beside its partner -- the same turn of the
pair's other game -- which for a first divergence is the two players' answers
side by side.

They share `position_analysis_records` with opening racks -- the request differs
by job type, but what comes back is a position analysis either way. In-game rows
carry the CGP, the game index and the turn number, which are NULL for an opening
rack.

A simulated in-game position can also carry its **inference**: what the
player inferred of the opponent's leave from their previous move, before it
simulated. MAGPIE infers once per turn, for a player that uses inference, past
a game's first turn, when the previous move was not a pass; an opening rack
never infers (there is no previous move). It is a position's, not a move's,
so it is stored once per position (`position_analysis_inference`): how many
distinct leaves the inference found, how many it drew, their mean equity, and
up to ten of the leaves the opponent most likely kept, most drawn first. The
job page shows it under the position's moves.

Two things this design turns on, both consequences of a task having one slot:

- **In-game positions are keyed on `(task_id, game_index, turn_number)`**: a
  task has one slot, so a conflict could only be a duplicate, and the insert
  fails on one rather than skipping it. Opening racks keep their per-claim key,
  so the several analyses of a rack a consensus job asks for can be compared.
- **There is no sampling and no per-task cap.** Every position of every game is
  captured, which makes `games_per_batch` the control on submission size: a
  batch of 20 games is a few hundred KB, a batch of 1,000 is on the order of
  15 MB.

Capture roughly doubles the rows a job produces -- at ~22.5 turns a game, a
40,000-pair job is 1.8 million positions -- so it is off by default. See
[Position Capture From Games](#position-capture-from-games) for the full design.

#### Naming: the request is an opening rack, the result is a position analysis

`opening_rack_requests` is named for what it asks for -- a set of opening racks
-- while the result tables stay `position_analysis_*`, because what comes back
from analyzing one is a position analysis. There is no general position-analysis
job, so nothing else writes a request here.

The record deliberately does **not** store the best move, its score or its
equity. Those are the rank 1 row of `position_analysis_moves`, and a second copy
is only something to keep consistent. `num_moves` stays, since the stored moves
are truncated to `num_plays_recorded` and cannot tell you how many were ranked.
Every read of a best move goes through its record — the results listing joins
`record_id` and filters `rank = 1`, and a rack lookup reads a record's whole
ranked list — so the index that serves them is on `(record_id, rank)`. There is
deliberately no job-wide index on `(task_id) WHERE rank = 1`: one existed for a
dashboard aggregate over every best move of a job, that aggregate is gone (see
[Job Detail Page — By Job Type](#job-detail-page--by-job-type)), and it cost
maintenance on every move insert into a table that runs to tens of millions of
rows.

#### How much of an analysis is kept

A worker reports the leading `num_plays_recorded` moves per rack and states, in
`num_moves`, how many it ranked to get them. Those are **deliberately different
numbers** (the player config's `num_plays` decides the second): a simmer may
need to rank hundreds of candidates to order the top few correctly, while
storing hundreds of rows for each of millions of racks is not something the
database should be asked to do. `position_analysis_records.num_moves` keeps the
count, so the discarded tail is still visible.

The cap is applied on the **client**, not only on the server that stores the
result. Reporting everything and truncating on receipt sends bytes nobody
stores, and it does so per rack across a batch of up to 10,000 — enough, with a
recorder that keeps every candidate, to put an ordinary job's submission past
`MAX_RESULT_BYTES` and have it refused. `num_moves` is what makes the cap free
of information loss. A client that omits the field reported everything it
ranked, which is what builds before the field was added did, so the list's own
length stands in for it.

**An opening-rack job cannot use a static `best` recorder to rank moves.** `-r best`
is `MOVE_RECORD_BEST`: move generation keeps the single top play and discards
the rest, so a static player's rack comes back with one move however many
`num_plays_recorded` asks for. (A simulating player's candidates are every
play up to `num_plays`, whatever its recorder, as in autoplay. MAGPIE's
opening-rack executor once generated them with the recorder, so a `best`
simmer — legal at `num_plays_recorded` 1 — had one candidate, and reported
the static top play as the simulation's; twenty-eighth audit.) Nothing downstream
notices: the racks are analysed, the results accepted, `racks_analyzed` climbs,
and the corpus quietly holds a fraction of the analysis the job was configured
for. Job creation therefore refuses a static player (`num_plies` 0) with
`recorder_type = 'best'` and a `num_plays_recorded` above 1, and names the
remedy. A simulating one is accepted: it ranks as many moves as it records.
`best` with `num_plays_recorded` of 1 stays legal for any player, because "the
best opening play for every rack" is a real job. The rule is scoped to opening
racks: a `games` job's players go through autoplay, where a simmer's candidate
list is sized by `num_plays` rather than by the move recorder, and `best` is
right there.

The same shortfall has two other sources. `num_plays` sizes the move list for
every player, static or simulating, so a config whose `num_plays` is below its
`num_plays_recorded` is refused for an opening-rack job too (twenty-ninth
audit). And a static `equity` recorder keeps only the moves within
`movegen_margin` of the best, which can be fewer than it reports, so `all` is
the recorder to rank with. A static `score` player's reported equity is the
move's score: MAGPIE sets it so under a score sort.

`position_analysis_plies` is populated only for **simming** player configs. A static player produces no per-ply statistics, so for the common case the table stays empty rather than filling with placeholder rows.

### Statistical Result Evaluation

For game and game-pair jobs, results can be evaluated with the **match test**, which answers one question: is one player better than the other, at a stated confidence? The test is **optional and off by default** (`test_enabled`): a job without it plays `max_games` (or `max_pairs`) and completes, with no verdict stored — its `job.completed` audit row says `reached_target` — and its result is the match score (wins, losses and draws, score and win rate, the players' average scores and spread). A job that only wants the games played should not be stopped early by a test nobody asked for, nor show an interval nobody acts on. Its `min_*` and `confidence_pct` are stored at their defaults (a floor of 0, 95%) and read by nothing, and the API refuses either sent without `test_enabled: true` rather than dropping it: the job would otherwise play to its cap with no test, and nothing would say so until it finished. A job with the test has two finish conditions:

1. **A decision**: the test is evaluated as results are submitted (debounced, below), and acted on once `min_games` (or `min_pairs`) have been completed. The job auto-completes when a check finds player 1's score interval wholly above an even score (`player1_better`) or wholly below it (`player2_better`).
2. **Hard cap**: the job auto-completes when `max_games` (or `max_pairs`) is reached, regardless of the test, which is then `inconclusive`. **No task is generated past the cap**: once every game (or pair) up to it has been handed out, a claim finds nothing new to generate, tasks whose claims lapse are still re-dispatched, and the job completes as their results land. Generating beyond the cap handed out work that could not change the verdict — as many batches as workers asked before the debounced check next ran.

The test is evaluated inline on the submission path (no background sweep), and
**debounced**: every eighth submission for a job (`TEST_CHECK_EVERY`), plus
unconditionally whenever that job has nothing left in flight. A submission
checks only while its job is active, so one more trigger covers the results
that land with nobody to check them — the last results of a job deactivated
while they were out, or a last check that failed: a claim that finds a games,
pairs or opening-rack job with nothing to hand out and no claim in flight
checks it, off the claim request, at most every ten seconds per job and at
once after an activation. Without it such a job, reactivated, stayed active at
its allocation for good (thirty-second audit). The server flips the job to
`completed` automatically when either condition is met, and **stores the
verdict it completed on** — status, player 1's interval at that moment (lower
and upper) and units (`jobs.test_decided_*`) — with the completion. The claims
in flight at that moment are still played and accepted, and the live figures
are recomputed from every accepted result, so the interval can go on moving
afterwards, even back around an even score; without the stored verdict a job
that decided could show "running" on its own page with no record of the
decision that stopped it. The page shows the stored verdict as the result and
the live interval beside it. A purge clears it.

The debounce trades *when* a job notices it is finished for the cost of
noticing, and nothing else — the check still reads `game_results`, so a
debounced check is late, never wrong. That is what separates it from replacing
the read with a counter, which would make a drifted counter able to stop a job
early (see [What these reads cost](#what-these-reads-cost-measured)). The cost
is bounded at seven extra tasks, and the first several of those are free: when
the test decides, the job flips to `completed`, but every task already claimed
across the fleet is still played and still accepted, because the submit path
validates the claim rather than the job's status. A bound at or below the number
of tasks typically in flight therefore wastes nothing that was not already going
to be wasted. Checking this often is safe only because the interval stays valid
however often it is checked (below).

The unconditional check when nothing is in flight is a correctness cover rather
than an optimisation. The check is triggered *by* submissions, so a job whose
contributors all stop between checks would not be evaluated again until work
resumed — which, for a job that has already reached its stopping point, means
never: it would sit `active` holding its allocation.

#### Why a match test, and not an SPRT

Until October 2026 the test was an SPRT between two Elo hypotheses, H0 at
`elo_low` and H1 at `elo_high` (−10 and +10 by default), with error rates α
and β. Those error rates held only *at* the two hypothesised values, and its
verdict said which of the two the data favoured, not whether player 1 was
better. Two problems followed:

- **Equal players got a winner about half the time.** By symmetry, two equal
  players "passed" about as often as they "failed", and the badge said
  "passed (H1 accepted)".
- **It asked for settings the question doesn't have.** An admin had to pick an
  Elo margin, α and β, none of which appear in "is A better, at 95%
  confidence?". Its normal-approximation LLR also overstated |LLR| when nearly
  every pair split (KL-87), which is common for pairs that play identically.

The match test answers the question directly: it keeps a confidence interval
for player 1's score that stays valid however often it is checked, and stops
as soon as the interval excludes an even score. Its only settings are the
confidence and the floor and cap the SPRT already had.

#### How the interval is computed

**A games job's batch is even.** MAGPIE gives player 1 the first move in a run's
first game and alternates from there, and every task is a run of its own. At a
batch of 1 — the default until the thirty-second audit's pass 18 — player 1
moved first in every game of the job, and the SPRT then in use passed two
identical players on the first move alone (+42 Elo measured, an H1 of +10
accepted after 358 games); any odd batch leans the same way, by less. An even
batch gives each player the first move in half of every task's games. Game
pairs are balanced already: each pair gives each side the first move once.
(The schema's column default is 2, and the API always sets it. A `games` job
made before then with an odd batch is biased, KL-87.)

What differs between the two job types is **what one observation is**, and that
choice is the whole statistical content of the test:

| Job type | Unit | Score | n |
|---|---|---|---|
| `games` | one game | 1 / 0.5 / 0 | games played |
| `game_pairs` | one **pair** | `i / 4` for pentanomial bucket `i` | pairs played |

From the counts the server already stores (`stats/outcomes.rs`: the per-game
`Tally` and the `Pentanomial`) it takes n, the mean score μ̂ and the sample
variance σ̂² of the unit scores. For a plain `games` job the sample is
per-game, with `mean = (wins + 0.5·draws)/n` and `second_moment = (wins +
0.25·draws)/n`.

The interval is an **asymptotic confidence sequence** (Waudby-Smith, Arbour,
Sinha, Kennedy & Ramdas, *Time-uniform central limit theory and asymptotic
confidence sequences*, 2021): the Robbins normal-mixture boundary with the
sample's own variance (`stats/match_test.rs`). With α = 1 − confidence/100:

```
half_width(n) = sqrt( 2·(n·σ̂²·ρ² + 1) / (n²·ρ²) · ln( sqrt(n·σ̂²·ρ² + 1) / α ) )
ρ²            = (−2·ln α + ln(−2·ln α + 1)) / (n*·v)
n*            = sqrt(min_units · max_units)
v             = 1/4 for a game, 1/16 for a pair
interval      = [ μ̂ − half_width, μ̂ + half_width ], clipped to [0, 1]
```

Why this method, and the choices in it:

- **It holds under repeated looks.** The interval contains the true score at
  every n at once, with probability about the confidence, so checking it every
  eighth submission and stopping the moment it decides is valid. Stopping the
  first time an ordinary fixed-n 95% interval excluded ½ would name a winner
  between equal players far more often than one time in twenty; with this one
  the chance of ever naming a winner between them is at most about α, split
  between the two sides. Simulated, checked after every batch of 50 pairs from
  500 to 10,000, equal players got a winner in at most α + 2% of runs, and a
  player scoring 53.5% per game was found better in at least
  90% of them (TESTING.md, `U-STATS-3b`).
- **It needs only stored sums.** n, μ̂ and σ̂² come from the stored counts at
  any moment, in any order, so it is recomputed on every read, as the SPRT
  was, with no column per result and no per-unit history. An exact,
  nonasymptotic (betting) confidence sequence would need each unit's outcome
  in order, which batches do not keep.
- **Asymptotic means a floor.** It assumes enough units for the mean to be
  close to normal, so `min_units` is required with the test, at least 1 and at
  most the cap, and nothing is acted on before it.
- **Tuned at n\*, for a planning variance v.** n\* = √(min_units · max_units),
  the geometric mean of the earliest point the test acts and its cap, is where
  the boundary is tightest; v is ¼ for a game (the most a score in [0, 1] can
  vary) and 1/16 for a pair (about what a paired match's pair scores do). The
  paper's ρ is tuned for a variance of 1: used as it is on scores of variance
  near 1/20, it put the tightest point twenty times past n\*, and early in a
  job 90 wins in 100 games decided nothing. The boundary is flat around its
  tightest point — at 95%, within a tenth of the narrowest any tuning gives,
  from a fifth of n\* to twenty times it — so neither guess needs to be close.
  Both are fixed before any game is played, as the guarantee requires; the
  variance *inside* the boundary is the sample's own.
- **No observed variance is still an interval.** If every pair splits, σ̂² is
  0 and the half-width is `sqrt(2·ln(1/α) / (n²·ρ²))`, which shrinks with n
  but never reaches zero, so a run of identical pairs cannot decide anything
  on its own. The SPRT's overstatement there (KL-87) is gone with it.
- **The confidence is strictly between 50% and 100%.** At 100% ln α is
  infinite and the interval never closes; at half or less it is no test.

**The decision**, made only once n ≥ `min_units`: a lower bound above ½ is
`player1_better`; an upper bound below ½ is `player2_better`; n ≥ `max_units`
with neither is `inconclusive`, and the interval then says how large a
difference the games rule out. Below `min_units` the interval is computed and
reported but never acted on — except that the hard cap still applies, so a job
whose `max_units` is below its `min_units` ends inconclusive rather than
running for ever. Before anything is played the mean is ½ and the interval
every score. The status is one of `running`, `player1_better`,
`player2_better`, `inconclusive`.

**No rating scale.** The result is player 1's per-game score and its interval
(a pair's `i/4` is a per-game score, so both job types read alike), and
nothing on a rating scale is derived from it: the Elo figures the API once
carried beside them are gone, so no number on a job's page can be mistaken for
a pool's rating, whose scale is WESPA's ([The scale is
WESPA's](#the-scale-is-wespas)). The job page's Significance Test card says it in
one sentence ("static-equity scores 53.1% per game (95% interval 51.2% to
55.0%).") over a bar of the interval around 50%.

#### The pentanomial, and why pairs are the unit

A `game_pairs` task plays each seed twice with the players swapped. The two
games of a pair **share a seed**, so they are not independent of each other —
counting them as two observations overstates how much evidence there is. The
pair is the independent unit, and its outcome is one of five: player 1 lost
both, lost one and drew one, split, won one and drew one, or won both. MAGPIE
reports those five counts directly (see [MAGPIE reports the
pentanomial](#magpie-reports-the-pentanomial)), indexed by player 1's half-point
score across the pair, and the sample's mean and variance are taken over pair
scores of `i/4`. That puts the mean on the same per-game scale as a game's
score while `n` honestly counts pairs.

**Every completed pair is in the sample, including the pairs whose two games
played identically.** Those are guaranteed 1-1 ties: they score exactly 0.5,
they contribute nothing to the variance, and they pull the variance *down*.
That is precisely where paired play's variance reduction comes from — not from
discarding them.

Testing only the pairs that *did* diverge is the trap, and it is not a small
one. Filtering does not flip the direction, since the identical pairs sit
exactly at even and both views land on the same side of it, but it destroys the
magnitude and with it the test's purpose. Take 20,000 games split 9,950-10,050,
of which only 100 diverged and one player took 99 of them:

| Sample | Score rate | Below even |
|---|---|---|
| Every pair (10,000 of them) | 0.4975 | **0.25** percentage points |
| The 50 divergent pairs alone | 0.01 | **49** percentage points |

Same games. A test fed the second number decides almost immediately, on a
hundredth of the evidence, whatever confidence it is asked for, and reports
every difference as decisive. The divergent counts are still collected and
still shown, as a **diagnostic** of how often two configs differ at all.
Nothing is tested on them.

**The sample size and the progress count are the same number.** `min_pairs` and
`max_pairs` gate on pairs played, and the pentanomial's sample is pairs played,
so the two cannot drift apart. (They could, and did, when the SPRT's LLR ran
over a filtered subset of games while the gates counted pairs — a job would
then either end early or never end.)

### Ratings

Ratings are **siloed from job control flow entirely**. Nothing in the rating
system is read while dispatching, claiming, validating or completing a task, and
no job decision reads a rating. The coupling runs one way: a fit reads finished
`game_results` and writes a snapshot. The match test stays where it belongs,
on the job config tables — it is a per-job **stopping rule**, not a
measurement, and the score it shows is one comparison's rather than anyone's
rating.

Everything below lives in four `rating_*` tables and one module. See
the [Schema](#schema-1) ("Ratings") for the tables and [Two things the fit has to
handle honestly](#two-things-the-fit-has-to-handle-honestly) for the fit.

#### In one place: when a rating is computed, and where it is shown

A rating is never updated by a result landing. It is recomputed for a whole
pool, from scratch, by `ratings::fit_and_store`, on exactly four triggers:

| Trigger | When | Who | What is written |
|---|---|---|---|
| **evidence** | Every two minutes, from a sweep started by `main.rs` (`ratings::recompute_stale`), for each pool whose eligible jobs' `games_completed` sum, or whose members, differ from its last run's (`evidence_games`) | The server, unattended | A new `rating_runs` row with a `player_config_ratings` row per member and a `rating_run_residuals` row per head-to-head — or nothing, if the evidence has not moved |
| **membership** | Immediately, inside the request, when an admin adds or removes a member (`POST`/`DELETE /api/admin/rating-pools/:id/members`) | An admin | The same, unconditionally |
| **manual** | Immediately, on `POST /api/admin/rating-pools/:id/recompute` | An admin | The same, unconditionally |
| **anchor** | Immediately, in the same transaction as the change, when an admin moves the anchor or its rating (`PATCH /api/admin/rating-pools/:id`). The sweep would never notice: neither the evidence nor the members moved | An admin | The same, unconditionally |

So a result submitted now is in a rating within two minutes, and a job that
finishes at 03:00 is rated by 03:02 without anyone doing anything. Nothing on
the claim or submit path reads or writes a rating, and no job decision depends
on one; the match test is a separate, per-job stopping rule.

Ratings are **displayed on the ratings pages and nowhere else**:

- `/ratings` (`GET /api/rating-pools`) — every pool with its conditions,
  member count and the time of its last fit.
- `/ratings/[id]` (`GET /api/rating-pools/:id`) — the pool's **newest run**:
  each member's rating with its standard error as a dot plot (and, for an
  admin, a table), the
  run's provenance (trigger, iterations, convergence, evidence consumed), and
  the [cross table](#the-cross-table) of every head-to-head, whose hover shows
  where the fit disagrees with the games. A config
  with no path of games to the anchor is listed as unrated rather than drawn.
  Admin membership controls appear inline here for admins, with the anchor
  (config and rating) and a Delete button. They work from the pool's members,
  which the detail serves apart from the fit's ratings: a member the latest fit
  has not rated -- added since, or its refit failed -- is listed as not yet
  rated, with a Remove button, and is not offered under Add (thirty-third
  audit, pass 4; the page had derived membership from the ratings).
- **No history chart.** The page once drew each config's rating across the
  stored runs; it was removed as more noise than signal on a page read for the
  latest fit. `GET /api/rating-pools/:id/history` still serves those series to
  API callers. Long config names on the dot plot are shortened together -- the
  segments a family of configs shares dropped, the rest shortened in the
  middle, and labels still alike widened where they differ -- so no two read
  the same (the full name is each one's title).

A job's own pages (`/jobs/[id]`) show its match test's verdict, win rate and
pentanomial, and **no rating**: a rating belongs to a pool, not to a job, and a
`games` job (unpaired) feeds no pool at all. Runs are kept in full for a month
and then thinned to one a day (see below), so the history is bounded while the
run-by-run diff stays available for as long as it is useful.

#### Nothing is incremental, and nothing is frozen

Elo and Glicko are **sequential filters**, built for humans whose strength
drifts over time: they nudge a rating per result because history is stale and
cannot be refit. Every design choice in Glicko-2 — the volatility parameter, RD
growth during inactivity, rating periods — is machinery for tracking a moving
target.

A player config is a frozen set of MAGPIE flags, immutable once any job
references it. **Its strength is a constant.** There is no drift to track, so
the filter buys nothing while costing path dependence, and the question "should
a bot's rating be fixed once it is established?" has no good answer because
*establishing* one incrementally is the wrong move to begin with.

#### Why not WESPA's Glicko

WESPA rates people with Glicko, and a word game's own system is the obvious one
to borrow. It was weighed and is **not implemented**. Beyond the drift
above, which a fixed config does not have:

- **Order matters.** Glicko updates one rating period at a time, so the same
  results in a different order give different ratings. Results arrive in
  whatever order volunteers finish them, and purges and reruns would reshuffle
  ratings. Bradley-Terry uses all the evidence at once and does not care about
  order.
- **The uncertainty never shrinks.** WESPA's RD floor (50 to 75 by band) is
  there to keep human ratings responsive. For a bot with 100,000 games it means
  the rating keeps moving as if it were uncertain by ±50, where Bradley-Terry's
  standard error shrinks with the evidence.
- **Periods would be arbitrary.** There are no tournaments. Any choice of
  period (a day, a batch, a job) is arbitrary and changes the numbers.
- **The calibration is for humans.** Newcomer seeding (five virtual games at
  1500), an initial RD of 300 and the RD bands are calibrated to WESPA's human
  population. Bot ratings would land in bands that depend on where the anchor
  sits.
- **Game pairs lose their advantage.** Glicko's updates are per game; feeding
  it pairs throws away the variance reduction game pairs exist for.

The same reasoning is in `stats/bradley_terry.rs`'s module doc. What is taken
from WESPA is its scale, below.

#### The scale is WESPA's

A rating gap Δ predicts a score of `1 / (1 + e^(−Δ/250))`: 250 points per
logit (`bradley_terry::POINTS_PER_LOGIT`), WESPA's k. WESPA's Glicko predicts
`1 / (1 + e^(−gΔ/250))` between two players, with g about 0.95 to 0.99 between
established ones, so a gap in birdtest predicts the win % the same gap does
between two established WESPA players, to within about 1 to 5%: 100 points is
59.9%, and a 75% score is 250·ln 3 ≈ 275 points. (Until October 2026 the fit
used Elo's 400/ln 10 ≈ 174 points per logit, on which 100 points was 64.0%.)
Standard errors, the residuals' predicted scores and the convergence test all
use the same constant; the priors are in log-strength units and do not.

What the scale cannot match is the **absolute level**. Bot-against-bot results
say nothing about strength against people, so a number is only as meaningful
as the anchor's: the default 2000 reads as a strong club player's, and an admin
can pin a pool's anchor anywhere. The ratings page says so under its table,
and labels the column "Rating (WESPA scale)".

What birdtest actually has is a static tournament: N configs and a matrix of
pairwise results. The right tool is a **batch maximum-likelihood fit** over the
whole matrix at once — Bradley-Terry, solved by Newton's method,
anchored on one config at a fixed rating. Every rating is a joint solution to the
entire graph, recomputed from scratch whenever the pool or the evidence changes.
Three properties follow, and they are the reasons for the choice:

- **Order independence.** Ratings do not depend on the sequence results arrived
  in.
- **Add and remove are free.** Changing membership is a refit, not a surgical
  undo of one player's historical updates. Correct by construction.
- **One anchor is enough.** Ratings are identifiable only up to an additive
  constant, so exactly one config is pinned — chosen explicitly per pool
  (`rating_pools.anchor_player_config_id`, conventionally a static bot at 2000).
  No other rating is ever frozen.

The fit is Newton's method on the log-strengths, with a backtracking line
search, no step moving any config more than 2,000 points, a small step
taken whole, and damping if rounding makes the curvature fail to factor. The
objective, the likelihood plus the prior below, is strictly concave, so it has
one answer; each step solves for every config at once through the full
curvature, which is what a group of configs that moves together needs. (Without the step bound, clean sweeps that
contradict the rest of a pool could throw one config thousands of points away in
a single step, where every head-to-head it has saturates and the fit stalls; the
thirty-second audit's adversarial check found that.) Minorization-maximization, which the fit used until the thirty-second
audit, updated one config at a time and crept toward such a group's answer,
stopping at its iteration cap short of it (KL-74). A step is a Cholesky
factorisation of an `n × n` matrix, so a hundred-member pool fits in
milliseconds, which is what makes "refit everything on every change"
affordable rather than aspirational.

#### Non-transitivity is displayed, not solved

An adversarial config can beat some opponents and lose to others in a cycle: A
beats B, B beats C, C beats A. **No scalar rating system can represent that** —
it is not a flaw in Bradley-Terry but a fact about collapsing a tournament graph
onto one axis. Modelling it properly (Blade-Chest, disc decomposition) costs the
single number a leaderboard is made of, so birdtest does not attempt it.

What the batch fit buys is that the failure becomes **visible and localized**.
The fit returns the best scalar approximation, and the *residuals* — actual
score versus model-predicted score for each head-to-head — say exactly where the
model is lying. A rock-paper-scissors triangle shows up as three large,
sign-flipped residuals and as ratings that collapse toward each other. An
incremental filter cannot show this at all; it just oscillates quietly. The
ratings page therefore carries the scalar rating as the headline and the cross
table beside it -- each cell's hover gives what the ratings predict, and a cell
they predict badly is amber -- and says so when the residuals are large enough
that the ranking should not be read as one: three or more head-to-heads at least five
points off *and* at least three standard errors from zero on the pairs behind
them (`charts/residuals.ts`). Without the second condition a young pool — a few
pairs per head-to-head — showed the warning on sampling noise alone.

#### The cross table

Under the ratings, the pool page lays every head-to-head out as a cross table:
an n × n matrix of the configs the latest fit rated, best first (then any with
no chain of games to the anchor), with each config's rating in the rightmost
column. A cell is the row config's record against the column's, from the
row's side:

- **Win %**, (W + ½D) / games — the pentanomial's half-points over its games,
  the score the fit itself reads.
- **± its standard error**, from the pairs, not the games. A pair is the
  independent unit and scores `k/4` for bucket `k`, the mean of its two games'
  scores and so already per game; over the head-to-head's summed pentanomial
  (`N` pairs, mean `m`), `s² = Σ c_k·(k/4)² / N − m²` and the error is
  `√(s²/N)`, shown in percentage points. Counting the games as independent
  (`√(m(1−m)/2N)`) would undo what pairing buys: pairs that played identically
  all score ½ and narrow the error, as they should.
- **Average spread**, `Σ games·(p1_mean − p2_mean) / Σ games` over the same
  `game_results` rows, from the row's side.

The evidence is exactly the fit's: the same pool-scoped `game_pairs` jobs
between two members, summed per pair of configs whichever seats them (a job
with the configs the other way round is flipped, bucket `k` to `4 − k` and the
spread negated). A cell's mirror across the diagonal is the same games: 100 −
the win %, the same error, −the spread. The residual is folded into each
cell's hover — what the ratings predict, and how far off — and a cell they
predict badly on enough pairs that it is not chance is amber; three or more
raise the warning above.

**Stored per run, not computed on request.** The cells come from the grouped
scan `build_matrix` already makes for the fit (each job's games and
games-weighted spread beside its pentanomial), so a fit stores them in
`rating_run_residuals` with its ratings at no extra read: the head-to-head's
`stderr` and `spread` beside its `actual` and `predicted`, once per
head-to-head, mirrored by the API (`ratings::both_sides`). Computing them on
request would be that scan again on every view of a public page — the cost
the residuals were moved into the run to escape — and could show evidence that
has moved on from the ratings beside it. On a phone the table scrolls
sideways inside its card, the config names held in a sticky first column.

#### What counts as evidence

Only `game_pairs` jobs, and only those matching the pool's scope, with **both**
configs in the pool.

Plain `games` jobs are excluded deliberately. `-gp` plays both orderings of every
seed, so a pair is **side-balanced by construction**; an unpaired job is not, and
going first in a word game is worth real rating points. Pooling unbalanced
results would bias every rating toward whoever happened to start more often. Including them
would require an explicit side-advantage term in the model, which is not worth
the complexity while every rating-relevant job is paired anyway.

A pool is scoped by `(variant, letter distribution, layout)` because a rating is
only meaningful against fixed conditions: pooling a wordsmog job with a classic
one, or two different letter distributions, produces a number describing no game
anyone played. Lexicon is deliberately *not* part of the scope — it lives on the
player config, and two configs on different lexicons playing each other is a
meaningful comparison.

#### Membership is an admin decision

Not every player config belongs in a rating. An admin adds and removes configs
from a pool, and **either change refits the whole pool**, because a config's
games are evidence for everyone else's rating too. Removing a config removes its
games as evidence, which moves every other number — that is correct, not a bug,
and it is why this cannot be a targeted per-row delete. Removal is soft: the
membership row goes, the results stay in `game_results`, so re-adding costs
nothing but a recompute.

The anchor cannot be removed while it is the anchor; the pool would lose its
scale. It can be moved: `PATCH /api/admin/rating-pools/:id` sets another config
(added as a member if it is not one) or another rating, and refits in the same
transaction, so the page never shows the new anchor beside ratings on the old
scale. Earlier runs keep the scale they were fitted on, so the history steps at
the change — which is what happened. The change, a removal's anchor check and a
pool's delete all take the pool's fit lock, so none of them interleaves with a
fit or with each other: a removal that passed its check cannot land after an
anchor change made its config the anchor.

A pool can be deleted (`DELETE /api/admin/rating-pools/:id`): its members, runs,
ratings and residuals cascade, and its census (`name`, `members`, `runs`) is
logged first. The games stay with their jobs — a pool is only a view over them
— and its configs and input data are free to be deleted after it. A sweep that
listed the pool before the delete skips it without logging a failure.

#### Two things the fit has to handle honestly

- **Separation.** A config that has never lost sends the unregularised maximum
  likelihood to infinity, and that is not an edge case — it is what a strong new
  bot's first job looks like — and undefined for a config with no games. So
  every config, the anchor included, plays virtual drawn games against a
  virtual config at the pool's *centre*, the plain mean of every rating, on a
  logistic twice as wide as real games' (500 points a logit against 250): two
  for a config with no games, fading with its real ones as `2 / (1 + g/200)` but
  never below a fifth, since the prior is for the barely played. That
  keeps every rating finite and the objective strictly concave: one answer,
  continuous in the scores, and conceding a point lowers a config against its
  opponent (its own rating, too, all but about once in 500 fuzzed cases, by
  under a point and a half). The
  pull is toward the pool's centre, not the anchor, so a field or group far
  from the anchor is not dragged back to it; and it joins configs only through
  the centre, never to each other. Its cost is a pull that levels off at a
  constant per config, however far a config is from the centre, and adds up
  along a thin chain: a 12-rung ladder, each rung 144 points above the last and
  played only against it over 100 pairs, has its top some 60 points low, about a
  third of its error (KL-79). The wider scale and the fade are what keep that
  small: left whole on well played configs, the pulls held a strong tier joined
  to the rest by one job some 500 points low, nearly four errors. The
  thirty-second audit tried six other priors, and each pulled some shape of
  pool where the evidence was fine: two draws per config against the anchor, a field or thinly linked group
  far from it (a 30-member group 290 points low, KL-74); two per config spread
  over its opponents, a config over a gauntlet of lightly played ones; draws
  only where the maximum likelihood diverges, a 290-point jump when a newcomer
  conceded a quarter point; Firth's penalty, which is not concave, two answers
  for a config between far-apart opponents; the centre prior left whole, a
  thinly joined strong tier held nearly four errors low; and a centre fitted as
  a strength of its own, which followed a newcomer's unfaded virtual games in a
  mature pool, leaving its first job barely shrunk.
- **Connectivity.** If two configs only ever played each other and neither
  connects to the anchor's component, their ratings are unidentifiable — the fit
  would otherwise return a confident number produced entirely by the prior. Each
  rating carries `connected_to_anchor`, and the page shows an unconnected config
  as **unrated** rather than as a plausible-looking 1500.

Standard errors come from the inverse of the full Fisher information over the
anchor's component, counting each paired game as one trial, so a config's
error includes the uncertainty of every link between it and the anchor: a
group joined to the anchor by one 300-pair job carries that job's ±40 points,
where the diagonal the fit used until the thirty-second audit showed ±3 (KL-74).
Each error is also widened by how far the prior holds that config from where
its games alone would put it — one Newton step on the games from the answer,
`I⁻¹ · ∇prior`, taken one and a half times since one step underestimates a
pull that has saturated — added to its variance. The prior's pulls add up
across a group joined to the rest thinly, which no weighting of it removes
(KL-79): two tiers of lightly played configs 860 to 1,150 points apart, joined by
one small job, put the upper one about two of the games' errors low, its 95%
interval covering the truth 30 to 65% of the time; with the pull in its error,
95 to 100%. Two further approximations remain, and both widen the errors. Counting a pair (two games,
scored in quarters) as one trial widens them by √(2/(1+ρ)), where ρ is the
correlation between a pair's two games: √2 if they are independent, more when
pairing works (ρ < 0), less when a config wins both halves on the same racks
(ρ > 0), and never below 1. And the prior's virtual games are in the fit but
not in the information, which widens them most for the barely-played. They are
good enough for the distinction the page needs to draw, 1700 ± 15 against
1700 ± 200.

#### When a fit runs

On membership change and on demand, immediately; on new evidence, from a periodic
sweep (two minutes) rather than a hook on result submission. A fit is global to a
pool, an active job submits results far faster than any rating needs to move, and
— unlike the match test — nothing blocks on the answer. The sweep first compares the sum
of the pool's eligible jobs' `games_completed` (each job's running total of games
played, kept by the submission that stores the result) against the last
run's `evidence_games`, and its current members against the configs that run
rated — so a membership change whose own refit never ran (the request dropped
after the membership committed) is repaired too, even when the config added or
removed had no pairs. Only when either moved does it build the evidence matrix,
compare its pair count against the last run's `pairs_used`, and fit from that
same read. (It built the matrix every time to find out, for every pool every two
minutes: seconds each at millions of results.) No dirty flag is needed anywhere.

**A fit holds the pool's lock while it runs** (`pg_advisory_xact_lock`, per
pool, for the fitting transaction). It is a read-then-write over state an admin
can change underneath it — it reads membership and evidence, then writes a run
stamped `clock_timestamp()` under the lock (the column's `now()` default is the
transaction's start, before the lock, which misordered two fits exactly the way
the lock exists to prevent) — so two at once interleave, and the fit that *started* first
can commit last. The newest `rating_runs` row is what the ratings page shows,
so the visible symptom is a config removed from a pool coming straight back in
the fit: the sweep had already read the old membership when the removal landed.
It would correct itself at the next sweep, two minutes later, having shown
something untrue in between. The lock is per pool, so pools never wait on each
other.

**One pool's failure does not stop the others.** A fit can fail on state an
admin can reach — a pool whose anchor is no longer a member is the obvious one
— and propagating that ended the whole sweep at the first such pool, so every
pool ordered after it silently stopped being refit for as long as the
misconfiguration lasted. Each pool is logged and skipped instead.

Runs are **snapshotted, not mutated**: one `rating_runs` row per fit with its
provenance (trigger, iterations, convergence, evidence consumed) and one
`player_config_ratings` row per config per run. That is what makes "why did this
rating change?" answerable and gives the ratings page a time axis for free. A run
that did not converge is stored and displayed, flagged — hiding it would leave
the page silently stale.

**Runs are kept in full for a month, then thinned to one a day.** A pool with
an active job is refit every two minutes: ~720 runs a day, each with a rating
row per member and a residual row per head-to-head — ~150,000 rows a day for a
pool of twenty, for the life of the pool. Inside the month the run-by-run diff
is what answers "why did this rating change?". Past it, an hourly sweep
(`ratings::thin_old_runs`) keeps each UTC day's last run and the pool's first
and deletes the rest, their ratings and residuals cascading. A day is the
resolution the history endpoint serves at anyway — it thins to 500 points over
the pool's whole life — so the history keeps its shape and its ends; deleting
everything past the window would have started its past at the window's edge.
The newest run, the one the page shows, is the last of its day and so always
survives, however long the pool has been quiet. Runs go in batches of a
thousand so each transaction is bounded, and no fit lock is taken: a fit
inserts a run stamped `now()`, never inside the window.

---

### User Accounts

Users can create an account to track their contributions. Account creation requires:

- Username
- Password (minimum strength enforced at registration time)
- Email address (used for account confirmation and password reset)

A confirmation code is sent to the email address on registration. Users can generate one or more API keys from their account, which are used to authenticate task submissions.

API keys are stored as hashes (never raw values) in the database. The raw key is shown to the user exactly once at generation time. Users may hold up to **100 API keys**, and a request to create the 101st is refused rather than silently evicting one. The count and the insert run under the account's row lock: the limit lives in the application rather than the schema, and counted and inserted as two bare statements, requests arriving together each read the same count. Each key can be independently marked **active** or **inactive** — only active keys are accepted for worker authentication. This lets contributors rotate or temporarily disable a key without deleting it. A worker request authenticated with a key stamps its `last_used_at`, at most once a minute per key, so a contributor can tell which of their keys is actually in use before revoking one.

**v1 account scope**: The sole v1 purpose of a user account is to generate an API token, which attributes task submissions to that account instead of an anonymous UUID. No other feature is gated behind registration. Anonymous workers can complete tasks fully, with no account or API token required.

#### Account Creation Flow

1. User fills out the registration form (`/register`) with username, email, and password.
2. The server validates, and returns `400` with field-level errors listing **every** problem at once rather than the first:
   - Username is 3–32 characters (counted as characters, not bytes), trimmed, with no line break, control or invisible character (it is written into mail to the address's owner, which may be a stranger's — KL-34; a zero-width joiner or non-joiner may stand between two letters of a script written with them — Arabic, Syriac, NKo, Mongolian, the Brahmic scripts — or inside an emoji sequence, and a variation selector after a pictograph or ideograph or on a keycap), and unique whatever its case (a
     unique index on `lower(username)`): "Josh" and "josh" side by side on the
     public lists is an impersonation.
   - Email is one bare address — `local@domain.tld`, printable ASCII, none of
     `<>,;:"()[]\` and no spaces — trimmed and lowercased, so neither case nor a
     display name (`x <victim@example.com>`) nor a list is a way to register, or
     mail, the same inbox under another string.
   - Password scores at least **3** on zxcvbn, with the username and email passed
     in as context so a password derived from either is rejected. Scored
     server-side only. The form shows a rough guide from length and character
     classes, which is not zxcvbn and can disagree with it either way (a
     keyboard walk it calls strong is refused; a long passphrase it calls good
     is taken); the server's refusal says only that the password is too weak,
     not why.

   The form checks the length and address rules itself first, as the server
   words them (`frontend/src/lib/accountRules.ts`), and is `novalidate`: the
   browser's own checks showed a popup where the server's errors are red text
   under the field, and took `a@b`, which the server refuses. The server still
   checks everything. The sign-in and password-reset forms do the same.

   A **taken username** returns `409` naming it. The user has to choose another
   one to get anywhere, and `GET /api/users` publishes the whole list anyway, so
   there is nothing here to protect.

   A **taken email** does not. Saying "that address is already registered" would
   make this endpoint an oracle for whether a given person has an account —
   something login and password reset both go out of their way not to reveal,
   and which registration should not undo. The caller gets byte-for-byte what a
   new registration gets, no account is created, and a notice goes to the
   address's owner telling them someone tried and pointing them at login and
   password reset (or, when the account holding the address has not confirmed
   it, at the confirmation link already sent). A real person who has forgotten
   they signed up still finds out; someone probing the address learns nothing.
   The notice is limited per address as well as per IP, and skipped rather than
   refused when limited: five an hour at most, which slows a flood but does not
   stop one -- with reset mail, one address can still be sent some 240 a day
   (KL-91).

   **An account that never confirmed gives up its username and address once its
   confirmation has expired.** The next registration naming either deletes it
   first. Held for ever, it was a dead end — it cannot sign in, reset its
   password or be sent a new code — and a way to squat anyone's address:
   register it first, and its owner could never sign up unless they clicked a
   stranger's link within the day.

   The password is hashed **before** this branch, not after, so both paths pay
   the same Argon2 cost. Returning an identical body and then answering tens of
   milliseconds sooner would hand back the answer through timing, which is the
   flaw the branch exists to avoid.
3. Password is hashed with Argon2 and stored. A confirmation code is generated, hashed with SHA-256, and stored in `email_confirmations` with a **24-hour** expiry. The raw code is emailed via SES.
4. The frontend redirects to `/register/check-email` — a static holding page instructing the user to check their inbox. No session is created yet.
5. The user clicks the confirmation link in the email, which lands on `/confirm-email?code=<raw-code>`. The page auto-submits the code to `POST /api/auth/confirm-email`.
6. The server hashes the submitted code and matches it against `email_confirmations`. On success, `email_confirmed_at` is set. The user is redirected to `/login`.

Email is confirmed before the first login. Logging in without a confirmed email returns `403` with a message indicating confirmation is required.

#### Login Flow

1. User submits the login form (`/login`) with username and password. Attempts are rate limited per client IP (10 a minute) and per username from everywhere (100 a minute), both checked before any Argon2 verify runs. The username limit was per username alone at 10 a minute, which let one address trying a wrong password every six seconds hold any account — an admin's, whose name is public — out of signing in; now a lockout takes a fleet of addresses, and the account-wide cap still bounds a distributed guesser. The account's bucket is keyed by the account the lookup found, not by the name as typed: the lookup matches with Postgres's `lower`, which does not agree with Rust's (`İ`), so a name keyed on Rust's lowering gave one account a bucket per spelling. A name that matches no account has a bucket of its own, so a 429 does not say which names exist.
2. The server looks up the user by username, whatever its case — the name is unique whatever its case, so this finds at most one account, and a contributor who registered "Josh" and types "josh" is not told their password is wrong. If not found, or the password does not verify, it returns `401` with an identical message for both, so the response body cannot be used to enumerate accounts.

   A known account still costs an Argon2 verify where an unknown one returns
   immediately, so the *timing* does distinguish them. That is a known and
   accepted gap: closing it means verifying the password against a fixed dummy
   hash on the miss path, which is worth doing if account enumeration ever
   matters more than it does today.
3. If email is unconfirmed, returns `403` with a prompt to check their inbox.
4. On success: a PASETO token is set as an `httpOnly`, `SameSite=Strict` cookie (`Secure` when configured), **and a CSRF cookie is set alongside it**, since every subsequent state-changing call needs the double-submit pair. The response body carries `{ username, is_admin }`.
5. The frontend redirects to `/account` (or to the page the user was trying to access before being redirected to login).

#### Password Reset Flow

1. User clicks "Forgot password?" on `/login` and is taken to `/reset-password`.
2. User enters their email address and submits. The server always returns `200` regardless of whether the email is registered — no account enumeration.
3. If the email matches a confirmed account, the server generates a reset token, hashes it with SHA-256, stores it in `password_reset_tokens` with a **30-minute** expiry, and emails the raw token link. Reset tokens are short-lived where confirmation codes are not: a reset link is a live credential for taking over an account, and a confirmation code is not.

   **The mail is sent off the request path.** Awaiting a provider round trip here
   and returning immediately for an unknown address would answer the question by
   timing — hundreds of milliseconds against a sub-millisecond index miss is not
   a subtle signal — which would undo the identical body the endpoint is careful
   to return. Spawning also keeps a slow or failing mail provider out of the
   caller's latency; a send that fails is logged and nothing else, since the
   caller was told the same thing either way.
4. The user clicks the link, landing on `/reset-password/confirm?token=<raw-token>`. The page shows a new-password form.
5. On submit, `POST /api/auth/reset-password/confirm` reads the token (hash match, not expired, not already used, its account not deleted) without a lock, so a wrong link costs nothing more; takes one of scoring's own turns (two, on the blocking pool — never sign-in's), then charges the link's own bucket (five scorings an hour, whoever sends them: a link refused a weak password still works), and scores the new password against the account and then hashes it on the password threads, both outside any transaction, so neither wait holds a lock; then locks the account, spends the token (checked again), stores the new hash, and **spends every other outstanding reset token for that account** so an earlier link cannot be replayed. It clears the caller's session cookie.

   The reset also **revokes every existing session**. Each session token carries the account's `session_generation`, and `CurrentUser` compares it with the `users` row it already reads on every request; the reset increments it, so every token minted before it — an attacker's included — stops working. `POST /api/auth/sign-out-everywhere` (the "Sign out everywhere" button on the account page) and account deletion increment it the same way.
6. The user is redirected to `/login` (with no message; signing in with the new password is the confirmation).

---

### Security

**CSRF**: CSRF protection applies to session-cookie-backed endpoints only (Auth API, Account API, Admin API). Worker endpoints (`/api/worker/*`) use bearer tokens or the `X-Worker-UUID` header — neither is sent automatically by browsers, so they are not susceptible to CSRF and are exempt.

**Rate limiting**: Endpoints that cost work and need no session — registration, login, password reset, redeeming confirmation and reset links, every worker endpoint, the live stream — are rate limited in their handlers (and the worker identity extractor), per client IP, per worker identity or per account as each needs, using the `governor` crate (token bucket algorithm). The public read pages are not metered; they run on a display pool of their own (`db::connect_read`) so they cannot starve claims and submissions. Rate-limited responses return `429 Too Many Requests` with a `Retry-After` header. The specific limits, and how a client's address is determined behind a proxy, are in [Rate limits](#rate-limits).

For v1, rate limit state is held in-memory (resets on process restart). A persistent backend can be added later for cross-instance coordination.

API keys and session tokens are handled as described in the Auth tech stack note below.

---

### Dashboard

The dashboard has two levels: a **job list page** and a **job detail page** per job.

Live updates are delivered via **Server-Sent Events (SSE)**. The client subscribes to a per-job SSE stream; the server pushes a new event after accepted task results — coalesced, at most one per `JOB_STATS_CACHE_SECONDS` (see [Live updates](#live-updates)). SSE is one-way (server → client) and sufficient since the client never needs to send data over the live connection.

#### Job List Page

Shows all jobs with: job type, status, allocation, and a completion counter (tasks completed / total, or games completed / max for on-demand jobs).

#### Job Detail Page — Common Elements (all job types)

- Job metadata: type, status, config summary, created by, created at.
- Completion progress.
- **Tasks that hit the time limit**: "N tasks hit the time limit — lower the batch size" beside the status, once any has; and for a job the server set aside for them, why, in place of "Paused" ([Task time limit](#task-time-limit)).
- **Per-worker contribution table**: worker identity (username, or an anonymous worker's pseudonym — never its UUID, which is its credential), tasks completed for this job and the compute time those claims were held (on a wide screen). Sorted by tasks completed descending.

#### Job Detail Page — By Job Type

**Games / Game pairs**

- A **Significance Test** card, for a job that runs the test (one without has none, and `games.test` is `null`): a status badge (`running`, `player 1 better`, `player 2 better`, `inconclusive`, or paused or undecided from the job's own status), one sentence — "static-equity scores 53.1% per game (95% interval 51.2% to 55.0%)." (nothing on a rating scale) and, once decided, which player is better at that confidence — a bar of the interval on a scale of player 1's score with 50% marked, and a folded explanation with the job's own confidence.
- The pentanomial (game pairs only), in the Significance Test card: the pair outcomes the test is computed from, as three rows (won both, won one and drew one, even) with a column per player and each count's share of the pairs (`lib/charts/pentanomial.ts` `PAIR_OUTCOMES`). Ratings are not here — they are pool-scoped and live on the [ratings page](#the-ratings-page).
- A **match score** card, after the settings and before the Significance Test card: a table with a column per player and a row each for wins, losses, draws, average score per game and average spread, the better figure of each row green and the worse red (`MatchScore.svelte`, `lib/matchScore.ts`). A pairs job has a second table beside it, **Games that diverged**, over only the games of the pairs whose two games did not play identically (`games.divergent`). It counts games for a pairs job too. For a job without a test it is the job's result. The averages are the batches' `p1_score_mean` / `p2_score_mean` weighted by their games, over every result of the job (one per task; `jobstats::SCORE_MEANS`).
- **Saved positions**, for a job with `capture_positions` set: one captured position at a time, drawn on the job's own board (`Board.svelte`) — its premium squares from the layout, the tiles with their letters and scores (a blank in lower case, scoring nothing), both racks and scores with the player to move marked, and the tiles the move before it placed outlined — beside its ranked moves — each with its win percentage, how often the simulation played it out (**Iters**), and its first two plies' average score and bingo percentage (P1-S, P1-BP, P2-S, P2-BP, P1 the reply) when it was simulated, and a solved move's depth as **Solved Plies** — and its CGP as text. A simulated position past turn 0 whose player inferred the opponent's leave first (its previous move not a pass) shows that inference under the moves: "Inferred from MOVE: N possible leaves, average equity E", and up to ten of the leaves the opponent most likely kept, with their draws and equity. **Random position** draws another (`GET /api/jobs/:id/positions/random`); a rack search shows that rack's positions newest first, one at a time with **Next** and **Previous** (`GET /api/jobs/:id/positions?rack=`). On a game-pairs job each position comes with its `partner`, the same turn of the pair's other game, and one of the two is drawn at a time ("Game 1 of the pair", "Game 2 of the pair"), with a toggle between them naming each game's player to move ("static-equity's move"); a rack both games hold finds the pair once. A job keeping only first divergences (`capture_first_divergence`) shows exactly the turn each pair's players first chose differently. There is no list of the newest positions: a job that captures holds millions, and one at a time on a board is what the section is for. The CGP and the move notation are read by `lib/cgp.ts` as MAGPIE writes them (`game_get_cgp_string`, `move_get_string`: `8G HUH` across, `E9 (E)RUVIM` down, letters played through in parentheses, `[L·L]` for a multi-letter tile); a position it cannot read is shown as text. The board scales to its box, so a phone shows it whole. From an `lg` screen up the pane is a grid of `minmax(24rem,1fr) minmax(0,1fr)`: the board and the moves share it equally, the board never under 24rem and never over 42rem (`max-w-2xl`), and the moves start right beside it, every cell on one line with tight padding (as in the opening-rack lookup's table), the inference's leaves as wide as their three columns; narrower, the moves go under the board. The rack tiles stay square, shrinking to fit one line rather than deforming. Anyone may see them, signed in or not: an account only makes API keys. Clicking a ranked move to preview it on the board is a follow-up.

**Opening rack analysis**

- Progress: racks settled against the size of the rack space, racks analyzed, and for a job seeking a consensus the racks settled and those settled without one.
- Search input: enter a rack string to look up its analysis. Returns the full ranked move list (all N plays that were evaluated) for that rack, every analysis of it numbered, sourced from `position_analysis_moves`, each move with its win percentage, how often the simulation played it out (**Iters**, `position_analysis_moves.iterations`, which MAGPIE reports per move) and its first two plies' statistics when the player simulated (`GET /api/jobs/:id/results?rack=`).
- Below it, up to ten racks the job has analysed -- the newest, from the first page of the results feed -- each a button that looks it up, so a visitor has something to try.

  The panel used to carry the average best equity and a breakdown of what the
  best opening play was — placement, exchange or pass — and both are gone.
  They aggregated over every stored move row of the job, which made them the
  most expensive read in the payload and one that grew without bound; and they
  counted per *claim* where `racks_analyzed` counted per task. Nothing is lost from storage: every ranked move is still there,
  `GET /api/jobs/:id/results` still returns the best move, score and equity per
  rack, `?rack=` still returns a rack's full ranked list, and an
  [admin export](#exports) is the path for analysing the corpus properly.
  Summarising millions of racks in two numbers on a progress panel was not
  where that analysis belonged.

**Leave generation**

- Current generation number and that generation's own occurrence target (e.g., "Generation 3 of 6 — target 500 occurrences per rack"), and every generation's target when they differ.
- **Live**, pushed as results land: tasks completed and games played in the in-progress generation.
- **As of the last merge**, and labelled with its time: racks at target out of the generation's total, and the rack with the fewest occurrences with its count. Accepted results are staged and merged into the per-rack totals in batches — every half hour, and about once a minute as a generation nears its end — so these lag by that much; see [What a merge costs](#what-a-merge-costs). Neither comes from a worker's heartbeat.

---

#### The stats payload

One function computes job statistics, and both `GET /api/jobs/:id` and the SSE
push use it, so a live update is byte-for-byte what a page reload would produce.

```
JobStats {
  job:               { id, name, job_type, status, allocation,
                       min_magpie_version, created_at, created_by, lexicon, variant }
  tasks_total, tasks_completed, tasks_available, tasks_claimed
  movegens           // jobs.movegens: every accepted claim's, summed
  games?:            { unit: "game" | "pair", wins, losses, draws,
                       units_completed, pentanomial?, divergent_pairs?,
                       divergent?: { wins, losses, draws, p1_score_mean, p2_score_mean,
                                     spread_mean },   // pairs: the games that diverged
                       min_units, max_units,
                       p1_score_mean, p2_score_mean, spread_mean,   // null before any game
                       test: { mean, lower, upper, confidence_pct, status } | null,
                       decided?: { status, lower, upper, units } }
  opening_racks?:    { racks_analyzed, racks_settled, racks_without_consensus, racks_total }
  leave_generation?: { current_generation, generation_count,
                       generations_closed,    // generations whose KLV is built
                       target_rack_count,     // the current generation's
                       target_rack_counts,    // every generation's, in order
                       tasks_completed, games_played,            // live
                       racks_at_target, racks_total, min_rack, min_rack_count,
                       progress_as_of }                          // as of the last merge
  workers:           [ { user_id, anon_id, username, tasks_completed, compute_seconds } ]
  other_workers:     number
  eta_seconds:       number | null      // null without enough recent throughput
  completion?:       { at, forced, reason }  // a completed job's job.completed row
}
```

The three per-type blocks are omitted rather than null for job types they do not
apply to. `workers` is always present, so a client can read its length without a
presence check. There is no `ratings` block: ratings belong to rating pools, not
jobs, and are read from the ratings page.

**Several of these figures are running totals, not aggregates.** `games?` and
the job list's `units_completed` read `jobs.games_completed`,
`opening_racks.racks_analyzed` reads `jobs.racks_analyzed`, and the job list's
task counts read `jobs.tasks_total` / `jobs.tasks_completed`. The first two are
maintained in the submit transaction, once per accepted result (a task has one). `tasks_total` rides on the claim's existing
`UPDATE jobs`, and `tasks_completed` on the moment a task actually *reaches*
completed, which the submit path's own update already computes.

They exist because the reads did not scale: the job list re-derived per-task
game totals for every job on every page view (2.2 s at the test volume) and
counted each job's tasks twice more besides, and counting distinct analysed
racks cost seconds at a million racks on every detail view and every live push.
Nothing that *decides* anything reads them: the match test still reads `game_results`, so
a drifted counter is a wrong number on a page and cannot stop a job early. A
purge zeroes them and a partial restore recomputes them (RUNBOOK §2.3).

**The contributor lists have counters of their own**, on the identity rather
than on the job: `users.tasks_completed` and
`anonymous_workers.tasks_completed`, with a `last_completed_at` beside each.
`/api/users` and `/api/workers` rank by contribution, so counting meant reading
every identity's whole claim history before a `LIMIT` could apply — the ranking
is the thing that cannot be paginated around. These differ from the job counters
in one way that matters: a job's counter belongs to the job, so a purge zeroes
it, while an identity's spans every job it ever worked on. `purge_job` and
`delete_job` therefore hand back exactly what the job contributed, per identity,
before its claims are destroyed. `last_completed_at` is deliberately not rewound
by that — finding the new maximum is the scan the counter exists to avoid, and
it is a display figure that only ever moves forward. Account deletion subtracts
nothing: it anonymizes in place and keeps the claims, so no donated compute is
lost.

Beside `tasks_completed` each identity carries **`compute_ms` and
`movegens`**, and `/api/workers` ranks by movegens unless asked otherwise
(`?sort=movegens|compute|tasks`). MAGPIE reports no CPU time or thread count,
so compute is the time each accepted claim was held, claim to submission, in
whole milliseconds (`CLAIM_COMPUTE_MS`, one expression for the submission that
adds it, the purge that gives it back and RUNBOOK §2.3b's recount). Movegens
are MAGPIE's own count of the work: every call to its move generator during
the task, on every thread -- sims, endgame and pre-endgame searches and
autoplay alike -- reported beside the result as the submission's `movegens`.
They are the default ranking because they measure work done whatever the
machine: tasks undercount a machine that plays slow, deep games, and compute
time overcounts a slow one. Each claim records its own (`task_claims.movegens`),
which is what lets a purge give back exactly what the submissions added by
summing the claims it is about to delete, without reading the results. Movegens
are the one measure the server cannot check, so a claim reporting more than a
million per millisecond held is refused (`MAX_MOVEGENS_PER_MS`): that stops
nonsense, not a determined liar, which is what bans are for. (Until 2026-10 the
counters were games played and racks analysed, derived from each result; they
said little about the work, and a games batch of deep sims counted the same as
one of static play.) Each order is its own pair of partial indexes, one per
kind of identity, so every order is two index scans merged, as the list always
was.

**Contributions by job type.** The job keeps a running `jobs.movegens` and
`jobs.compute_ms` too, added in the same `UPDATE jobs` every accepted
submission already makes (last, after the claim, the task and the
contributor's row -- so no new lock and no new lock order: the statement
takes the job's row whatever it adds), from the figures that submission
credits its contributor with. The movegens are the line under the job page's
Contributors table ("N movegens for this job"). Summed by `job_type` over the
jobs with `tasks_completed` -- one row per job, whatever the claims number,
in one scan -- they are the Contributions page's "Site totals": the three
figures across the site, then the same by job type (`GET
/api/workers/movegens`, each type a `{movegens, compute_seconds, tasks}`). A
purge zeroes them with the other job counters and a delete takes the row,
exactly as each gives the same claims' work back from the contributors, so
the jobs' totals and the contributors' always add up to the same figures. One contributor's breakdown
(`GET /api/workers/user/:id/movegens`, `/api/workers/anon/:anon_id/movegens`,
fetched when their row is unfolded and with each refresh while it is) sums
their claims per job -- movegens, compute time and completed tasks, the last
two over completed claims only (a FILTER on `completed_at`, never a `state`
the planner would read as a cue for the fleet-wide completions index) -- and
groups the jobs by type: an index-only walk of their range of
`task_claims_user_idx` / `_anon_idx`, which `INCLUDE (movegens, claimed_at)`
for it, costing their own claim count and nothing else's. The page shows it
as a small table, a row per job type under the site table's headers. Only an account or a
pseudonym the list shows is answered (a `404` otherwise), and an anonymous
worker by its pseudonym only, as everywhere public.

With the two opening-rack aggregates removed, `opening_racks` is now those two
counters and nothing else: two single-row reads, constant time at any job size.

**The contributor list is capped** at 50, with `other_workers` carrying how many
more there are. It was every worker with an accepted result, unbounded, and a
popular job has thousands — all of them serialized into every detail view and
every live push. The count is only computed when the cap is actually reached,
which for most jobs is never.

**`compute` logs when it takes over a second.** Every read in the payload is
display-only — nothing in the claim path reads a statistic — so none of it is
urgent, but several still grow with a job's history: contributions with claims,
leave progress with generations, task counts with tasks. Moving them to a
background refresh is a real option and a real cost (staleness on a live
dashboard, and a cache to keep coherent), so the log line is there to make that
decision on evidence rather than on a guess about when it starts to matter.

**ETA** is extrapolated from claims completed in the last hour, or since the job
was last activated if that is more recent (at least a minute): over a whole hour
a job ten minutes old read six times its real time left. The hour stays a
constant bound on the scan, so the completed-claims index serves it; the
activation is a second condition. It is `null` for
an inactive job and `null` when nothing completed in that window — there is
nothing to extrapolate from, and a fabricated number is worse than a blank. For
games and pairs jobs the remaining work is measured in units against `max_units`, at the
rate units have been finishing (claims × games or pairs per batch). For an
opening-rack job it is the analyses left -- the racks not yet settled, times
the analyses each still needs -- at the rate racks have been analysed (claims ×
batch). A leave job has no ETA: its generations' size
depends on the draws. A job already past its cap reports 0. Tasks are made on
demand, so "remaining tasks" was only what was in flight — a 3.2-million-rack
job 1% done read three minutes left — and the job list's progress bar likewise
counts each type's own units (games or pairs, racks settled, generations
closed) rather than tasks handed out so far, as do the job pages (a leave job's
`generations_closed`, not `current_generation`, which stops at the last).

#### Live updates

Each job with at least one dashboard subscriber gets a broadcast channel, created
on first subscribe and dropped when a later publish or subscriber check finds
nobody listening (a few kilobytes a job until then). The submission path checks
for a subscriber before building a payload at all. `GET /api/jobs/:id/stream`
sends the current stats immediately as its first event, then pushes as results
land, at most one per `JOB_STATS_CACHE_SECONDS`, all named
`stats`, with a 15-second keep-alive so an idle connection survives an
intermediary's timeout. The route is public, and each open stream holds a
connection, a task and a receiver, so at most 2,000 are open at once across
every job, and at most 32 from one client address (otherwise one host could
hold every place, idle, and every other page got a 503). Past either a stream
is a `503`; the page tries again after 5 s, doubling to a minute while it is
refused, jittered so refused pages do not return together. A push is shared among its subscribers (`Arc<str>`),
not copied to each.

**An event per round, not one per result.** Building a payload is several
aggregates over the job's history, so it happens off the submitting request and
one at a time per job: a submission that finds a build already running marks it
to repeat rather than starting a second. A burst of submissions therefore
collapses into the one payload that follows it, which is both fewer reads and
strictly fresher data than a queue of payloads would deliver. What the page
loses is a guaranteed event per result, which it was not counting: every event
carries the whole payload rather than a delta, so a merged one says everything
the ones it replaced would have. Every build is followed by a cool-down of
`JOB_STATS_CACHE_SECONDS` (10 by default, never under a second) with the push
still in flight, so a job's pushes are at most one per interval however its
submissions arrive; an admin's change, a job's completion or a leave generation closing cuts the cool-down short. Paused only
after a build during which another was asked for, a job whose submissions came
slower than a build was rebuilt for each (thirty-second audit).

---

Raw result data is queryable via a **public** API with pagination and filtering
by worker. **Bulk reads are admin-only**: the streaming download scans a job's
result tables from a cursor and holds a database connection for as long as its
caller keeps reading, so one request was enough to start a scan of tens of
millions of rows against a pool of twenty. It lives under `/api/admin`, at most
[two run at once](#exports), and for a completed job it redirects to an export
rather than re-scanning.

The **job list** carries a `stalled` flag per job — workers are declining it and
none is completing it. A job pinned to data nobody has does not announce itself:
the workers go on contributing elsewhere and this one simply gets nothing done,
so the symptom is an absence and has to be stated rather than noticed. For
games and pairs jobs the list also reports `units_completed` against `max_units`,
because a task count that grows as work is handed out is not a meaningful
denominator; for an opening-rack job it is racks settled of `total_racks`, as
the job's own page counts them (it was racks analysed until the October 2026
audit, which a job seeking a consensus reaches long before it is done), and for
leave generation generations closed of the count.

#### Exports

A job's whole corpus is read **once**, not once per caller — at any point in the
job's life, as a snapshot while it runs and as its final corpus once it has
completed.

The results stream scans from a cursor and holds a pool connection for as long
as its caller keeps reading. That is fine for a spot check and wrong for a
corpus: a full English opening-rack job is tens of millions of rows, and one
caller per scan is one connection per scan against a pool of twenty. So bulk
reads are admin-only, at most **two streams run at once** (a semaphore permit
held for the life of the response body, released when a caller disconnects as
well as when one reads to the end), and a completed job is served from an
artifact instead. A caller that disconnects takes its query with it: the
stream's connection, like an export's, is closed rather than returned, because
a pool connection dropped mid-result is drained first — Postgres built the whole
corpus for every hung-up spot check, outside the permit, until the thirty-first
audit. Building an export reads the corpus through the same pool, so
a job has **one export running at a time** (a partial unique index) and at most
**two build at once** across the server (a build that outlives six hours is
failed, freeing the job for another request, and one whose row an admin
cleared while it waited does not start); beyond that an export waits its turn
`running`.

`POST /api/admin/jobs/:id/export` spawns a task that streams the job's rows out
as gzipped NDJSON straight into an S3 multipart upload — nothing larger than one
8 MiB part is ever resident, which is what lets it run against a job whose
results do not fit in memory. The row is written before the task starts, polled
through `GET`, and answered with a presigned URL once ready, so **the bytes never
pass through the backend** and never touch the connection pool the cap exists to
protect. A URL is good for an hour, or for what the credentials signing it have
left if that is less: on ECS they are the task role's temporary ones, and a URL
dies with them whatever it says, so each is signed with credentials asked for
then (the SDK's cache kept the old ones until moments before they expired) and
says no more than they have (`artifacts::presign_ttl`, `U-ART-1`).

**The compression runs on the blocking pool, not on the async executor.**
Compressing and hashing are the whole cost of an export — a corpus is gigabytes
of JSON — and the database delivers rows faster than they compress, so done
inline the loop's `await` never had to wait and the task never yielded. A Tokio
worker that does not yield stops more than its own task: the worker that last
polled the I/O driver is the one new socket events wait on, and when that is
the worker doing the compressing, nothing is accepted, read or written — by the
whole server — until it comes up for air. Found with one full-size leave
generation exporting: eleven worker threads parked, one at 100%, and `/health`,
claims and submissions unanswered for as long as the export ran (minutes, in a
debug build; shorter bursts of the same thing in a release one). Rows are now
read as text Postgres has already serialized, gathered a mebibyte at a time,
and compressed and hashed through `spawn_blocking`; the same export finishes in
34 s with `/health` at 2 ms median and 8 ms at worst throughout. The rule is
general, and the other computations of that size follow it: an import's
gunzip-untar-hash of a whole tarball; every password's zxcvbn score (close to a
second for a crafted hundred characters of its substitution letters: on the
executor, one address within its limits held `/health` for 8.5 s until the
thirty-second audit; two at a time, with turns of their own, since on sign-in's
turns a reset link replayed with weak passwords from eight addresses held every
sign-in at `503` — and each reset link buys five scorings an hour) and every
Argon2 hash and verify (on four
threads of their own, at most four at a time — each holds 19 MiB, and a flood
of sign-ins and registrations from seven addresses, within every per-address
limit, took the process past its 2 GiB task until the thirty-second audit; a
request that waits ten seconds for a turn is told `503` to come back; the
turn goes with the run, so a client that hangs up does not leave a run queued
past the four; and the allocator's mmap threshold is pinned at 1 MiB so each
run's buffer is given back, not kept by the thread that ran it — at a cost of
12–18% of Argon2's throughput, paid for a peak of 129 MB where 2.6 GB was measured);
each
50,000-rack chunk of a universe's `COPY` (`/health` reached 1.3 s while one was
seeded, 3.6 ms after); the parse of a request body of 256 KiB or more; and the
decoding and validation of every submission, which for a full batch is tens to
hundreds of milliseconds.

**What a line holds** follows the job type: a `game_results` row for games and
game pairs, a `leave_rack_progress` row for leave generation, and for opening
racks a `position_analysis_records` row **with its ranked moves and their
per-ply statistics nested in it** (`moves: [{rank, move, score, equity,
win_percentage, blended_utility, plies: [...]}]`). The record alone is a header
— the rack, how many moves were ranked, when — and for a while that was all an
export held, which made the artifact this document calls the path for analysing
the corpus an artifact with no move in it. Nested rather than joined, so the
unit stays one line per record and `row_count` counts records; each record's
moves come through `(record_id, rank)`, so the cost is an index probe per
record — about 70 seconds per million records at five moves each on the
development machine, on a background task. The admin stream runs the same
queries, so the two are the same corpus. A captured position's record carries
its `inference` beside its moves (null when it has none), and an opening-rack
record its rack's consensus standing when the job's maximum is above one
analysis per rack.

**A games or game-pairs job that captured positions exports two objects.** Its
result rows, as ever, and beside them `…/<export>.positions.ndjson.gz`: every
position the job captured while playing (`capture_positions`), each with its CGP,
game index, turn number and ranked moves, in the shape an opening-rack line has
— the same query, since every `position_analysis_records` row of such a job is a
captured position. `GET …/export` answers with `positions_row_count`,
`positions_bytes` and a `positions_download_url` beside the results' own, and
the admin stream takes `?positions=true`. A second artifact rather than tagged
lines in the first: a games export has always been one shape per file, and a
consumer of it should not start meeting lines of another kind. Before this the
corpus capture exists to build had no way out of the database at all. Both
objects are written before the row says `ready`, a purge deletes both, and both
live under `exports/` and expire with it.

**Any job can be exported, and the export says what it is.** A job still taking
results — active, or inactive between runs — exports a **snapshot**: the corpus
as of the moment its snapshot was taken, consistent in itself, downloadable,
and labelled on the admin page "Snapshot as of <time> — job still running". A
completed job exports its **final** corpus, which is what makes that artifact
worth keeping: a completed job's results are immutable, so it is built once and
reused by every later download, and the results stream redirects to it. Only a
final export is ever served in the completed job's place — `newest_ready`, the
redirect, reads `is_final` exports only — so a completed job whose only exports
are snapshots streams from the database, and the page offers to build its final
export. (Before exports could be taken mid-run the rule was "completed only",
and the redirect served the newest ready export whatever it was built from:
an export taken mid-run would have become the completed job's corpus.) The one
way a completed job's corpus changes is a consensus edit of an opening-rack
job. One that unsettles racks reopens the job, which takes results again; one
that leaves every rack settled still restates each rack's standing, which
every line of the export carries (a lower share turns racks settled without a
consensus into agreed ones). Either way the edit demotes the job's final
exports to snapshots (`exports::unfinalize`), since its final corpus is the
next one built once it is completed; an export still building then fails ("the
job's consensus settings changed while this export was building: export it
again"), since its snapshot may have been read before the edit, and the job is
completed -- still, or again -- by the time it finishes. (Until the
thirty-third audit's third pass only a reopening demoted them, and an edit
that left the job completed went on serving the old standings as its corpus.)

**Final is decided inside the snapshot.** An export reads its results and its
captured positions in **one** `REPEATABLE READ`, read-only transaction on one
connection — read on two, as they were, the positions file of a running job
could name results the results file did not hold. The transaction's first
statement takes the snapshot and asks of the state it sees whether this is the
final corpus: the job completed, no claim still open, and (a leave job) nothing
staged. Not what `start` saw: a job exported while active can complete before
its build begins its read, with its last claims still landing, and one marked
final on a status read before its rows were would be the short corpus the
marker exists to rule out. The transaction holds one connection and one
snapshot for both scans, as the two scans each held one before; the two-builds
cap, the six-hour limit and the closed-not-drained connection bound it the same
way.

**A running leave-generation job's export reflects its last merge.** Its corpus
is `leave_rack_progress`, which a merge updates at most every half hour; the
export does not force one (only a completed job's is merged first, below), and
the admin page says so beside a snapshot of a leave job.

**A completed job's export waits until its results have settled.** A job is
marked completed the moment its stopping rule is met or an admin forces it, but
the claims already out are still played and still accepted, so its results keep
arriving for up to the heartbeat timeout. An export started in that window
missed them, and every later download was redirected to it (it would now be a
snapshot, never redirected to, but it is still not what an admin exporting a
completed job asked for). So a completed job's export is refused (`409`) while
any claim of the job is still open — a running job's is not, since an active
job always has claims out; no claim can be issued against a completed job,
so once none is open the results are fixed. The job's lapsed claims are
reclaimed first, through the same statement dispatch uses: reclamation is
otherwise lazy, running only when a worker asks for work and the job is a
candidate, and nothing ever asks for work from a completed job — so a
claim whose worker vanished would have stayed `claimed`, and refused the
export, for good.

**For a leave-generation job, settled includes merged.** Its corpus is
`leave_rack_progress`, and an accepted result reaches that table only at a
merge. A job whose last generation closed has nothing staged — the transition
drains first — but one an admin force-completed mid-generation does, until the
half-hourly sweep, and an export built in that window was short by every result
accepted since the last merge, for good. The export's background task merges
what is staged (waiting for a merge already running) before it reads a row, and
the admin stream of a completed job does the same (`exports::settle`).

Exports are derived data and are treated differently from the leave-generation
KLVs in every way that matters: a purge deletes a job's exports with the results
they describe (a row left saying `ready` would hand an admin a stable-looking
artifact of a job that no longer holds any of it), they expire from the bucket
after 30 days — and the server stops relying on one a day before that
(`exports::EXPORT_LIFETIME_DAYS`): past it, a completed job's result stream falls
back to the scan instead of redirecting to an object the bucket has deleted, and
the admin page says *expired* rather than offering a dead download — and they
are **not** cross-region replicated — losing one costs a
re-export, where losing a KLV costs a rebuild that needs the database. `row_count`
is recorded so a later mismatch against the job is visible rather than silent,
the same reason the KLVs carry a digest.

#### Audit Log

Every significant admin and account action (job created, user banned, …) and every worker decline is written to an append-only log table for debugging and accountability. Claims and submissions are recorded by `task_claims` itself.

#### What these reads cost, measured

Every query below is copied verbatim from the code and run against synthetic
volume in a throwaway database on the local compose Postgres (16 at default
settings: 128 MB `shared_buffers`, 2 parallel workers, 12 cores), 2.7 GB in all:
a `game_pairs` job of 400,000 pairs and a `games` job of 400,000 games with every
tenth task completed twice (as under redundancy 2, removed since: these
timings were measured before it was, on about 10% more result rows than a
current database holds); 40 more paired jobs over 20
configs in one rating pool, 600,000 paired results in all; an opening-rack job of
1,000,000 analysed racks at 10 moves each, a third of a full English job; and a
leave-generation job's 3,199,724 progress rows. Warm times, best of two:

| Query | Runs | Time |
|---|---|---|
| `game_pair_stats` over 400,000 pairs | every eighth paired submission (and an idle job's check), and every SSE build | 54 ms (the thirty-second audit measured 430–520 ms at 400,000 *result rows*, a batch of one pair each, the form's default, on the redundancy-era read that sorted every row by task and spilled at the default `work_mem`; the plain sum that replaced it is not re-measured) |
| `game_stats` over 400,000 games | every eighth game submission (and an idle job's check), and every SSE build | 50 ms (the thirty-second audit measured 340–620 ms at 400,000 *result rows*, a batch of one game each, the form's default, on the redundancy-era read that sorted every row by task and spilled at the default `work_mem`; the plain sum that replaced it is not re-measured) |
| `list_jobs`, 42 game jobs — **as it was**, re-deriving per-task game totals | every job-list page view | **2,188 ms** |
| `list_jobs` task counts — **as they were**, two `COUNT(*)`s over `tasks` per job | every job-list page view | linear in every listed job's task history |
| Contributor lists — **as they were**, grouping every completed claim in the database | every `/api/users` and `/api/workers` page view | 93 ms at 44,000 claims, linear from there |
| `opening_rack_stats` — **as it was**, racks analysed and average equity in one query | job detail and every SSE push | **2,086 ms** (3,296 ms at 1,000,000 racks on a later run) |
| `opening_rack_stats`: the average alone, after the split | job detail and every SSE push | 343 ms |
| `opening_rack_stats`: best-move types | job detail and every SSE push | 541 ms (322 ms on the later run) |
| `opening_rack_stats` **as it is now** — two counters, after both aggregates were dropped | job detail and every SSE push | two single-row reads |
| Rating sweep `build_matrix`, 600,000 paired results | a fit: the sweep, when a pool's evidence has grown (public reads serve the stored run) | 452 ms |
| `worker_contributions`, 44,000 claims | job detail and every SSE push | 136 ms |
| Public worker list — **as it was**, grouping every claim | page view | 93 ms (it now reads the per-identity running counters on `users` and `anonymous_workers`, a page at a time through their indexes) |
| Leave `next_step` rack selection — **as it was**, ordering on `(occurrence_count, rack)` through an index without `rack` | every leave claim, inside the dispatch lock | 225–390 ms at **400,000** racks (an eighth of English; a scan and sort of the generation, so linear from there — 2–3 s at full size). The 47 ms first recorded here was measured on counts that rarely tied |
| Leave `next_step` rack selection **as it is now** — a sweep of the primary key from a cursor while many racks are below target, lowest count first on the narrow index once few are | every leave claim | sweep: 3.9 ms at 400,000 racks with 97% of them at target (16,000 rows stepped over for 501 racks; nearer 0.1 ms early in a generation), and the same with nothing staged or ten thousand results staged; tail: 0.5 ms with little out, 67 ms with all 50,000 racks below target out (it begins only at a lap's boundary, so a sweep's claims are never among what it excludes) |
| Leave universe probe (`universe_exists`, and the tail's "any rack below target") — **as it was**, an `EXISTS` | every leave claim, inside the dispatch lock | a sequential scan once the generation is in the table's statistics, reading every older generation's rows of every leave job before the current one's: 133–156 ms over 2,000,000 older rows, custom or generic plan, and 135–279 ms with `synchronize_seqscans` on and two jobs' claims alternating (one job's claims alone were cheap only while each scan started where the last stopped). An English generation adds 3.2 million rows. Measured by the thirty-third audit's third pass (PostgreSQL 16.15, default settings): two leave jobs, generations 1–3 and 1–2, 1,000,000 racks each |
| Leave universe probe **as it is now** — `ORDER BY` the index's order, `LIMIT 1`, outside an `EXISTS` (which drops the order) | every leave claim | 0.02–0.08 ms, an index-only scan of the primary key, custom or generic (below target: 0.04–0.13 ms on the pick index); `I-LEAVE-25` |
| `leave_gen_stats` — **as it was**, counting the generation's racks at target | job detail and every SSE push | 210 ms |
| `leave_gen_stats` **as it is now** — the generation's summary row | job detail and every SSE push | one single-row read |
| Transition: stream generation 1 by rack | once per generation | 674 ms |
| Opening-rack `next_reissue` — **as it was**, preferring unseen racks by walking the unsettled racks: one identity that analysed every rack of a 1,000,000-rack job, 50,000 racks in flight (100 reissues of 500) | every reissue claim of a consensus job, inside the dispatch lock | **5.7–6.0 s** (950,000 probes each of the rack's analyses and their claims, then nothing found; the batch came from a second query). With a prepared statement's generic plan the in-flight `<> ALL` is a linear scan per rack: that walk did not finish in 120 s, and even the second query alone took **25 s** (468 ms at 5,000 in flight) |
| Opening-rack `next_reissue` **as it is now** — a window of four batches (2,000 racks), in flight a hashed `NOT IN` | every reissue claim of a consensus job | 12–13 ms with nothing in flight, 17–18 ms with 5,000, 42–50 ms with 50,000 (most of it passing and hashing the array; reading the in-flight racks first, the rows below), custom or generic plan alike **with current statistics**, and the same whether the identity analysed every rack or none (2,000 probes, 14,000 buffers). The generic plan's estimate crosses the default `jit_above_cost`, and with JIT on (this Postgres's default) it took 90–130 ms, nearly all of it compilation — but the plan cache does not choose it (`plan_cache_mode = auto`: eight executions, all custom), its estimate being hundreds of times the custom plan's |
| Opening-rack in-flight racks — **as they were**, every reissue of the job through the seed index, its state checked on the heap | every reissue claim of a consensus job, inside the dispatch lock | 27–44 ms at 200,000 completed reissues and 100 open (4,448 buffers), 19–22 ms with none completed: linear in the job's reissue history, which for a full English job is 13,000–26,000 reissues at batches of 500, 128,000–256,000 at 50 and 0.6–1.3 million at 10. Measured by the thirty-third audit's second pass on a dataset of its own (PostgreSQL 16.15, default settings): 1,000,000 unsettled racks with 3,000,000 analyses over 10 identities, 200,000 completed reissues of 50 racks and 100 open (90 claimed, 10 given back), and another job's 2,000,000 tasks with 1,000 open claims |
| Opening-rack in-flight racks **as they are now** — the open reissues alone, given back through the queue index and claimed through the open claims' | every reissue claim of a consensus job | 2–5 ms at 200,000 completed reissues (433 buffers), custom or generic plan; 11–12 ms with none completed, where the planner sorted the 5,000 racks to de-duplicate them rather than hashing them (the reads themselves under 1 ms, 230 buffers). Bounded by the open reissues and the fleet's open claims, not by history (`I-OR-REISSUE-2` for what it reads) |
| Opening-rack "unseen first" — **as it was**, an `EXISTS` | every reissue claim of a consensus job, inside the dispatch lock | with current statistics, as above; with statistics that understate `position_analysis_records` — a database just restored by `pg_restore`, which restores none, or autovacuum behind — Postgres hashed it, reading every analysis the identity made in the job to answer for 2,000 racks: **1.95 s** custom and **1.2 s** generic for an identity with 300,000 analyses (the same dataset; 5.3 s generic with the table never analysed, in the pass's first measurement) |
| Opening-rack "unseen first" **as it is now** — a scalar subquery, `(SELECT 1 … LIMIT 1) IS NOT NULL`, which Postgres never hashes | every reissue claim of a consensus job | 22–23 ms custom with current statistics (a window of 2,000 with 5,000 in flight, the same dataset), 25–27 ms with the understated ones; generic, 30–37 ms with JIT off and 116 ms with it on |
| Opening-rack finish check — **as it was**, `EXISTS` a task and `NOT EXISTS` one not completed | the submission that settles the job's last rack, before its worker is answered; the idle and post-edit checks | 395–414 ms at 3,200,000 tasks over 11 jobs (607 ms with a claim per task): both were sequential scans of `tasks`, every job's history, the second the whole table exactly when the job was done. Measured by the thirty-third audit's third pass |
| Opening-rack finish check **as it is now** — a first task by seed, none available, no open claim | the same | 0.32–0.37 ms done, 1.0 ms with reissues in flight, custom or generic; bounded by the fleet's open claims, not by history (`I-STATS-9j`) |
| "Anything still in flight" — **as it was**, `task_claims` joined to `tasks` for the job | inline on most submissions (the finish check when the debounce would skip), an idle job's check, an export of a completed job, the job list's `stalled`, leave generation's in-flight reads | 4.8 ms for a job with no open claims of its own and 600 open in the fleet, just after a restart (a `tasks_pkey` probe per open claim; 69 ms fully cold, about 0.1 ms more per open claim); without statistics — after a `pg_restore`, before ANALYZE — a scan of every task the job ever had: 311 ms at 250,000. Measured by the thirty-third audit's fourth pass on 2.2 million tasks and 2.33 million claims |
| "Anything still in flight" **as it is now** — `task_claims.job_id`, no join | the same | 0.17–0.27 ms, and 0.04–0.08 ms without statistics: the open-claims index, bounded by the fleet's open claims (`I-STATS-9k`) |
| An export's "has this job captured positions" — **as it was**, an `EXISTS` | once per export of a games or pairs job, inside its snapshot | a sequential scan of every older job's records, for a job that did capture: 129–147 ms behind 1.5 million, custom or generic (the same dataset) |
| An export's "has this job captured positions" **as it is now** — `ORDER BY` the feed index's order, `LIMIT 1` | the same | 0.02–0.05 ms, an index-only scan of `position_analysis_records_feed_idx` (`I-EXPORT-17`) |
| Materializing a generation's rack universe | once per generation | **56–66 s** (measured as a SQL copy inside the transition, which is where it used to run; it now runs from the first claim of the generation it belongs to, off the critical path) |

What the numbers settled:

- **The match test still reads the rows, but not on every submission.** About
  50 ms at 400,000 units in large batches, but 340–620 ms at 400,000 result rows
  (batches of one, measured on the per-task sort since removed with redundancy;
  KL-10), and linear in the job's history from there. The
  stopping rule keeps reading `game_results` rather than a counter — it cannot
  be wrong because a counter drifted — and is debounced to every eighth
  submission instead, which is late rather than wrong. See [Statistical Result
  Evaluation](#statistical-result-evaluation) for why the overshoot that buys is
  mostly free.
- **The SSE push moved off the submission path**, and is coalesced per job. It
  was the other 50 ms — the full payload was built before the worker was
  answered, so a dashboard nobody had open still cost every submission the same
  aggregates, and one that *was* open cost them twice over (once for the finish
  check, once for the payload). It is now built on a spawned task, one at a time
  per job. The finish check stays inline, because it decides something.
- **The reads that grew with history became running totals.**
  `jobs.games_completed` and `jobs.racks_analyzed` first, then
  `jobs.tasks_total` / `jobs.tasks_completed` for the job list's own task counts,
  and `tasks_completed` on `users` and `anonymous_workers` for the contributor
  rankings — which could not be paginated around, because the ranking *is* the
  ordering. Splitting the opening-rack query mattered as much as its counter:
  neither the distinct-rack count (631 ms) nor the average (343 ms) is expensive
  alone — computing them together is what cost 3.3 s.
- **A job's results are read through the job, not through its tasks.**
  `position_analysis_records` and `game_results` carry `job_id`, so the public
  feed, the rack lookup, the admin stream and the export stop joining `tasks` to
  find out which rows belong to the job — which had put the filter on the far
  side of a join from the sort, making the whole job the unit of work before the
  first page existed. With the feed indexes and cursor pagination, a page costs a
  page.
- **The rating matrix is scoped to the pool's own jobs first.** It used to pick
  one result per task across the *whole* of `game_results` and filter
  afterwards, so every fit sorted the entire table. With the job filter first it
  is an index walk of those jobs' tasks. The sweep also builds it once per tick
  rather than twice (once to decide the pool was stale, once to fit).
- **Recent completions are read from a time index or a column.** The ETA (on
  every detail view and live push) counts a job's claims completed in the last
  hour. No index on `task_claims` leads with the job, so it used to walk every task of the
  job and every claim of each — the job's whole history, for a question about
  its last hour — on a job whose age is exactly what makes the walk long. A
  partial index on completed claims by time (`task_claims_completed_idx`)
  bounds it by the fleet's recent completions instead, whatever the job's age.
  The job list's `stalled` flag ("any completed in the last day") reads
  `jobs.last_completed_at`.
- **Deleting a task no longer scans the moves table.** `position_analysis_moves`
  carried a `task_id` of its own, with a cascade and no index, so every task a
  purge or a job delete removed scanned the largest table in the schema to
  find rows the record's cascade was about to delete anyway — a full English
  opening-rack job's purge was thousands of sequential scans over tens of
  millions of rows, inside one transaction holding the job's dispatch lock.
  The column is gone; moves cascade from their record through the index that
  serves every read of them.
- **A job's configuration is read once per process, not once per claim.**
  The per-type config row, the parsed letter distribution, a three-way join
  per player and the six-table `expected_data` union were re-read inside the
  job's dispatch lock on every claim, and a re-dispatched task's request
  re-read the players and the distribution again; the submit path read the
  request row for the batch size and the player config for the move cap
  inside the task's row lock. All of it is fixed at job creation, so it is the
  job's template now (`jobs::dispatch::JobTemplate`), and the reads under the
  locks are the ones that change from claim to claim.
- **Deleting a claim no longer scans the position table.**
  `position_analysis_records.task_claim_id` cascades from `task_claims`, and
  its only index was the partial unique one on `(task_claim_id, rack) WHERE
  game_index IS NULL`, which a plain equality cannot use — so a purge or a job
  delete, which removes every claim of the job one cascade at a time, scanned
  the whole records table once per claim: thousands of sequential scans over
  millions of rows for a full opening-rack job, the same shape as the moves
  cascade fixed the audit before. `position_analysis_records_claim_idx` and
  `worker_data_gaps_claim_idx` serve the two cascades that had no index.
- **A pool's residuals are stored with its fit, not rebuilt per view.** The
  public pool page used to rebuild the evidence matrix for its residual table on
  every view — 452 ms at 600,000 paired results, on the connection pool claims
  and submissions share, unauthenticated and not rate limited. A fit now writes
  its residuals to `rating_run_residuals` in the same transaction as its
  ratings, and the page reads them: a view costs a read, and the table describes
  the evidence that fit used rather than evidence that has moved on since.
- **Display reads have a pool of their own.** See [Two connection
  pools](#two-connection-pools): the reads in this table that still grow with a
  job's history can no longer take a connection a claim or a submission is
  waiting for, and each is cancelled at fifteen seconds.
- **`?worker=` is resolved before the job is read.** The results feed applied
  its contributor filter to every row of the job — a username compare, or a
  SHA-256 of the claim's UUID, per record behind two joins, which no index can
  serve — so a page for a contributor with few results, or for a name nobody
  has, read the *whole job* to find fifty rows that were not there: 2.4 s a
  request at a million opening-rack records, on a public route. The name is
  resolved to an account or an anonymous worker first (two indexed probes; a
  name that is nobody's is an empty page), and the filter is an equality on
  `task_claims`' indexed identity columns. The page is then read through the
  contributor's own claims in the job, newest completion first — one backward
  range of the identity indexes, keyed `(identity, job_id, completed_at)` —
  joined to each claim's records until the page is full. A claim's records
  share its completion time (the same transaction), so this is the feed's own
  order, and the cursor is the claim's time, so paging is exact. It costs a
  page whoever the contributor is: 0.05–1.5 ms warm, at most 52 ms cold, at
  three million games claims and two million opening-rack records, with one
  contributor holding 30% of a two-million-claim job. The two readings it
  replaced each failed someone. Walking the job's feed newest first and keeping
  the contributor's rows costs the distance to their fiftieth: all of the job
  for someone who never worked it (5.8 s cold at two million records), and
  everything since for someone whose work was early (7.4 s for 150,000 early
  results in a 1.5-million-result job). Reading through a list of their claims
  in the job needs the list to be short, and the threshold between the two, a
  thousand claims, only moved the walk to the contributors just past it. The
  range names no `state`: given `state = 'completed'`, the planner could use
  the fleet-wide completion index instead, and did for a heavy contributor on
  a large job, walking every completion in the fleet (6.2 s). A name that is
  both an account and a pseudonym is two pages, one per identity, merged; as
  one sort over both identities' claims it read all of them (1.3–2.5 s). Past
  the first page the tie at the cursor's time is broken with
  `NOT (completed_at = $4 AND id >= $5)`, not the equivalent row comparison:
  Postgres estimates a row comparison from its time column, counted the range's
  own bound twice, and for an old cursor read the contributor's whole range or
  every position record in the fleet (0.5–2 s). The opening-rack page is
  chosen before its best moves and accounts are joined, so only its fifty rows
  are.
- **A leave claim neither sorts the generation nor reads what is staged.**
  Selection used to order on `(occurrence_count, rack)` through an index on the
  count alone. Counts tie in their millions — every rack starts at zero and the
  rare ones stay there — so the index could not supply the order, and every
  leave claim read and sorted the whole generation inside the job's dispatch
  lock, whose other claimants give up after two seconds: 4.9 s a claim at full
  size. Two fixes were measured. Carrying `rack` in the index made the claim
  0.2 ms and the index 180 MB a generation instead of 22 (unique keys cannot be
  deduplicated), and left a cost that grew with what was *staged*: between
  merges the racks of every staged result are exactly the lowest, and each
  claim hashed and stepped over all of them, a microsecond a rack — 160–290 ms
  with 400 results staged. What is built instead keeps the small index and has
  neither cost; see claim step 2 under [Leave
  Generation](#leave-generation--on-demand-partitioned-generations). While
  many racks are below target they are handed out by **sweep**, in primary-key
  order from a remembered cursor, and a claim needs no list of what is out;
  once few remain, selection is lowest count first on `occurrence_count`
  *alone*, ties falling as the index holds them, with everything out excluded —
  a set that is small because the set below target is. Two details of that
  second statement are load-bearing. The exclusion is `NOT IN` over an
  uncorrelated subquery, which Postgres evaluates as a hashed subplan; as
  `NOT EXISTS` the planner ran it as a nested loop over the held-out racks (it
  guesses ten elements per `unnest`), 3.5 s at a tenth of full size. And it is
  only safe because the subquery filters its own NULLs.
- **The public results feed of a leave job is a seek into the primary key.**
  It used to run newest generation first and then furthest from target — an
  order no index runs in, so as one statement each page of fifty was a sort of
  every progress row the job has: 4.0 s a page over HTTP at one full-size
  generation, on a public route. It runs newest generation first and then by
  rack, read a generation at a time because the two directions differ: 7 ms.
- **Writing a generation's rack universe is the one slow write**, and it is
  slow on an under-provisioned database: a minute here for 3.2 million rows. It
  runs once per generation, on a task of its own started by the generation's
  first claim, generated from the pinned letter distribution and `COPY`ed in
  (`seed_generation`; see [Leave Generation](#leave-generation--on-demand-partitioned-generations)), so it costs
  the job time rather than correctness, and no worker waits on it. The
  production instance class has not been measured. (A script that timed copying
  the previous generation's rows against generating them settled that choice,
  and was removed in the thirty-second audit once it no longer ran against the
  schema.)

---

### Tech Stack

| Concern | Decision |
|---|---|
| Web framework | Axum |
| DB access | SQLx |
| Database | RDS Postgres |
| Compute | ECS with Fargate |
| Task queue | Postgres-based (SKIP LOCKED) |
| Frontend framework | SvelteKit |
| Frontend hosting | ECS (same service as backend), S3 + CloudFront later |
| Styling | Tailwind CSS |
| Component library | shadcn-svelte (dark mode only; Tailwind `darkMode: 'class'` with `dark` always applied to root) |
| Charts | Plain SVG and markup (no chart library) |
| Live updates | Server-Sent Events (SSE) via Axum |
| Auth | Roll your own (Axum + Argon2 + Paseto) |
| Email | AWS SES |
| Secrets | AWS SSM Parameter Store |
| Artifact storage | AWS S3 |
| Infrastructure as Code | Terraform |
| Local development | Docker Compose — the full stack (Postgres, MinIO, backend, Nginx frontend) runs in containers so Docker is the only host dependency |

#### Key Technology Notes

**Postgres task queue**: Tasks are claimed using `SELECT ... FOR UPDATE SKIP LOCKED`, under a per-job advisory lock, so claims against one job serialize while claims against different jobs never wait on each other. Timeout reclamation is lazy and runs at claim time.

**Claim tokens**: Each claim issues a UUID token. Workers must submit this token with their results. Stale tokens (from timed-out claims) are silently rejected.

**ECS with Fargate**: Two ECS services, each with its own task definition and image variable — the Axum backend, and an Nginx container serving the SvelteKit static build. The load balancer sends `/api/*` and `/health` to the backend's target group and everything else to the frontend's. The backend's service is a single instance that stops its task before starting the next (see `desired_count`); the frontend's is stateless and rolls (100%/200%), so a release that changes only the pages has no gap, and `scripts/deploy.sh` redeploys only the services whose sources a commit changed. (Until October 2026 the two containers shared one task, and every deploy, a page change included, stopped the backend.)

**Database migrations**: `sqlx migrate run` executes at container startup before the server accepts connections. No separate migration runner needed.

Until birdtest is deployed there is only ever **one migration**. Schema changes edit `0001_initial.sql` in place rather than adding a numbered migration, because there is no live database whose history needs preserving, and a single file is far easier to read than a schema reconstructed from a chain of diffs. The cost is that sqlx records a checksum per applied migration, so editing `0001` means any existing development database must be reset — see [Development](#development). Once there is a deployment to migrate, this convention ends and migrations become append-only.

**Observability**: Structured JSON logs via `tracing` + `tracing-subscriber` (JSON formatter). No metrics or distributed tracing for v1.

**Auth**: Sessions are PASETO **v4.local** tokens (encrypted and authenticated with a 32-byte symmetric key) in an httpOnly cookie named `birdtest_session`, default TTL 7 days. The token carries the user id, username and admin flag, but **only the user id is consumed** — username and `is_admin` are re-read from the database on every request, so a deleted or demoted account cannot keep acting on a token minted before the change.

Two different hashes, for two different threat models. **Passwords** get Argon2 with a per-user salt: they are low entropy and are only ever verified against one known row. **API keys, email confirmation codes and reset tokens** get SHA-256: they are 24–32 bytes of randomness with no low-entropy secret to protect against offline guessing, and a worker request looks a key up by exact hash match on every call — a per-key Argon2 salt would force a full table scan and a verify per row. Raw API keys are `bt_` followed by 64 hex characters.

**Secrets**: Database credentials and the session signing key are stored in AWS SSM Parameter Store and injected into the ECS task at runtime. SES needs none: the backend sends as the task's role (`infra/ecs.tf`).

**Configuration** is read entirely from environment variables — `.env` locally,
task-definition values in ECS, with the secret ones pulled from SSM at task
start. The process has no SSM code path of its own.

| Variable | Default | Notes |
|---|---|---|
| `DATABASE_URL` | — | **Required**, unless the `DB_*` parts below are set. |
| `SESSION_SIGNING_KEY` | — | **Required.** 32 bytes, hex-encoded. Startup fails if absent or the wrong length. |
| `BIND_ADDR` | `0.0.0.0:8080` | An IP address and port; a host name fails startup. |
| `SESSION_TTL_SECONDS` | `604800` (7 days) | 60 to 31,536,000 (a year); anything else fails startup. |
| `SECURE_COOKIES` | `false` | `true` in any deployment served over TLS. |
| `DEV_LOGIN` | `false` | `true` mounts `GET /api/dev/login?username=…[&next=/path]`, which signs a browser in as any account by name with no password; the local compose stack sets it, so `scripts/dev.py` opens the site signed in. Refused with `SECURE_COOKIES=true`, so no deployment can run with it. |
| `MAIL_BACKEND` | `console` | `console`, `ses`, or `file` — the end-to-end suite's, never production: each mail written to `MAIL_OUTBOX_DIR`, which it requires. Anything else fails startup. |
| `MAIL_OUTBOX_DIR` | unset | The `file` backend's directory. |
| `MAIL_FROM` | `no-reply@birdtest.local` | Required under `ses`: the default is only for local use, and SES refuses it. |
| `MAIL_MAX_PER_SECOND` | `1` | Under `ses`, the most mails sent a second: the account's sending rate (1 in the sandbox). Sends past it wait their turn rather than being refused by SES. Outside 1–1000, startup fails. |
| `PUBLIC_URL` | `http://localhost:5173` | The base for links in emails; a trailing slash is dropped. Required under `ses`, whose links must reach the real site. |
| `HEARTBEAT_TIMEOUT_SECONDS` | `300` | How long a claim survives without a heartbeat. 180 to 86,400; anything else fails startup: below MAGPIE's thirty-second cadence a live worker's claim lapses and is handed to the next claimant. 180 is six heartbeats and no more: one heartbeat stalled until MAGPIE gives up on it (about 127 s) leaves some 187 s between recorded ones, and one that crawls lapses its claim at any setting (KL-83). |
| `JOB_STATS_CACHE_SECONDS` | `10` | How old a job's stats payload (`GET /api/jobs/:id`, the stream's first event) may be, and the least spacing of its live pushes. The payload reads the job's whole history; rebuilt on every view and every second a busy job was watched, it cost about a second of database time per second on a large job. Built one at a time per job; dropped by every admin action on the job, by its completion and by a leave generation closing, so the admin page reloading after an action reads the change. `0` builds it on every request (the tests). |
| `S3_BUCKET` | `birdtest-artifacts` | |
| `S3_ENDPOINT` | unset | Set to MinIO's address locally; the AWS SDK works against it unmodified. |
| `S3_PUBLIC_ENDPOINT` | unset | The object store as a browser reaches it, when that differs from `S3_ENDPOINT`: export download links are signed for it. The compose stack sets `http://localhost:${MINIO_PORT}`, since its backend reaches MinIO as `minio:9000`; production leaves it unset. |
| `MIN_MAGPIE_VERSION` | `0.1.1` | The enforced global floor, and the default floor for a new job. Also a floor the server's own pinned MAGPIE must clear: startup fails if `MAGPIE_BIN` reports less, since the server would be publishing hashes built by a MAGPIE its workers may not run. The default (`config::DEFAULT_MIN_MAGPIE_VERSION`) is copied into Terraform's `min_magpie_version`, both compose files, `scripts/e2e_magpie_native.sh`, `backend/.env.example`, the root `.env.example`, the `jobs` column defaults and `scripts/dev.py`; `U-CFG-5` fails when a raise misses one. |
| `MAGPIE_BIN` | `/usr/local/bin/magpie` | The pinned MAGPIE the server runs for every derived file and every leave-generation KLV. The backend image builds one in; locally, a checkout's `bin/magpie`. Startup fails without a working one. |
| `MAGPIE_THREADS` | `1` | Threads given to a conversion. The web task keeps 1; the derived-file builder task sets its vCPU count. |
| `MAGPIE_SCRATCH_DIR` | the system temp directory | Where a conversion's throwaway data directory goes. The builder task points it at its ephemeral volume, since a rack info table is 1.9 GB. |
| `MAGPIE_DOWNLOAD_URL` | the MAGPIE repository | Sent in a shutdown directive, and returned as `download_url` by `GET /api/worker/client-version`. |
| `MAGPIE_DATA_REPO` | `jvc56/MAGPIE-DATA` | Where import fetches tarballs from. Configuration, never user input. |
| `GITHUB_API_URL`, `GITHUB_RAW_URL` | `https://api.github.com`, `https://raw.githubusercontent.com` | Where import resolves refs and fetches tarballs; the end-to-end suite points them at its fixtures. |
| `GITHUB_TOKEN` | unset | Optional in development, set in production: unauthenticated, GitHub allows 60 calls per hour per IP, and resolving a ref takes one call for a branch, two for a tag, and one more for each tag object peeled (at most six). |
| `TRUSTED_PROXY_HOPS` | `0` | Reverse proxies in front of the process that append `X-Forwarded-For`. Per-IP rate limits key on the entry this many from the right; `0` keys on the TCP peer. `1` behind the ALB and behind the compose Nginx. |
| `DB_HOST`, `DB_PORT`, `DB_NAME`, `DB_USER`, `DB_PASSWORD`, `DB_SSLMODE` | unset (`DB_PORT` 5432) | Read only when `DATABASE_URL` is unset, and assembled into one with the password percent-encoded — so a deployment can inject a managed password without hand-writing a URL. `DB_HOST`, `DB_NAME`, `DB_USER` and `DB_PASSWORD` are then required, and startup fails naming the missing one. |

A numeric setting that does not parse, a `SECURE_COOKIES` or `DEV_LOGIN` other
than `true` or `false`, a `MAIL_MAX_PER_SECOND` outside 1–1000, or a `MIN_MAGPIE_VERSION` that is not a version fails startup rather than
silently taking the default.

There is deliberately no `DATA_PATH`. The server reads letter distributions out
of the `input_data` row a job pins, so there is no filesystem copy to drift from
what the workers are checked against.

**Live dashboard updates**: Each job detail page subscribes to `GET /api/jobs/:id/stream` (SSE). The server pushes an event after accepted task results for that job — coalesced, and at most one per `JOB_STATS_CACHE_SECONDS` — carrying the updated aggregate stats. The client merges the event into its local state without a full page reload. Axum supports SSE natively via `axum::response::sse`.

---

### Design Decisions & Rationale

| Decision | Rationale |
|---|---|
| Allocation-only job scheduling | One axis: every claim goes to the active job furthest behind its share. A priority tier was a second axis that expressed nothing allocation cannot — a job at 0% gets nothing, exactly as an inactive one does, and a job alone above 0% gets everything — while adding a rule (100% per tier) that had to be kept coherent with it |
| Admin-only job management | Avoids abuse prevention and quota complexity in v1 |
| Seed uniqueness for seed-based tasks | `(job_id, seed)` unique index prevents duplicate work at the DB level; uint64 seed stored as signed BIGINT, reinterpreted at the application layer |
| No JSONB in configs or the audit log | All `config` and audit `metadata` are expanded into typed columns and per-job-type config tables; avoids schema-less data and keeps queries typed. (`backups.row_counts`, a per-table count the backup job records, is the one JSONB column: nothing queries into it.) |
| Lazy timeout reclamation | No background process needed; simpler to operate |
| Claim token for stale result rejection | Race-condition-free; no timestamp comparison needed |
| Anonymous workers identified by UUID | Enables per-worker contribution tracking and result filtering without requiring account creation |
| No pre-aggregation for dashboard v1, except measured exceptions | Simple stats don't require it; avoids premature optimization. Measurement (Dashboard, "What these reads cost") found the reads that grew with history badly enough to matter, and only those are kept as running totals: four on `jobs` and a contribution counter per identity |
| AWS throughout | Learning goals; avoids future migration pain; production-grade from day one |
| Batch Bradley-Terry instead of incremental Elo/Glicko | Player configs have fixed strength, so there is no drift for a sequential filter to track; a batch fit is order-independent and makes add/remove a refit rather than an unwind. WESPA's Glicko in particular would add arbitrary rating periods, an RD floor that never lets a bot's rating settle, human calibration and per-game updates that waste game pairs ([Why not WESPA's Glicko](#why-not-wespas-glicko)) |
| Ratings on WESPA's scale, 250 points per logit | A rating gap predicts the win % the same gap does between established WESPA players; the absolute level stays the anchor's, since bot games say nothing about strength against people |
| Named `player_configs` table | Reusable across jobs; maps directly to MAGPIE per-player arguments (`-r1`/`-r2`, `-s1`/`-s2`, etc.); **immutable once created** — no update endpoint exists; deletion only if no job references the config |
| Frontend dark mode only | Single theme simplifies the component library configuration; no light/dark toggle in v1 |
| Deficit-based job selection, measured from when a job joins | Deterministic; guarantees long-run allocation accuracy — as a share of claims, not of worker time (KL-88) — regardless of claim timing; no randomness means reproducible behavior. No starvation of any job above 0% — which holds because a job's deficit is measured from a baseline reset to parity on activation and allocation change (`claims_baseline`; a purged job rejoins when activated again), not over its lifetime, and a job with nothing to hand out is lifted to parity rather than banking debt (`scheduler::lift_passed_over`, `issue_claim`): a lifetime deficit let a newly activated job take every claim until it had issued as many as the oldest job beside it |
| Seed gap of batch size | Keeps task seeds unique and ordered, `next_seed = MAX(seed) + batch_size`; a batch's games are drawn from a stream seeded with its task's seed, so no two tasks share them |
| Ratings pooled across jobs, scoped by (variant, letterdist, layout) | A rating is only comparable under fixed conditions, but it is not a property of one job; pooling is what lets a config's whole record produce one number |
| Only paired jobs feed ratings | `-gp` swaps seats on every seed, so a pair is side-balanced; unpaired games would need an explicit side-advantage term to avoid biasing every rating |
| Two finish conditions for a job with a match test | `min_games`/`min_pairs` keeps an asymptotic interval from being acted on too early; `max_games`/`max_pairs` bounds compute cost |
| Jobs created inactive | Allocation is set at activation time, not creation, so the admin reviews the full active job set and assigns percentages as a single deliberate act |
| API keys active/inactive toggle | Lets contributors rotate or temporarily suspend a key without losing it; only active keys accepted for auth |
| Account deletion is app-layer, not CASCADE | The account is anonymized in place, its contributions kept, and its keys, codes and tokens deleted; a cascade would delete what the account did, and the census row it writes first must outlive it |
| Match test evaluated inline on the submission path, debounced | No background sweep needed; a debounced check is late, never wrong (see Statistical Result Evaluation) |
| Two ECS services, backend and frontend | Axum backend + Nginx for SvelteKit static files, each with its own task definition and image: cleaner than co-mingling in one process, and the stateless frontend rolls without a gap while the single-instance backend is left alone |

---

## Input Data and Capability Negotiation

birdtest used to name its input data by string — `"NWL23"`, `"winpct"`,
`"english"` — and a name is not an identity. Every such name is now a row in
`input_data` pinning one file by SHA-256; jobs and player configs reference
those rows; and a contributor who does not have the bytes a job requires
declines the task, which is a normal, expected condition rather than a failure.

The source of truth is the versioned data tarball that
[MAGPIE-DATA](https://github.com/jvc56/MAGPIE-DATA) publishes and that MAGPIE's
`download_data.sh` installs. An admin imports one by date; birdtest shows what
is new; the admin confirms; the rows become the vocabulary jobs are built from.

### The problem this solves

A task request used to name its inputs without pinning them:

| Config field | Named | Actually a file | Size (NWL23/english) |
|---|---|---|---|
| `lexicon` (per player) | `"NWL23"` | `lexica/NWL23.kwg` | 4,719,596 B |
| `leaves` (per player) | `"NWL23"` or null | `lexica/NWL23.klv2` | 3,667,340 B |
| `win_pct_model` (per player) | `"winpct_english"` | `strategy/winpct_english.csv` (one per letter distribution from `data-20260925`; a single `winpct.csv` before it) | 397,459 B |
| `letter_distribution` | `"english"` | `letterdistributions/english.csv` | 489 B |
| (board layout, never named) | — | `layouts/standard15.txt` | 244 B |
| `use_wordmap` | true/false | `lexica/NWL23.wmp` — built locally from the `.kwg` | ~104 MB |
| `use_rit` | true/false | `lexica/NWL23.NWL23.rit` — built locally from the `.klv2` and the `.wmp` | ~1.9 GB |

Two contributors could both honestly report running `NWL23` with `winpct` and be
running different bytes. This is not hypothetical: `download_data.sh` installed
`data-20251004.tgz`, whose `english.csv` is 489 bytes, while the live file on
MAGPIE-DATA's `main` is a different 273-byte file that dropped two full-width
display columns. Both are called `english`, and nothing in the protocol could
tell them apart.

The failure is silent. A wrong `.kwg` does not crash, it plays a slightly
different game. A stale `winpct.csv` does not error, it makes different move
choices under simulation. Results are well-formed, pass every shape check, and
land in `game_results`, `leave_rack_progress` and `player_config_ratings`
alongside everyone else's. Blast radius, worst first:

- **`leave_generation`** — generation *N* becomes the KLV generation *N+1* plays
  with, so bad data propagates into every later generation. It is folded into
  running per-rack totals (`occurrence_count +=`, at the next merge), so there
  is no per-claim detail to subtract back out afterwards.
- **`games` / `game_pairs`** — the match test is a decision procedure over an
  aggregate. A minority of workers on a different lexicon biases the win rate
  and the test reaches a confident, wrong conclusion. Nothing about the output looks anomalous.
- **`opening_rack`** — most recoverable (per-rack rows can be deleted and
  recomputed), but easiest to corrupt: a different `.kwg` changes which plays
  exist at all.

**And a second problem, which is not corruption at all.** A contributor has the
lexica they downloaded, not every lexicon that exists. A job on `CSW24` is
simply not work every machine can do, and the protocol had no way to say so. Any
design that only detects *wrong* data and stops would treat "I don't have that
lexicon" as an error, when it is an ordinary fact about a volunteer machine.

### Which files a task needs

Every row here was checked against MAGPIE's source rather than inferred from
names, and three of them do not work the way the field names suggest. These
rules are applied **once, at job creation**, to choose an `input_data` row —
not at dispatch, and never by the client.

| Role | Where it comes from | File | MAGPIE's rule |
|---|---|---|---|
| `kwg` | **each player**, independently (`-l1` / `-l2`) | `lexica/<name>.kwg` | per-player lexicons are first-class; `PlayerSpec.lexicon` only fell back to a shared one because birdtest sent one |
| `klv` | each player (`-k1` / `-k2`) | `lexica/<name>.klv2` | `get_default_klv_name(lex) = lex` — the old default was a duplicate of the lexicon name |
| `winpct` | each player, **only if it simulates** | `strategy/<name>.csv` | `DEFAULT_WIN_PCT_PREFIX "winpct_"` + the letter distribution (`winpct_english`), checked to cover the distribution's bag; loaded lazily by `config_load_win_pcts`. The dev and tier-6 seeds pick `winpct_<distribution>`, falling back to a pre-20260925 `winpct` |
| `letterdist` | **the job** — one per job, shared by both players | `letterdistributions/<name>.csv` | stated by the job, never inferred |
| `layout` | **the job** — `standard15` unless stated | `layouts/standard15.txt` | `board_layout_get_default_name()` = `"standard" BOARD_DIM`, `DEFAULT_BOARD_DIM = 15` |

**`variant` is not the layout.** MAGPIE has two separate settings: `-var` (game
variant, `classic` | `wordsmog`) and `-bdn` (board layout, `standard15` |
`standard21`). `variant` is a rules setting with no file behind it, so it stays a
plain `TEXT` column on `jobs` — the one field in this area that must *not* be a
foreign key. The board layout is a real file that no config named before, which
is why `jobs` now has `layout_id`: `standard15` is a default, and a default is
not a pin.

**A lexicon belongs to a player, not to a job.** `-l1` and `-l2` are independent
settings, and the two reasons a `games` job exists pull in opposite directions:
collecting data means both players run the same config, while comparing
strategies means they differ — and what differs may well be the lexicon. Putting
the lexicon on the job forced a shared value and made the per-player field an
"override", which is backwards. The consequence is that a job's expected data is
the **union over its players**, not a single lexicon's files.

**`leave_generation` needs neither leaves nor a win% model.** Generation 1 starts
from a **zeroed KLV**, not from a lexicon's shipped leaves, and the bot plays
statically, so no `winpct.csv` is ever loaded. Its data requirement is three
files: the `.kwg`, the letter distribution, and the layout.

Generation 1 is therefore not a special case on the client. Job creation
builds a KLV over every leave of 1–6 tiles with every value `0.0`, stores it at
`leaves/<job>/generation-0.klv2` — outside the creating transaction, since it is
a multi-megabyte build and an object-store write — and generation 1 fetches it
through `GET /api/worker/artifact` exactly as every later generation fetches its
predecessor. `LeaveRequest.previous_artifact_key` is consequently **never null**
and the client has no first-generation branch. The fallback it replaced ("fall
back to the lexicon's default leaves") produced different generation-1 output
from a zeroed start with nothing to flag the difference, which is why the branch
was removed rather than fixed. There is no virtual generation 0 anywhere else:
no `leave_rack_progress` rows, no tasks, nothing beyond the one row recording
the key.

**Exemptions.** `.wmp` and `.rit` get no rows — they are ~179 MB and ~1.9 GB,
absent from the tarball, and built locally. They are covered instead by
`derived_data`, where the server records the hash of the copy it built and every
claim states it (see [Wordmap and rack info table
provenance](#wordmap-and-rack-info-table-provenance)). Their inputs still have
`input_data` rows, which is what makes a derived file's identity expressible at
all.

**Byte-exact files only.** Byte identity is stricter than semantic identity, and
the gap is real: two builders can produce different bytes for the same values —
which is why `derived_data` records the builder that produced every hash. That
gap does not bite `input_data`, because every row there comes from a tarball
distributed byte-exact. It *would* bite the moment someone adds a row for a
locally generated file. Don't.

### How production data is actually distributed

`download_data.sh` pins a version as a constant in the script
(`DATA_VERSION="20260925"`), probes for `data-<version>.tgz.aa`, walks the chunk
suffixes `aa`, `ab`, `ac`, … while they exist, concatenates them, and pipes the
result through `tar -xzf`. Today (`data-20260925`) that is five chunks, ~195 MB total, 257 MB uncompressed. Two
consequences:

1. **The data version is a MAGPIE release-time constant.** Everyone running a
   given MAGPIE build has the same `DATA_VERSION` unless they went out of their
   way. This is what keeps the steady state small: most workers converge on the
   same answer about what they can do.
2. **`download_data.sh` verifies nothing.** No checksum, no signature; a
   truncated chunk or a corrupted extraction is silent. Per-file digests from
   birdtest are, incidentally, the first integrity check anything in this
   pipeline performs.

Most of `data/versioned/<version>/` upstream is symlinks into the live tree, so
**a version name is a label, not a freeze**: change `data/lexica/NWL23.kwg` and
the versioned path changes with it, and the name stays `20260925`. The only
thing that pins content is the built tarball — which is what production installs
and what import reads. The dedupe rule below is what makes this survivable: if
`20260925` is ever re-cut with different bytes, importing it again produces *new
rows*, visibly, rather than silently redefining what `20260925` meant.

### The `input_data` table

One row per distinct file — a `(path, sha256)` pair, so the same path with
different bytes is a different row, which is the entire point. `tarball_date`
records the tarball a row was **first** seen in: provenance, not membership. A
file unchanged between two tarballs stays one row labelled with the older date,
because it is the same bytes and a job pinning it is pinning those bytes
regardless of which tarball the contributor installed. The full definition is in
[Schema](#schema-1).

The alternative — a join table recording every tarball each row appears in —
answers "which versions contain this file", which nothing here asks. First-seen
is one column and answers the question that *is* asked: where did this come
from, and what do I tell someone to download. A message built from it should
therefore say "this file comes from data-20251004 or later", not "you must
install 20251004". If membership is ever needed it is an additive table, not a
change to this one.

**A job pins exactly one row per role.** There is no "any of these acceptable
versions" — the complexity is real and the payoff narrow, since dedupe already
means an unchanged file across two tarballs is one row.

**Rows are deletable when nothing references them.** No soft `retired_at`, no
tombstones. The foreign keys from `jobs`, `player_configs` and `job_leave_config`
have no `ON DELETE` clause, so Postgres defaults to `NO ACTION` and a referenced
row cannot be deleted — the constraint *is* the safety mechanism. The admin
endpoint translates the violation into "this row is used by 3 jobs".

#### Why the row carries bytes

`input_data.content` holds the file's bytes for the `letterdist` and `layout`
roles, and the `CHECK` is an equivalence rather than a nullable convenience: a
`letterdist` or `layout` row without bytes cannot exist, and a `kwg`, `klv` or
`winpct` row with bytes cannot either. That is deliberate, and the reason is a
failure the rest of this design would otherwise not catch.

The server is a participant in the verification story, and it used to be the one
participant nobody checked. birdtest does not only dispatch work — it computes
things itself, and to do that it read letter-distribution CSVs off its own
filesystem from `DATA_PATH`: `total_racks` and `expand` in
[`opening_rack.rs`](backend/src/jobs/opening_rack.rs), `seed_generation` in
[`leave_gen.rs`](backend/src/jobs/leave_gen.rs), and the machine-letter numbering
baked into KWG node bytes by the KLV builder. None of those
reads went anywhere near `input_data`. There were two copies of `english.csv` in
the system with no relationship between them.

The concrete failure: a distribution gains a tile, the admin imports the new
tarball, a new job pins the new row — but the operator forgot to update the
container's `../data`. The server counts the rack space from **the old**
distribution and writes `total_racks` onto the job; it expands each task's rack
range from **the old** distribution; the worker checks its own `english.csv`
against the pinned row and it *matches*, because the worker has the new file
exactly as pinned. Every check is green. What actually happened is that the
sampling frame was enumerated over one alphabet while every game was played with
another, and if the letter *order* changed rather than the counts, the KLV's
machine-letter numbering disagrees with the worker's and leave values silently
attach to the wrong leaves. Nothing reports any of it. This is worse than every
failure capability negotiation was built to catch, because those all announce
themselves.

So the bytes live on the row, and every server-side read resolves through the row
the job pins. `LetterDistribution` has a bytes-taking constructor and **no
path-taking one**; there is exactly one copy of the truth and drift is not
representable. Lexica stay out: a 15 MB `.kwg` in a table row is a different
proposition and nothing server-side reads one.

The rejected alternative — hashing the server's local file at job creation and
refusing on a mismatch — is not sufficient on its own: it checks only at
creation, so a deployment that swaps `DATA_PATH` underneath an existing job puts
the drift back, and `expand` re-reads the file at claim time with no check at
all. It is worth keeping only as a health check for any role that is ever added
to the server-read set without being added to `content`.

**What this deleted.** `DATA_PATH` and `cfg.data_path`, the `data_path`
parameter threaded through `registry.rs`, `handler.rs` and the job handlers
purely to reach these reads, `COPY data /app/data` and `ENV DATA_PATH` in the
Dockerfile and compose file, and the `data/` directory itself. `testdist.csv`
became compiled-in fixture bytes in the test tree.


### Importing a tarball

Admin-triggered, two-phase, and never on the dispatch path. Dispatch reads local
tables only; GitHub can be down for a week without a worker noticing.

**Phase 1 — fetch and diff, in the background.** The archive is ~190 MB, so this
is not a request that waits. `POST /api/admin/input-data/imports` resolves the
ref, inserts a `running` row, spawns a tokio task, and returns the import id
immediately; the admin UI polls. birdtest runs as a single instance, so the
spawned task needs no lease — and, for the same reason, startup marks any row
still `running` as `failed`, since nothing else can be working on it. That
assumption is load-bearing only here: if birdtest is ever replicated, the import
is the first thing that breaks.

1. Resolve `ref` → a commit SHA, so `main` is pinned at import time and the
   record names a commit, never a branch.
2. Fetch `versioned-tarballs/data-<date>.tgz` at that commit, mirroring
   `download_data.sh`'s chunking exactly: try `.aa` first, walk `aa → ab → …`
   while chunks exist, fall back to the unchunked name, cap the walk and cap
   total bytes. A `404` on the first probe is the ordinary "no such version"
   answer and says so, rather than surfacing as a transport error.
3. Stream the concatenated chunks through SHA-256 (recording the tarball's own
   digest), then gunzip, then tar. This is the one place birdtest parses an
   untrusted container format, so every entry is checked against an explicit
   allowlist before it is trusted enough to hash: **regular files**, and symlinks
   at pinned paths that resolve inside the archive to a pinned file of the same
   role (MAGPIE-DATA ships aliases so), each pinned with its target's bytes —
   an alias of a letter distribution or layout counted as those bytes against
   the caps, since its row keeps a copy; a lexicon's, KLV's or win table's is a
   link on the worker and costs nothing; the
   path must be relative, carry no `..` segment, and match the expected
   `data/<dir>/<basename>` shape. **Never construct a filesystem path from an
   archive name** — the import hashes bytes and has no reason to form one.
   Anything failing aborts the whole import rather than skipping the entry,
   because a malformed archive is not a partially trustworthy one. A
   well-formed entry whose *name* MAGPIE would refuse as a path — anything
   outside `[A-Za-z0-9_-]`, a `.` included — is skipped like any file birdtest
   does not pin, so no job can be made on it: the worker checks every name a
   task carries and fails the task (after claiming it) otherwise, so a job
   pinned to `CSW24.v2` would have stopped every worker it reached.
4. Upload every `kwg` and `klv` entry's bytes to the object store, keyed by
   digest, while the archive is still in memory — re-downloading 190 MB at
   confirmation time for files already hashed would be pure waste. The server
   builds reference wordmaps, rack info tables and word info tables from these;
   a `winpct` file is neither read nor built from server-side, so it stays
   digest-only. An object already present is skipped, so a tarball whose
   lexica have not changed uploads nothing. This happens **before** anything is staged: a row that names
   an object has to be a row whose object is there, or the first derived build
   from it fails with a missing key instead of a reason.
5. Compare each `(path, sha256)` against `input_data`. Stage the result, keeping
   the bytes of every `letterdist` and `layout` entry, and the object key of
   every `kwg` and `klv` entry, so confirmation does not have to download again.
6. Mark the row `staged`, or `failed` with the reason in `error` — or, when
   every file is already known, `nothing_new`, audited as
   `input_data.import_nothing_new`: there is nothing to confirm, and left
   `staged` it offered an Insert of 0 rows and waited a day to be expired. The
   page says "No new data to insert." Its lexica and leaves were still
   uploaded in step 4, so an import is still how a missing object comes back.

**Phase 2 — confirm.** The admin sees three groups: **new** rows, **known** rows
(the majority, and the reason the diff exists), and **path collisions** — a path
already known under a different `sha256`. That last group deserves a second look,
because it is either a legitimate data update or a tarball re-cut under a name
that was already used. Confirmation inserts the new rows and the changed ones
(collisions), in one transaction, with `tarball_date` set to this import's
date; the known rows are already there.

**Limits, all enforced during the walk:**

| Limit | Value |
|---|---|
| Compressed bytes downloaded | 512 MiB |
| Chunks walked (`aa`, `ab`, …) | 64 |
| Total uncompressed bytes (an alias of a distribution or layout counted as its target's; a lexicon's is a link and costs nothing) | 1 GiB |
| Uncompressed : compressed ratio (the same total) | 20× |
| Single entry | 128 MiB |
| A letter distribution or layout (kept in its row) | 64 KiB |
| Entry count | 5,000 |
| Whole task | 30 min, 30 s connect, 120 s idle read |

Every one aborts the import rather than skipping the entry. The ratio is checked
continuously rather than at the end, since that is the zip-bomb case a total cap
alone lets through. The walk reads the archive raw, in one pass: every entry's
size is its own header's, which is what the reader follows, and the per-entry
cap and the ratio judge it before a byte is read. The walk reads extension
headers itself, each capped at 64 KiB, and takes from them only a path or link
target for the entry after; a PAX `size` or sparse record, a global PAX path,
two names for one entry, a sparse entry, and a pinned path named twice are each
refused. (In the thirty-first audit the tar reader, left to interpret
extensions, held what no cap reached three different ways — a PAX `size` that
passed every cap, a PAX header read whole, GNU sparse blocks expanded — before
this replaced three patches; and the 30-minute limit was not enforced at all:
the client's timeouts are per read.) The archive URL is built from `MAGPIE_DATA_REPO` and never
from user input, and the ref an admin gives is resolved only among that repository's own
branches and tags (`/git/ref/heads/…`, then `/git/ref/tags/…`, or only one of
them for a `heads/`, `tags/`, `refs/heads/` or `refs/tags/` prefix, so a branch
whose own name begins `tags/` is named `heads/tags/…`; an annotated
tag peeled to its commit through up to four tag objects) — `/commits/{ref}`, which it used, also resolves a
sha, five characters of one, a pull request's ref or a `git describe` name
from any fork, and GitHub serves a fork's files under the upstream's name
(thirty-second audit) — so the residual exposure is a compromised upstream; that is the threat model these checks are written
against.

Staging rather than recomputing on confirm means the download happens once and
the admin confirms exactly what they were shown. An import left staged for 24
hours is expired by an hourly sweep: its staged rows — and the distribution and
layout bytes they carry — are deleted, and the import is marked `cancelled` with
a reason, so the admin page says what happened rather than showing a gap. The
objects it uploaded stay, since they are keyed by digest and are exactly what
the next import of the same files would upload. `tarball_sha256` is kept because it is a single
value identifying a whole install, which makes "did this version change under its
own name?" one comparison. Both phases are audit-logged.

Config gains `MAGPIE_DATA_REPO` (default `jvc56/MAGPIE-DATA`) and an optional
`GITHUB_TOKEN` — optional in development, set in production, because
unauthenticated GitHub allows 60 calls per hour per IP, and resolving a ref
takes one call for a branch, two for a tag, and one more for each tag object
peeled (at most six). A `403` from GitHub
is rendered with the `X-RateLimit-Remaining` and `X-RateLimit-Reset` headers it
carries, so the failure names its own remedy.

### What the configs pin

**`player_configs`** carries `kwg_id`, `klv_id` and `winpct_id` where it used to
carry three `TEXT` names.

`kwg_id` is `NOT NULL`, which is the whole point of the move: there is no job
lexicon left to fall back to, so every player names its own. Two players in a
`games` job may name the same row or different rows, including the degenerate
case where both `player_config_id`s are the *same config row* — which stays legal
and therefore gets no `CHECK (player1 <> player2)`.

`klv_id` is `NOT NULL` too: "NULL = the lexicon default" was exactly the implicit
name-based resolution this design removes, and with two independent lexicons in
play there is no single lexicon to take a default from.

`winpct_id` stays nullable with a new meaning. It is not "use the default" — it
is "this player never loads a win% model", which is true of every static player,
since MAGPIE only reads one through `config_load_win_pcts`. Job creation
validates the pairing: a player with simulation parameters must have a
`winpct_id`; a player with none must not. That rule is worth its weight because
`expected_data` is built from what a task actually loads, and a contributor
missing `winpct.csv` should not be locked out of jobs that would never have
opened it.

**A consequence to accept deliberately:** a player config now pins bytes, so it
cannot be reused across a data update — a new `winpct.csv` means a new config
row. That is the correct outcome rather than an inconvenience:
`player_config_ratings` are only comparable among players that ran on identical
data, and nothing in the schema said so before. The cost is that a data update
means cloning configs. `name` is `UNIQUE`, so the clone convention is fixed:
`simmer-NWL23-4ply@20260101` — base name, `@`, the `tarball_date` of the data it
pins. The API leaves the name, and `cloned_from_id`, to the caller (the
admin API's player-config section); no page sets `cloned_from_id` yet. It records the lineage, because
a clone starts with no rating history and that would otherwise read as a bug: the
config page shows "cloned from simmer-NWL23-4ply@20250101 — ratings restart on
new data".

**`jobs`** carries `variant`, `letterdist_id` and `layout_id`. These were
duplicated across all four job config tables, which was three chances to disagree
and no way for a query to ask "what letter distribution is this job on" without
knowing its type first. Putting the letter distribution here rather than on the
player is not arbitrary: MAGPIE takes one `-ld` for the whole game, and two
players cannot draw from different bags. The same is true of the board.

**The per-type tables** keep only what is genuinely per-type. No lexicon sits on
a job: leave generation's bot plays as a player config too
(`job_leave_config.player_config_id`), in both seats, and takes its lexicon and
wordmap setting from it.

**What no config pins: the build's board and rack.** `BOARD_DIM` and
`RACK_SIZE` are MAGPIE compile-time constants, not settings a request could
state, and every job here plays on 15×15 with 7-tile racks (job creation
refuses any other layout; rack spaces and leave tables are counted in 7). So
the worker states both in its claim, and a build other than 15 and 7 is sent an
`unsupported_build` shutdown before any job is consulted (see
[MAGPIE version negotiation](#magpie-version-negotiation)).

#### Validation at job creation

The schema cannot express these, so `create_job` must:

- **Role match.** `kwg_id` names a row with `role = 'kwg'`, `letterdist_id` a
  `letterdist` row, and so on. A composite foreign key on `(id, role)` with a
  redundant `role` column on each referencing table would enforce this in the
  database, and is worth doing if the application-layer check ever feels too
  load-bearing.
- **Cross-player compatibility.** With independent lexicons this is no longer
  trivially satisfied: both players' lexicons must be compatible with each other
  and each with its own leaves (`lexicons_and_leaves_compat`), and both with the
  job's single letter distribution (`ld_types_compat`). birdtest should not be
  able to build a job MAGPIE would refuse to load.

  **These rules are ported to Rust rather than approximated**
  ([`backend/src/compat.rs`](backend/src/compat.rs)). They are name-prefix rules
  over lexicon and distribution names, small enough to transcribe and stable
  enough to stay transcribed. The risk of a second copy is drift, so the port is
  pinned by a test asserting a table of known-good and known-bad combinations,
  which turns a future divergence into a failing test rather than a job that
  builds here and refuses to load there. The one thing a port must not do is
  guess: if a combination is not covered by the transcribed rules, reject it and
  let the table grow.
- **Sim/winpct pairing**, as above.

#### What dispatch does

Almost nothing. The digests come from a join over the job's `letterdist_id` and
`layout_id` plus its players' `kwg_id` / `klv_id` / `winpct_id`,
**deduplicated** — two players on the same lexicon contribute one `kwg` entry,
not two. The `expected_data` builder is a query, not an inference engine, which
is what removes the piece most in need of unit tests. It is also a *single*
query: a union over the per-type config tables, which contributes nothing for a
type that has no row in one, so there is no match on `job_type` and no second
round trip to resolve the players. It runs **once per job per process**, not
once per claim: what a job pins is fixed when the job is created, so the list
is read with the rest of the job's template
(`jobs::dispatch::JobTemplate`, see [The claim loop in
full](#the-claim-loop-in-full)) and every assignment shares that copy. It used
to run inside the job's dispatch lock on every claim, where what it cost was
time no other worker could be claiming from that job. Task requests still carry
*names*, because that is what MAGPIE's command-line surface takes.

### Capability negotiation

#### Client side

1. On receiving an assignment, resolve each `expected_data` entry with
   `data_filepaths_get_readable_filename()` and the matching `data_filepath_t`.
   Using the same resolver the executor uses is the point: it checks the file
   that will actually load, across the whole `data_paths` search list. A
   contributor with both a `download_data.sh` install and a MAGPIE-DATA clone on
   `data_paths` has two `english.csv` files, and only the resolver knows which
   wins. **Every message prints the resolved absolute path**, because "your
   english.csv does not match" is unactionable when there are two of them.
2. Hash each with SHA-256 and compare. Cache by **(resolved path, size, mtime,
   inode, ctime)** so a file is hashed once per process rather than once per
   task. The inode and ctime are not decoration: `(path, size, mtime)` alone
   collides when a file is replaced with different bytes of the same size inside
   one mtime tick, which is exactly what archive extraction does, and a stale
   cache entry is the one way a bad file passes verification.
3. **Any missing or mismatched file:** `POST /api/worker/decline` with the
   details, add the `job_id` to an in-memory unsupported set, do not start the
   heartbeat, do not run the task, and go straight back to claiming. Print one
   line per newly-discovered gap — keyed by (resolved path, expected digest), so
   the first occurrence logs at warn and repeats are silent until the key
   changes. Not once per claim, or a client with a missing lexicon becomes a log
   firehose.
4. **All files match:** proceed as normal — heartbeat, execute, submit.
5. Send the unsupported set with every subsequent claim.
6. **On a `shutdown` directive:** print the accumulated gaps, which the client
   knows file by file, followed by the server's message and the
   `download_data.sh` remedy, then exit cleanly through the `ErrorStack` the
   other `impl_*` entry points use, so a GUI driving `contribute` in `-mode
   async` gets one terminal state rather than a scrolling failure.

```
Cannot contribute to any available job.

Missing or outdated input data:
  lexica/CSW24.kwg          not found in any data path (./data)
  strategy/winpct.csv       has sha256 4f2a…, jobs require 51b651f1…

These come from MAGPIE-DATA data-20260101 or later. Run ./download_data.sh
from your MAGPIE directory to update, then start contribute again.
```

**The unsupported set is in memory only.** It is never written to
`contribute.txt`, and the assumption behind that is explicit: a contributor who
stops and restarts `contribute` has, in the case that matters, just updated their
data — that is what the shutdown message told them to do. A client that
remembered its limitations across restarts would refuse work it can now do, and
the only cure would be a config file the user has to know to edit. Forgetting
costs one wasted claim per job on the next run.

#### Scheduler side

`candidate_jobs` ([`scheduler.rs`](backend/src/scheduler.rs)) takes the
unsupported set *and* the worker's MAGPIE version and excludes every job either
rules out, in the same `WHERE` clause that excludes inactive jobs and jobs at
0%. What is left is ordered by deficit, so a worker that cannot run the job
furthest behind its share is simply offered the next one: the list is *the
jobs this worker can actually run*, in the order the scheduler would have
offered them to anyone.

The answers are one decision rather than four checks: `scheduler::claim` returns
a `ClaimOutcome` and the HTTP mapping happens once at the edge, so the compiler
enforces that every branch is considered — which scattered early returns cannot.

| Outcome | When | Response |
|---|---|---|
| `Task` | Candidate jobs remain after filtering and one has an available task | `200` with the assignment |
| `Idle` | Candidate jobs remain, none has an available task right now | `204` |
| `NoWorkExists` | No job is offering work: none is active, or every active one is parked at 0% | `204` |
| `Shutdown` | Jobs are offering work (active, above 0%), but this worker is ruled out of **all** of them | `200` with a `shutdown` object |

`NoWorkExists` is separate from `Idle` because a quiet server is not the worker's
fault, and telling a contributor to update their data because nothing happens to
be running would be actively wrong. `204` and `shutdown` must never be conflated
either: one is "nothing right now, sleep and ask again", the other is "you will
never be useful until something on your end changes".

#### Declines are the observability

Declining releases the claim the same way reclamation does, and the bookkeeping
matches `reclaim_expired` exactly: set the claim state, decrement
`tasks.active_claim_count`, recompute the task's state so it becomes available
again. A decline calls `release_claim(tx, claim_id, terminal_state)`; expiry,
which releases many claims in one statement, repeats the same counter and state
formula in `reclaim_expired_for` — the two must stay the same, and differ only
in the state they set. It acts only on a claim that is *still* `claimed`
and reports whether it did anything, because the counter it decrements is what the
scheduler believes about live work: releasing one claim twice would decrement
twice, and a drifting counter makes a job look saturated so dispatch quietly stops
days later, nowhere near the cause. `'declined'` is its own value on the `claim_state`
enum rather than a reuse of `'abandoned'`: the two mean different things, and
only one of them is diagnostic.

That enum addition has a sharp edge worth remembering: the one-slot index
`task_claims_one_slot_idx` is partial, `WHERE state IN ('claimed', 'completed')`,
for a reason. Were `'declined'` inside it, a declined claim would hold the task's
one slot for good: nobody could claim the task again, the worker that declined
it included, after fixing its data.

`worker_data_gaps` is worth more than it looks. A job pinned to data nobody has
does not announce itself — it quietly gets no work done, and the server-side
symptom is an absence: claims issued, no results. This table turns that absence
into a statement: *"job X: 14 workers, all missing `lexica/CSW24.kwg`"*. It also
answers "what is actually installed out there", which is what tells an admin
whether the fleet has picked up a new tarball yet, and therefore whether a job
pinned to it will find anyone to run it.

A decline's `missing` list is bounded before it is written: at most 32 files, and
at most 128 characters per role, name and digest. A task loads a handful of files,
so an honest decline names a handful; every entry past that is a row an untrusted
client chose to write.

**Scheduling uses the client's list, not this table.** The server records gaps
for humans; it does not use them to route. If it did, a contributor who updated
their data would stay blocked by a record of a problem they had already fixed,
and the only cure would be a server-side reset nobody would remember to run. The
client sending its own set each time makes the state self-correcting.

### MAGPIE version negotiation

Client capability has two axes and they behave identically: a worker either has
the data a job needs or it does not, and it either has a new enough MAGPIE or it
does not. The same shape applies to both, replacing the arrangement where the
server dispatched work and the client discovered afterwards that it could not run
it.

The claim carries `magpie_version`, and the scheduler excludes any job whose
minimum exceeds it, in the same pass as the unsupported set, so a worker
locked out of one job is offered the next in deficit order.

`min_magpie_version` is three integer columns rather than one `TEXT`, because
semver in `TEXT` compares lexically, where `'1.10.0' < '1.9.0'` — a bug that
appears only once a minor version reaches double digits, i.e. long after it is
written. Postgres compares row constructors element-wise, so the filter reads
directly and needs no function:

```sql
WHERE (j.min_magpie_major, j.min_magpie_minor, j.min_magpie_patch)
      <= ($1, $2, $3)
```

**The floor is not optional.** Every job pins input data — at minimum a letter
distribution and a layout — and a client too old to understand `expected_data`
will contribute unverified rather than decline. "No floor" is not a state worth
being able to express once every job depends on the client honouring a protocol,
so the columns are `NOT NULL` and default to **`0.1.1`**, the same value as the
server's `MIN_MAGPIE_VERSION`, which `create_job` writes explicitly.

`0.1.1` is the `birdtest-contribute` version the backend image pins. The version
was incremented through `0.5.1` as birdtest and MAGPIE changed together during
development and reset to `0.1.0` when the protocol settled; none of those numbers
shipped. **The version moves with every change that can alter what a task
computes or submits, and the floor moves with it**: a floor is a minimum, not a
pin, and it is the only way the server has to keep a build it knows computes
something wrong off its jobs. The eleventh audit found that rule had been broken
— builds from between 16 and 24 September 2026 all reported `0.1.0`, among them
ones whose capturing static player played its worst move and every one that
played a leave task after its first with the previous task's KLV — so the branch
went to `0.1.1` and the floor with it. The
one place the floor lives is the server's configuration: the column default,
the Terraform variable, the compose file and the env examples all carry the
same value so no path writes a lower one.

The floor and the backend image's pinned MAGPIE (`docker/Dockerfile`'s
`MAGPIE_COMMIT`) move together: the server refuses to start with a pinned MAGPIE
below its own floor, so the floor can rise only once the pin has been moved to a
commit that reports the new version, and moving the pin is a deliberate step.
Because a stale config value would silently floor every new job too low, the
effective
value is shown on the job creation form, pre-filled and editable — a visible
default rather than a hidden one.

**An unparseable version** is treated as `0.0.0`, which under that floor means
the client is offered nothing and told to update. An *absent* version is not a
case at all: the claim body is required, so a claim without one is rejected
outright.

The assignment still carries `min_magpie_version` as a formatted string. The
server filtering is the mechanism; the client's own comparison stays as a
cross-check, because a client that somehow receives work above its version should
refuse it rather than run it.

**Version mismatch is a decline, not an exit.** If the client does receive a job
above its version — a server bug, or a race with a floor that was just raised —
it declines with `reason: "magpie_version"` and adds the job to its unsupported
set. The set is not "jobs whose data I lack"; it is **jobs I cannot do**,
whatever the cause. That generalisation resolves an older rough edge: an
unrecognised `job_type` used to mean "the server is newer than this MAGPIE, so
exit", and it is now one more reason to decline. A client that cannot do
`leave_generation` because it predates that executor can still play `games` all
day. Exit is reserved for the case where *nothing* is doable, which the scheduler
already detects.

**Shutdown says which remedy**, because "update MAGPIE" and "update your data"
are different actions. `reason` is `magpie_too_old`, `data_out_of_date`, or
`both` -- or `unsupported_build`, below. When both apply, say so but **lead with the MAGPIE version**: updating
MAGPIE is the remedy that fixes both, since a release bumps `DATA_VERSION` and
the contributor runs `download_data.sh` as part of updating. Telling someone to
fix their data first sends them on a trip they would have made anyway. The global
floor short-circuits all of this: a client below `MIN_MAGPIE_VERSION` gets
`magpie_too_old` on its first claim without any job being consulted.

So does the build. `BOARD_DIM` and `RACK_SIZE` are MAGPIE compile-time
constants (`make magpie BOARD_DIM=21 RACK_SIZE=8`), in neither the version nor
any digest, and every job here plays on 15×15 with 7-tile racks (job creation
refuses any other layout; rack spaces and leave tables are counted in 7). A
21×21 build failed every task loudly, five times and then a layout error to
decode; an 8-tile build failed nothing -- it drew eight tiles in games and gave
a 7-tile bingo no bonus in an opening-rack analysis, rows indistinguishable from
real ones. So every claim states both, and a build that is not 15/7 is answered
`unsupported_build` before any job is consulted, with a message naming what it
was built with; MAGPIE prints how to rebuild with the defaults and exits.

`task_claims.magpie_version` records what was reported at claim time. This is not
bookkeeping for its own sake: keeping birdtest's pinned rows in step with
MAGPIE's `DATA_VERSION` is a human decision, and this column is what informs it.
"How many distinct workers claimed anything in the last week, and what were they
running" is one query, and it is the difference between raising a job's floor on
evidence and raising it on hope. Together with `worker_data_gaps` it covers both
axes: who is behind on code, and who is behind on data.

### Operational consequences

**A job can now be created that nobody can run.** Pinning a job to a
just-imported tarball while every released MAGPIE still installs the previous one
means every worker declines it. That is no longer silent: the workers keep
contributing to other jobs, `worker_data_gaps` fills with a single repeated
answer, and the admin UI says which file and how many workers. The remedy is an
admin decision — wait for the MAGPIE release that bumps `DATA_VERSION`, or pin
the job to the older rows.

Visible is not the same as noticed, though, so the job list carries a **stalled**
badge: at least one decline and zero submissions in the last 24 hours, with no
active claims. A stricter variant catches a bad pin the same day it is made — a
job older than an hour with zero submissions ever and at least one decline. Both
are computed from rows already written, shown where an admin already looks. There
is deliberately no alert: notifying on the stalled transition is the only thing
that works when nobody is looking, and it needs a channel birdtest does not have.
Revisit if a job ever stalls unnoticed.

**The MAGPIE floor is what makes any of this binding.** A client that ignores
`expected_data` never declines and contributes unverified — capability
negotiation cannot route around a client that does not speak it. Two things stop
that: the claim body is required, so a client that does not send a version cannot
claim at all; and `min_magpie_version` is non-nullable with a real floor, so a
client that sends one too low is offered nothing.

**Adoption order follows cost.** Pin a `games` job first: it is the cheapest
place to discover that a resolution rule or a message is wrong, since a declined
claim costs one round trip. Then `leave_generation`, whose bad data propagates
into later generations and cannot be subtracted back out — and which, once the
mechanism is trusted, should never run unpinned.

**Keeping the pinned rows and `DATA_VERSION` in step is a human job.** birdtest
does not read `download_data.sh`, does not warn when an import is newer than what
the released client installs, and will not grow a mechanism for it — the coupling
is real but it moves at the speed of MAGPIE releases, which is slow enough for a
person to handle. What makes that workable is evidence rather than automation.
**This belongs in the admin runbook**: import a tarball only when a MAGPIE
release installs it, and check `task_claims.magpie_version` and
`worker_data_gaps` before pinning a job to it.

**An end-to-end test guards the two constants**, and it runs nightly rather than
per pull request (`.github/workflows/nightly.yml`, running
`scripts/e2e_magpie.py`): it compiles MAGPIE `birdtest-contribute` as
`portable_release` (the build the backend image and the MAGPIE release use, so
the wordmap it builds hashes the same as the server's; MAGPIE's Makefile
refuses `BUILD=release`, which is a PGO target with its own recipe), installs data
with that MAGPIE's `download_data.sh`, and runs one real task per job type against
a seeded stack. If birdtest's pinned rows name content MAGPIE does not install,
the client declines, the task never completes, and the job fails — so the mismatch
surfaces as a failed build rather than a dead job in production. It proves the pin
agrees with the MAGPIE in CI, not with the MAGPIE contributors are running;
`worker_data_gaps` covers the difference. It is nightly because building MAGPIE
and downloading its data is slower and more environment-sensitive than a pull
request should wait on; the per-pull-request workflow
(`.github/workflows/ci.yml`) covers the cheaper checks, including MAGPIE's half
of the message contract against this branch's fixtures.

### What this deliberately does not do

- **It does not constrain a hostile contributor.** A digest the client computes
  is a digest the client can fabricate, and a client can decline work it is
  perfectly capable of. This targets the real and current threat — an honest
  contributor with stale, missing, or off-channel files. Constraining a hostile
  one needs output-side checks: a task replicated to two contributors and the
  results compared (birdtest hands each task out once; for games between
  static players that solve nothing, which are deterministic, the comparison is
  of bytes; anything that simulates or solves is not, so a cross-check there
  would compare distributions) or occasional canary tasks
  whose answers the server already knows. Complements, not alternatives.
- **It does not secure the distribution channel.** `download_data.sh` fetches
  over HTTPS with no signature and no checksum. This detects, for pinned jobs,
  that what landed is not what birdtest expects. A signed manifest shipped with
  the tarball is the real answer and belongs in MAGPIE-DATA.
- **It does not verify the binary.** `min_magpie_version` stays the floor. A hash
  of the executable is close to useless — it differs by platform and compiler for
  builds that are semantically identical.
- **It does not serve data.** The server names the file a contributor is missing
  and the tarball to get it from; it does not hand over bytes. Import already
  downloads the tarball, so serving it through `GET /api/worker/artifact` is a
  small step — but it is a distribution feature with a storage cost and a
  redistribution question, and `download_data.sh` already exists.
- **It does not accept out-of-tarball files.** Every `input_data` row comes from
  an imported MAGPIE-DATA tarball. An admin endpoint that uploads arbitrary bytes
  was considered and deferred: it is the right answer the first time a real
  hand-built lexicon or layout needs pinning, and it is a door through which
  unverifiable bytes enter the vocabulary, so it is not being built for a
  108-byte test fixture.
- **It does not defend against a client that spins.** A client that declined a
  job and then failed to record that fact would re-claim the same task
  immediately, looping as fast as the network allows. Tracking the unsupported
  set is the entire point of the decline path, so a client that omits it is
  broken in a way that would not survive its first run. The per-worker rate limit
  bounds the damage incidentally, and `worker_data_gaps` records declines per
  identity, so the behaviour would be visible without anything being built for
  it.

---

## Low-Level Design

### Request Handling

The core of birdtest is the task claim endpoint — the sequence that runs every time a worker asks for work.

1. **Auth and verification**: The server reads the worker identity from request headers (`Authorization: Bearer <api-key>` for authenticated workers, `X-Worker-UUID` for anonymous workers). It verifies the worker is not banned. Resolving the identity, stamping its throttled `last_used_at` / `last_seen_at`, and checking the ban list are **one statement**, not three: this runs on every worker request, so each round trip here is on the critical path of getting a worker its next task. An anonymous identity is only ever *created* by a claim that hands out a task (see [Workers](#workers)).

2. **Job selection**: The server filters to active jobs above 0% that this worker can run — its MAGPIE version and its unsupported set, see [Scheduler side](#scheduler-side) — and orders them by `(claims_issued - claims_baseline) / allocation`, most behind its share first (the baseline is where the job's share is measured from; see [Workflow](#workflow), step 2). `jobs.claims_issued` counts every claim ever issued for the job, **including abandoned and declined ones**, so it only ever goes up; excluding abandoned claims would let it shrink as timeouts accrue and would unfairly favour jobs with flaky workers. Ties break on `created_at ASC`. No randomness is involved, and no priority: a job at 0% is offered to nobody, which is what inactive means.

   ```sql
   SELECT j.* FROM jobs j
   WHERE j.status = 'active'
     AND j.allocation > 0
     AND (j.min_magpie_major, j.min_magpie_minor, j.min_magpie_patch) <= ($1, $2, $3)
     AND j.id <> ALL($4)
   ORDER BY (j.claims_issued - j.claims_baseline)::float / j.allocation ASC, j.created_at ASC
   ```

   **Each candidate is checked for its turn when its claim holds the job's dispatch lock** (`scheduler::try_claim_from_job`): its ratio, as committed then, must not be more than one of that job's claims past any of the worker's other candidates still in play. Claims arriving together read the same list and all land on its first job; for a job at 1% each is a whole ratio unit, and 32 concurrent claims put it 32 units ahead, paid back only slowly and forgiven by anything that lifts a job that lags. A claim that is not the job's turn goes on to the next candidate, and one that finds every job with work outrun reads the list again, up to eight times; the eighth takes the first job with work without checking it against jobs that are not busy, so an `Idle` while work exists is left only to a busy job and a last-round race (with equal jobs and 32 workers claiming together, eight checked rounds told 67 to 166 claims of 1,920 there was nothing). A job whose dispatch lock is busy past its two seconds is not tried again in the request — waited on every round, it held each claim 16 s and ended it `Idle` — but stays a rival in every round, with a ratio unit of slack rather than one of its claims: a 1% job's claim is a whole unit, the largest there is, so a 50% job beside a busy one can still take fifty claims, and a 1% job beside a busy 99% one takes one or two (the check is on the ratio before the claim). Dropped as a rival, the busy 99% job let the 1% job take every claim of the spell, and its settling forgave them (49 where 30 is fair). And a job found busy is settled a ratio unit short for ten minutes (`BUSY_MEMORY`): the lead the jobs beside it took while it was busy — a unit at most — is owed back, and settled all the way it was forgiven, a unit more each spell (a 10% job took 400 of 2,400 claims over twenty spells beside a settling 90% one, where 240 is fair); not settled at all, a newcomer found busy once took the majority's claims in a split fleet (their job's first came 331st). The check is a lookup, but made holding the job's dispatch lock, so it queues behind the claims on it. The one claim of slack is what concurrency needs — with none only the lowest job could be claimed, and a fleet claiming together went idle a third of the time — and it is the rival's claim, so a 99% job can run a 1% claim ahead and the 1% job a hundredth.

3. **Lazy reclamation**: Before acquiring a task, any claimed tasks whose `last_heartbeat_at` (or `claimed_at`, if no heartbeat has been received yet) exceeds the heartbeat timeout, or whose claim is a minute past its `deadline_at` ([Task time limit](#task-time-limit)), are returned to `available`. One statement covers every candidate job rather than one per job: no index on `task_claims` leads with the job, so the planner reaches expired claims through the partial index on open claims — one entry per claim in flight across the fleet — and filters by job afterwards. Per job, a claim request paid that scan once per candidate for a set of rows that does not depend on the job at all. Skipped entirely while the process is younger than the heartbeat timeout — see [Task States](#task-states) for why a restarted server has to hear from the fleet before it judges it.

4. **Task acquisition** — strategy-dependent:
   - **Re-dispatch first**, under the job's dispatch lock like everything else here: `SELECT ... FOR UPDATE SKIP LOCKED` on the job's `available` tasks — a lapsed or declined claim's task — **excluding, outside leave generation, a task this worker declined in the last hour**, so a task that fails everywhere is not handed straight back to each worker that just failed it (`registry::next_available`). An `available` task has no live or completed claim — that would make it `claimed` or `completed` — so there is no slot of the worker's own to exclude.
   - **Otherwise generate**: produce the next task request for the job type and insert + claim it atomically in a single transaction.

5. **Response**: The server serializes the job-type-specific task request and returns it to the worker along with the claim token.

#### The claim loop in full

The steps above are the happy path. The whole exchange is one function returning
a four-way `ClaimOutcome`, mapped to HTTP once at the edge so the compiler
enforces that every branch is considered:

```
claim(identity, capabilities) -> Task | Idle | NoWorkExists | Shutdown
```

**Before any job is consulted**, the worker's build is checked -- a
`board_dim` or `rack_size` other than 15 and 7 is sent an `unsupported_build`
shutdown -- and then its version against the server-wide floor. A client below
it cannot run any job that could ever exist, so it is sent a `magpie_too_old`
shutdown without a single job being queried.

Then, up to **eight rounds** (`scheduler::CLAIM_ROUNDS`; a round is repeated when a leave generation's transition makes new work, when two claims race to create the same on-demand task, or when every job with work was outrun):

1. Select candidate jobs (the CTE above). Empty → decide between `Shutdown`,
   `NoWorkExists` and `Idle`.
2. Reclaim the expired claims of every candidate at once, then, for each
   candidate in deficit order, try to acquire a task from it. Acquisition returns one of these:
   - **Task** — for a worker that arrived with no identity, insert its
     `anonymous_workers` row; insert the claim, bump the task's counters and the
     job's `claims_issued`, commit, and return it with the job's
     `expected_data` from its template. The `claims_issued` update is guarded on the job still being
     `active`: a claim that selected the job before the stopping rule or an admin
     completed or deactivated it waits on that update's row lock, then finds the
     job no longer active and hands out nothing.
   - **NoWork** — this job has nothing to hand out; try the next candidate.
   - **Busy** — another claim held the job's dispatch lock past the bounded
     wait. The job has work, so it is not lifted as passed over; it is not
     tried again in the request, but stays a rival of the others with a ratio
     unit of slack, and the next candidate is tried.
   - **NeedsZeroGeneration** — leave generation only: generation 1 is due but
     the zeroed KLV it plays with was never written. Built on its own task;
     nothing to hand out until then, and the next candidate is tried.
   - **JobFinished** — leave generation only: its last generation is built;
     flip it to `completed` and try the next candidate (games, game pairs and
     opening racks complete through the finish checks instead). The flip is written inside the claim transaction,
     under the dispatch lock, so a purge cannot restart the job between the
     decision and the write.
   - **NeedsUniverse** — leave generation only: the current generation's rack
     universe has not been written. Roll back, start the seeding on its own task
     (it is millions of rows, and a request a client gives up on would roll it
     back), and try the next candidate.
   - **NeedsLeaveMerge** — leave generation only: nothing is left to hand out
     or in flight, but accepted results are still staged, so whether the
     generation is complete is not yet known. Commit (its one possible
     write, a sweep deleting a finished lap's cursor, would otherwise be redone
     by every claim until the merge landed), start the merge on its own task (it gives up at once if one is already running, so a fleet asking
     together starts one), and try the next candidate.
   - **NeedsGenerationTransition** — leave generation only. **Commit** (the
     transaction's only write is the row claiming ownership of the transition),
     start the transition on its own task, and move to the next candidate. The
     transition uploads an artifact and does a multi-megabyte build, so it must
     not run inside the claim transaction — and it is not waited for either:
     this job has nothing to hand out until it finishes, so holding the claim
     open for the tens of seconds it takes only makes one worker idle for all
     of it. It answers `204` and asks again.

   A candidate that fails outright — a missing config row, a leave-generation job
   with no generation-0 KLV — is logged and skipped rather than failing the claim.
   Otherwise one broken job at the head of the list answers every worker with a
   `500` for as long as it stays there, and every client retries those.
3. If no candidate produced work and nothing asked for another round, return `Idle`. The eighth round skips the turn check against jobs that are not busy (see step 2), so a claim is told `Idle` while work exists only when every job with work is busy, or more than a unit past a busy rival, or a race on a created task or a leave generation's transition falls in the last round.

**Acquiring a task takes the job's dispatch lock first**, for re-dispatching
an existing task as much as for generating a new one
(`pg_advisory_xact_lock`, per job, held for the rest of the claim
transaction). Every job type decides what to hand out next from reads a
concurrent claim's uncommitted writes are invisible to: games, game pairs and
opening racks address the next slice with `MAX(seed)`, and leave generation
additionally decides which racks are still out and whether the generation can
close. The lock costs nothing that was not already being paid — issuing a
claim bumps `jobs.claims_issued`, which holds that job's row lock until
commit, so claims against one job already serialize — and it turns a lost race
into a short wait. It is per job, so claims against other jobs are unaffected.

**Under the lock, only what changes from claim to claim is read.** Everything
a claim needs that the job fixed at creation — its per-type config row, its
letter distribution (parsed), its players' configs flattened into the shape a
request carries, its `expected_data`, and for opening racks the rack-space
table a range is unranked with — is a *template* (`jobs::dispatch::JobTemplate`)
read once per job per process and kept in memory (`AppState.templates`), the
way a dispatchable job's derived-file hashes are. It cannot go stale in what
it is used for: player configs are immutable, an `input_data` row cannot be
deleted while a job or a config pins it, and a job's config rows have one
update path, an opening-rack job's consensus settings
([Editing the consensus](#opening-rack-consensus)). The template keeps those
as the job was created, and nothing reads them from it: a claim reads them
under the job's dispatch lock, and a submission after taking its claim
(`ConsensusSettings::load`), and the edit holds the dispatch lock and every
open claim, so neither reads them mid-edit. A purge changes none of it;
deleting the job forgets it. Before the template, a games
claim made five reads of those rows inside the lock — the config, the
distribution, a three-way join per player and the six-table `expected_data`
union — and a re-dispatched task's request made six; now the reads under the
lock are the seed cursor, the available tasks, and the request row of a task
being reissued. The submit path uses the same template for the batch size a
task was dispatched with and the number of moves to keep per position, inside
the task's row lock, instead of reading the request row and the player config
again.

**The wait for it is bounded** (`lock_timeout`, two seconds), and a claim that
gives up tries the next candidate — without treating the job as having
nothing to hand out: a busy job is not lifted as passed over. It is not tried
again in that request, so a claim waits on it once, and it stays a rival with a
ratio unit of slack (`registry::Acquired::Busy`). Ordinary contention is milliseconds, so this is never reached in
normal operation; it exists for the one holder that is not ordinary. Seeding a
leave generation's rack universe is millions of rows and tens of seconds, and
the task seeding it holds this lock throughout — so without a bound every
other claim for that job blocks for the duration *while holding a pool
connection*, and the pool is twenty. One slow claim on one job would stall
submissions and the dashboard for the whole server. The bound turns that into
those workers being told to look elsewhere.

**And for the long holders, claims do not wait at all.** A bounded wait
still holds a connection for its whole two seconds, and a job that handed out
nothing fell behind its share and so headed every worker's candidate list (it
is lifted as it is passed over now): a fleet of idle workers polling every five
seconds held the pool on it for as long as a seeding ran. So a seeding, a purge or delete, and a consensus edit mark the job in an
in-process set for as long as they hold its lock (`jobs::DispatchHolds`), and a
claim skips a marked job before taking a connection. The advisory lock is still
what makes the hold safe; the set only spares the wait.

**One thing restarts an attempt**: a **unique violation while generating a
task**. Before the dispatch lock it was a lost race on `(job_id, seed)`, two
workers generating the same on-demand task at once, and it was the common case:
past three-way contention on one job a worker was answered `204` while work
existed. The lock ended that race. Every generation reads its seed cursor and
inserts under the job's dispatch lock, and a claim that cannot have the lock in
its bounded wait is answered `Busy` and generates nothing, so no two claims ever
generate the same cursor-seeded task; a purge takes the same lock. What is left
is leave generation, which draws each task's seed at random: two draws that
collide are a unique violation, and the claim re-runs selection and draws
again. (A leave-generation transition used to restart an attempt too; it now
commits, runs on its own task, and answers the claim with nothing.)

A second claim on one task is not a race the loop retries. A task is taken only
while `available`, under its job's dispatch lock and its own row lock, and
`task_claims_one_slot_idx` refuses a second live or completed claim outright: a
violation there means a task's state or counters drifted, so the claim fails for
that job — logged, and the job skipped for the request — rather than re-running
selection into the same task. (Under redundancy a task had several slots, and
two per-identity unique indexes stopped one worker taking two of them; a lost
race on those was retried. One slot made both unreachable, and the one-slot
index replaced them.)

The round cap (`CLAIM_ROUNDS`, eight) bounds the loop; exhausting it returns
`Idle`, and the worker simply asks again.

**Every claim records the MAGPIE version** the worker reported, on the
`task_claims` row. That is what makes "what is the fleet running" a single query,
and it is what a decision to raise a job's floor should be made on.

#### Deciding between shutdown and idle

Reached only when the candidate list came back empty, so the question is whether
any job is offering work at all and, if so, which axis ruled them out. **A job
offers work when it is active and above 0%**, and every row below counts only
those: a job parked at 0% is offered to nobody, exactly as an inactive one is,
so it can no more shut a worker down than an inactive one can. (It could, for a
day: the queries counted every active job, so a parked job whose floor was above
a worker's MAGPIE told that worker `magpie_too_old` over a job that was handing
out nothing to anyone — and a contributor who exits on that is not there when
the admin raises a job it could have run.)

| Condition | Outcome |
|---|---|
| No job offering work anywhere | `NoWorkExists` → `204` |
| Jobs offering work exist, nothing rules them out | `Idle` → `204` |
| Some offering job's floor exceeds the worker's version | `magpie_too_old` |
| Every offering job the version does not rule out is in the worker's unsupported set | `data_out_of_date` |
| Both | `both`, leading with the version |

An unsupported entry naming a job that is no longer offering work counts for nothing:
the set is client-supplied and may be stale, and a worker too old for every
active job must not be told its data is out of date as well.

`required_magpie_version` is the **lowest** floor among the jobs that are too
new — the smallest upgrade that would unblock anything, not the largest —
compared numerically rather than as text.
`required_tarball_dates` comes from the letter-distribution and layout rows of
the jobs the worker said it could not run, newest first. `download_url` is sent
only when a version is at fault.

### Result Submission

The mirror of the claim, and the only place results enter the system.

1. Look up the claim's job by token, requiring `state = 'claimed'` and the
   identity the claim was issued to, **without a lock** — a task never changes
   job — and decode and validate the payload against that job type's response
   shape, with nothing locked and no connection held. A token that matches
   nothing means the claim already lapsed and was reclaimed, or this result was
   already accepted: respond `200` with `{"accepted": false}` rather than an
   error. A malformed body is `400` naming what was wrong.
2. Look the claim up again **inside the submission's transaction and `FOR
   UPDATE`**, with the same conditions, and answer `{"accepted": false}` if it
   no longer matches. The lock is what makes that sound — checked only outside
   the transaction, a timeout could abandon the claim between the lookup and
   the write, leaving it both abandoned and completed and the task's live-claim
   counter decremented twice; decoded under it, as it once was, the claim and
   task rows stayed locked for the whole decode (KL-20, closed).
3. Normalize it into the record shape and insert it.
4. Mark the claim `completed`, increment `accepted_count`, decrement
   `active_claim_count`, and complete the task (it has one slot), stamping
   `completed_at`.
5. Commit. No audit row is written: the completed claim and the stored result
   already record the submission. Ratings are deliberately **not** touched
   here: a fit is global to a rating pool and nothing in this path depends on
   it, so it runs on a periodic sweep instead.
6. **After** the commit: evaluate the finish conditions on only the aggregates
   they need — the match test's statistics, or an opening-rack job's task
   counts. Inline, because the test decides whether the job keeps dispatching. The job row
   the submission's transaction read (step 1, before anything was stored) is
   the one the check uses, rather than a second read of it on the path the
   worker waits on. The
   completion is conditional on the job's `claims_issued` not having fallen since
   the job was read, ahead of its results: only a purge lowers it, so a check that
   read results from before a purge does not complete the job the purge just
   restarted — and a completed job cannot be reactivated. Best-effort:
   the result is already committed, so a failure here is logged and the worker
   is still told `accepted: true` rather than invited to retry a submission that
   landed.
7. **Off the request entirely**: the live stats payload, when the job has an SSE
   subscriber. It is display-only — nothing in the claim path reads a statistic
   — and it is the most expensive thing in this path, several aggregates over
   the job's whole history. Built before answering, it made the worker's next
   claim wait on a dashboard nobody may have open. It is **coalesced per job**:
   the first submission to find no push running owns one, later ones only mark
   it to go round again when it finishes, so a busy job builds one payload at a
   time instead of one per submission, and the pushes stay ordered because one
   task issues them, at most once per `JOB_STATS_CACHE_SECONDS` (10 by default).
   The dashboard can therefore lag up to that interval behind, which is the
   intended trade; an admin's change is pushed at once.

#### What a submission has to satisfy

Validation is per job type, and is the server's only defence against a
submission written straight into the largest tables in the schema. A body over
64 MiB is refused before it is parsed (`413`); batch size is the admin's lever for
staying under it, and job creation bounds it: at most 10,000 games a task (a
pair is two), 1,000 when the job captures positions — past 32,768 captured
games a result could not name its games at all.

**Games and game pairs.** `wins + losses + ties == games`, all non-negative, on
every aggregate. A `games` result must contain at least one game. A `game_pairs`
result must additionally carry a `pentanomial` whose five counts are
non-negative and agree with the game aggregate on both the pair count and player
1's half-points — the cross-check described under [MAGPIE reports the
pentanomial](#magpie-reports-the-pentanomial). `divergent_games` is optional; when
present its own counts must be consistent, its `games` even, and no larger than
the total. A plain `games` result carrying either has it ignored rather than
stored, since a job that does not play pairs has no pairs to describe. The
pentanomial must also agree on the **draws**: a pair scoring one or three
half-points holds exactly one draw and a pair scoring two holds none or two, so
the ties must be buckets 1 and 3 plus an even number no larger than twice bucket
2 — `[0,1,0,1,0]` beside two wins, two losses and no ties has the right pair
count and score, and is two pairs that each needed a draw. The schema's
`game_results_pentanomial_all_or_nothing` check holds all three identities too,
so a row written any other way cannot contradict itself either.

**The body is decoded once, straight into the job type's response**, from the
raw text kept by the extractor (`serde_json::value::RawValue`). It went through
a `serde_json::Value` first, which holds every object as a B-tree node and every
key as its own allocation — ten to twenty times the JSON's size, so a result near
the 64 MiB ceiling became most of a gigabyte on a 2 GB task, and a few at once
were enough to have it killed. A result of more than 1 MiB (one the body budget
reserves for, § "What bounds a submission?") also waits for one of
three slots (a quarter of a gigabyte each at the ceiling), for up to thirty
seconds before its worker is answered `503`; ordinary results never wait. The
wait is before the submission's transaction opens and the slot is held until
it commits — waited for inside it, as the twelfth audit first had it, each
waiter held a pool connection and its claim and task rows, and the slot ended
before the insert that still held the record.

**Captured positions**, when present:

- `game_index` must fall inside the batch the task actually dispatched. The batch
  size is the only thing that legitimately bounds this.
- `turn_number` must be within a generous per-game ceiling (400), so a malformed
  number cannot masquerade as a valid one.
- Every position must carry at least one ranked move.
- Its CGP is at most 4,096 characters; the play that led to it, like every
  play, at most 256; that play's score, like every play's, 0 to 100,000.
- A job that captures positions must get some from **every game of the
  batch** (`registry::refuse_uncaptured_positions`): a result with none, as a
  MAGPIE from before capture sends, completed its task and left a hole in the
  corpus for good (the audit's pass 21). A job that does not capture them
  refuses any.

**Opening racks.** At least one rack, and every rack must carry at least one
move — the moves arrive ranked best-first, so an empty list means nothing was
analysed and there is no best move to record. The submission must also name
**exactly the racks the task dispatched**, as a set; see below.

**Leave generation.** At least one rack occurrence.

**On top of all of the above, the plausibility rules** in
[`plausibility.rs`](backend/src/jobs/plausibility.rs) — finite score moments, a
non-negative standard deviation, bounded play scores and probabilities, racks of
1–7 tiles (a bracketed tile at most 8 characters), plays of at most 256
characters, no rack listed twice in one leave submission, and no NUL in any
string (Postgres cannot store one, and the insert failed as a `500` that MAGPIE
retries) — reject impossibilities
rather than oddities. See [Why impossibility, and not per-worker anomaly
detection](#why-impossibility-and-not-per-worker-anomaly-detection) for the
reasoning and the full table.

Some cannot run in the pure validation step, because they need the job's
settings or the task's own row. They run in `decode_result`, from the job's
template (the batch size, a leave result's occurrence total, positions only
when the job captures them and then from every game), and in `store_result`, from the task's row (an
opening-rack batch's racks). Three of them are the only submission-time checks
that catch a worker reporting work it did not do:

- **A game batch must report exactly the games the task dispatched**
  (`num_games`, doubled for pairs).
- **An opening-rack batch must analyse exactly the racks the task dispatched.**
  The request names the racks rather than only how many, so the whole set is
  compared rather than its size; order is not part of the contract. The range is
  re-expanded from `rack_start`/`rack_count` against the job's pinned
  distribution, which costs what dispatching it cost. This matters in both
  directions: too few racks and the task still *completes*, but a rack it
  skipped has no `opening_rack_progress` row (a rack has one from its first
  analysis), so nothing reissues it, and the job, whose finish check counts
  settled racks against the space (`OPENING_RACK_FINISHED`), never completes;
  racks from nowhere are stored as analyses of
  this job and added to `jobs.racks_analyzed`, the progress counter the
  dashboard reads.
- **A leave batch must not report more rack occurrences than its games could
  have drawn** (`plausibility::check_rack_occurrence_total`): the total across
  the submission is held to `num_games` × 1,000, where MAGPIE records at most
  two racks a turn. The other leave rules bound a count from below only, and
  this is the job type where an unbounded one is a wedge rather than a wrong
  number: occurrences are summed into `bigint` columns by a merge that folds
  everything staged for the generation (in passes over slices of the racks),
  so a count out of a broken client's uninitialised buffer — as likely near
  2^63 as anywhere — made the merge fail with `bigint out of range` on every merge from then
  on, including the drain no transition closes without. The generation could
  not close until someone deleted the staged row by hand.

#### How much of an analysis is stored

The server keeps the leading `num_plays_recorded` moves per position, read from
the player config that produced them — the same number that told the worker how
many to report. It is required on every player config (at least 1), so the
worker and the server can never disagree about it. `num_moves` on
the record preserves how many were actually ranked, so the discarded tail stays
visible as a count.

Per-ply rows are written only where the worker reported them, which in practice
means only for simming players; a static player produces none and the table stays
empty rather than filling with placeholders. They are kept to the plies the config
records, the way moves are kept to the plays it records — `ply` below the player's
`num_plies_recorded`, the larger of the two players' for a captured position —
and must be numbered from 0 in order, with a bingo percentage in [0, 100] and an
average score no play could not reach. Nothing bounded them before: a 64 MiB body
could carry tens of thousands of ply rows per move. A ply row is keyed by
`(move_id, ply)`; it carried a `BIGSERIAL` id beside that key which nothing read,
at some 30 bytes a row — 2 to 5 GB for one simming opening-rack job.

Every position lands or the submission fails: a duplicate within one
submission is refused before the insert (two analyses of one rack, two
positions for one turn of a game), and with one slot no other claim can have
written the same rows. (In-game positions were inserted with `ON CONFLICT DO
NOTHING`, their moves skipped with a no-op, while a task could be run more than
once; the unique keys now make a duplicate fail the submission instead of
dropping rows.)

**Positions, moves and plies each go out in multi-row statements**, not one
statement per position. An opening-rack task carries `racks_per_batch`
positions — 500 by default and up to 10,000 — so a statement each meant
thousands of round trips inside the submit transaction, holding the task's row
lock for all of them. Batched, it is a handful. Two details make it correct
rather than merely faster: a plain multi-row insert returns its rows in the
order they were given, which is what lines record ids up with the positions
they came from and move ids up with their per-ply statistics; and the
conflict-ignoring path returns a *subset*, so those rows are matched back on
`(game_index, turn_number)` — the columns the partial unique index is on —
rather than zipped.

### Task Claim

A task claim is the message a worker sends to initiate the exchange. It carries no job-type-specific payload — the server decides the assignment. The worker's identity and auth are conveyed via request headers; the body carries only what the worker says about itself — `magpie_version`, `board_dim` and `rack_size`, which are required, and `unsupported_jobs`, which is empty when omitted — and is required (see [The Worker API Contract](#the-worker-api-contract)).

The server responds with the task request for the assigned job type and a claim token the worker must include when submitting its result.

### Job Type System

The core architectural pattern is a **job type registry**: a closed set of job types where each type defines four components. Adding a new job type requires implementing all four; the compiler enforces completeness via exhaustive matching.

The stored form of a processed task response is called a **task record** throughout this document.

### The Four Components

Each job type defines:

| Component | Description |
|---|---|
| **Task request** | Serialized and sent to the worker when it claims a task. Contains everything the worker needs to perform the work. |
| **Task response** | Deserialized from the worker's submission. The raw output of the work, validated on receipt. |
| **Task record** | The normalized form stored in a typed record table (one table per record type). Derived from the response; may omit fields, recompute derived values, or canonicalize formats. |
| **Creation strategy** | How tasks for this job type are generated. Every type is **on-demand** now (see below); the component survives as each type's claim-time generator. |

### Task Request Types

A task request is inserted into a typed request table at task creation time (in the same transaction as the `tasks` row), which for every job type is claim time.

Some request types are shared across job types:

| Type | Used by |
|---|---|
| `OpeningRackRequest` | Opening rack |
| `GameRequest` | Games, game pairs |
| `LeaveRequest` | Leave generation |

### Task Response Types

A task response is what the worker submits after completing a task. It is validated on receipt and then transformed into a task record for storage. Response types may differ from their corresponding request types (e.g., a single seed request may yield a batch of game results). Response and record types are shared across job types where the stored shape is identical regardless of how the task was generated — games and game pairs both submit the aggregate MAGPIE's autoplay reports for a batch (`{games, wins, losses, ties, score means and standard deviations}`), with game pairs adding the pentanomial over every completed pair and a second aggregate over the divergent ones. Autoplay does not emit individual games, and nothing downstream needs them: the match test and the dashboard both work off counts.

| Type | Used by |
|---|---|
| `PositionAnalysisResponse` | Opening rack analysis — one entry per rack in the batch |
| `GameResultsResponse` | Games, game pairs — one aggregate per batch, plus the pentanomial and a divergent-pairs aggregate for game pairs |
| `LeaveResponse` | Leave generation |

### Task Record Types

A task record is the normalized form stored in a typed table after a response is accepted. It may omit raw fields, recompute derived values, or canonicalize formats. Task record types may be shared when the stored shape is the same regardless of how the task was generated.

| Type | Used by |
|---|---|
| `PositionAnalysisRecord` | Opening rack analysis |
| `GameResultsRecord` | Games, game pairs |
| `LeaveRecord` | Leave generation |

### Creation Strategies

**Every job type is on-demand.** No tasks are inserted at job creation. When a worker requests a task, the server generates the next task request, then inserts and claims it atomically in a single transaction, issuing a claim token.

Opening rack analysis was the last pre-populated type. It became on-demand once tasks addressed *ranges* of the rack space rather than materializing a row per rack, and with it the pre-populated strategy disappeared entirely — there is no `CreationStrategy` any more, and one fewer axis on which job types differ.

### Per-Job-Type Creation Details

#### Opening Rack Analysis — On-demand, range-addressed

Each task covers a contiguous batch of `racks_per_batch` racks. One rack per task would spend a claim/submit round trip on each, and the space is large: the distinct 7-tile racks drawable from the English bag number **3,199,724**. At roughly one worker request per second, a rack per task would cap a single worker below one rack every two seconds before any analysis happened.

Nothing is enumerated up front. A task names the range `[rack_start, rack_start + rack_count)` and the racks are **unranked** from those indices on demand: a small dynamic-programming table over the letter distribution — how many k-tile racks can be drawn from tiles `i` onward — is enough both to count the space and to address the k-th rack in it directly, so producing one rack is a handful of additions rather than a walk over the millions preceding it. Job creation over the full English space is therefore constant time and writes no rows.

Ranges tile the space exactly as game seeds do, reusing `tasks.seed` as the starting index and the `(job_id, seed)` unique index to resolve two workers racing for the same slice. `total_racks` is computed once at job creation from the same table, so the scheduler knows when the space is exhausted without re-deriving it.

The rack size is a job setting (`rack_size`, 1–7, default 7) rather than a
constant: the space, the unranking and `total_racks` are all computed at that
size, so a job over 2-tile racks is the same code over a much smaller universe.
The figures above are for the default.

A short rack is analysed as the player's whole rack on an opening board with a
full bag: the moves it can make now, and in a simulation the plies after them
(a play draws back up to 7; a pass keeps the short rack). Neither situation
arises in a real game; it is the question the job asks. The request does not
restate the size, so MAGPIE requires only a rack of at least one letter it can
draw (it required a full rack for one pin, which refused every task of a
short-rack job; AUDIT_FINDINGS_19).

The unranking order must stay stable: results are recorded against racks expanded from an index, so changing it would silently re-point existing results.

At claim time (all in one transaction):
1. Compute the next start: `SELECT COALESCE(MAX(seed) + $racks_per_batch, 0) FROM tasks WHERE job_id = $job_id`. If it has reached `total_racks`, the job has no work left.
2. Unrank that range into racks.
3. `INSERT INTO tasks (job_id, seed, state) VALUES ($job_id, $next_start, 'available')` — the range's start is the task's seed: rack `i` of the batch is analysed from `seed + i`, its index in the job's rack space, which is the same number on every worker.
4. `INSERT INTO opening_rack_requests (task_id, variant, letter_distribution, board_layout, rack_start, rack_count, player_config_id)` — the range, not the racks. There is no lexicon column: the player config carries it.
5. Return the expanded racks, the seed, and a claim token.

#### Opening-rack consensus

A job can ask for a rack to be analysed until its analyses agree on its best
move. A simulation samples, so two analyses of the same rack -- on two
workers, or from two seeds -- can rank it differently; one analysis says
nothing about how settled its answer is. Three settings on
`job_opening_rack_config`:

- `min_results_per_rack` (default 1): the fewest analyses a rack gets;
- `consensus_pct` (default 100, above 50): the share of a rack's analyses
  whose rank-1 move must be its most common rank-1 move;
- `max_results_per_rack` (default 1, at most 100): the most analyses a rack
  gets.

They are the only settings of a job that change after it is created (see
**Editing the consensus**, below). Claims and submissions read them fresh
(`ConsensusSettings::load`) rather than from the job's cached template.

A rack is **settled** once it has at least the fewest analyses and they agree
in that share, or once it has the most -- then *without a consensus*, on its
most common best move (the alphabetically first of a tie). A settled rack is
not analysed again, unless an edit of the settings unsettles it. One and one is one analysis per rack, the job as it was;
a static player may ask for nothing else, since its analyses are deterministic
and always agree (job creation refuses it). The share is above half so that
only one move can hold it.

This replaced `redundancy` for opening racks (removed October 2026): that ran
every *task* N times and compared nothing, where this re-analyses only the
racks whose answer is in doubt, and stops as soon as it is not.

**Dispatch.** The first pass is unchanged: ranges tiling the space. Once the
cursor reaches `total_racks` it keeps stepping by `racks_per_batch`, and a task
there **reissues** up to a batch of unsettled racks, listed explicitly
(`opening_rack_requests.racks`), its seed the cursor -- so rack `i` of it is
analysed from `seed + i`, a seed no earlier analysis of it used, and the worker
contract is unchanged (a request has always carried its racks). Under the
job's dispatch lock `opening_rack::next_reissue` picks the unsettled racks with
the fewest analyses, excluding any rack another reissue holds, and preferring
racks the claiming identity has not analysed: when too few are left that it
has not (a small fleet, the last racks) it fills the batch with ones it has,
since each analysis has its own seed and a rack wanting more analyses than the
fleet has workers would otherwise never settle. A lapsed reissue goes back to
`available` with its list and goes out again before anything new.

The preference looks at a **window**: the first four batches of candidates
(unsettled, not in flight, fewest analyses first) are asked whether the
identity analysed them, the unseen go first, and nothing past the window is
looked at. As first built it walked the unsettled racks until it had found a
batch the identity had not analysed, a probe of that rack's analyses and their
claims per rack stepped over, under the dispatch lock. An identity is an
account -- every machine under one API key's account is one identity, which
early on is the owner's whole fleet -- and one that analysed the whole first
pass has analysed every rack: each of its reissue claims walked the job's
every unsettled rack (3.2 million for English), found nothing, and only then
filled its batch, while every other claim for the job waited on the lock. The
racks in flight are excluded with `NOT IN (SELECT unnest($array))`, a hashed
subplan in every plan; `<> ALL($array)`, as it was, is hashed only in a custom
plan, and a prepared statement's generic plan compared every rack stepped over
with every rack in flight -- which are the racks at the front of the order.
Two more reads were bounded by the thirty-third audit's second pass. The racks
in flight are read from what is open -- the reissues given back, through the
queue index, and those claimed, through the open claims' index -- where the
seed index found every reissue the job had ever made and checked each one's
state on the heap, a cost that grew with the job's reissue history (up to a
million reissues for a full English job at small batches). And the "unseen
first" sort key is a scalar subquery, `(SELECT 1 … LIMIT 1) IS NOT NULL`,
rather than an `EXISTS`: Postgres may plan an `EXISTS` as a hashed subplan,
which reads every analysis the identity made in the job, and does when its
statistics understate the records -- just after a `pg_restore`, which restores
none (RUNBOOK §5 analyzes the restored database before the service starts) --
while a scalar subquery is always a probe per rack.
(Measured in [What these reads cost](#what-these-reads-cost-measured).)

**State.** `opening_rack_progress (job_id, rack)` holds each rack's analyses,
its top move and how many ranked it first, `settled` and `without_consensus`,
written in the transaction that stores an analysis: `record_consensus` reads
the rack's rank-1 moves through the `(job_id, rack)` index -- at most
`max_results_per_rack` records a rack -- and upserts the row. **Every
opening-rack job keeps the rows**, one wanting one analysis per rack too: each
of its racks settles at its first, and its row is what an edit raising the
maximum later reissues from. (They were kept only above one analysis per rack
until the consensus became editable: an edit across one would then have had
no rows to reissue from, or, the other way, counted reissues in flight as new
racks.) Only reissuing depends on a maximum above one (`reissues()`). The
cost is one row per analysed rack, and the index on unsettled racks stays
small for a one-analysis job, every row of which is settled. The job's
`racks_analyzed` (racks with an analysis), `racks_settled` and
`racks_without_consensus` are running totals bumped with the rest of the
job's row, and the job completes once `racks_settled` reaches `total_racks`
with nothing in flight. The page's progress bar is racks settled; its rack
lookup numbers each analysis of a rack and says what they agree on; an export
line carries its rack's standing (for a job whose maximum is above one, and
for a rack with more than one analysis whatever the maximum is now: a job
whose maximum was lowered to one still counts those racks' disagreements in
`racks_without_consensus`).

**Editing the consensus.** `PATCH /api/admin/jobs/:id/consensus` takes any of
the three settings; only those sent change, and none that differs is a `200`
that writes nothing. It refuses what creation refuses (the same
`consensus_problems`, and a maximum above one for a static player), and any
job but an opening-rack one. Under the locks a purge takes, in its order --
the job's dispatch lock, its open claims, its row -- and the config row `FOR
UPDATE`, and under the hold a purge takes (`jobs::DispatchHolds`, not counted
as a purge), on a task of its own as a purge runs (`run_to_completion`), it
updates the settings and **restates every rack**
(`opening_rack::restate_racks`) by `standing()`'s rule: agreed is at least the
fewest analyses with the top move's share at least the consensus; settled is
agreed or at the most. `racks_settled` and `racks_without_consensus` are reset
from the rows. Raising the settings unsettles racks; lowering them settles
racks. Then the job follows:

- **Completed, with racks now unsettled: it reopens, inactive at 0%**
  (`job.deactivated`, from `completed`). A completed job holds 0% and keeps
  nothing of its old share, and the fleet may have been given to other jobs
  since, so it waits for the admin to give it an allocation on
  `/admin/allocation` -- which joins it at parity and requests its derived
  data, as any activation does. (It used to come back active at its old
  allocation when the other jobs left room, and inactive otherwise.) Its final
  exports become snapshots ([Exports](#exports)).
- **Completed, with every rack still settled:** it stays completed, and its
  final exports still become snapshots, since the standings their lines carry
  were restated ([Exports](#exports)).
- **Active, with every rack now settled:** it completes at once if nothing is
  in flight, or with its last in-flight claim's submission.
- **Inactive:** its status stays; it completes when next activated if nothing
  is left.

A **force-completed** job is a completed job like any other here, and that has
a consequence nobody has decided on yet. The server completes a job only once
every rack of its space is settled, so reopening one hands out unsettled racks
and nothing else. An admin can force-complete a job part-way through its first
pass; an edit that then leaves any analysed rack unsettled -- which a job
seeking a consensus, stopped mid-run, always has -- reopens it, and
`next_request` resumes the first pass where it stopped (possibly millions of
racks the admin had stopped) before it reissues anything. A force-completed job
wanting one analysis per rack, whose analysed racks are all settled, is
reopened only by an edit that unsettles them. Whether an edit should undo a
force-complete at all, or reopen such a job inactive, is pending a decision
(the October 2026 audit, B-2); until then this is the behaviour.

**What the edit holds, and for how long.** Restating a full English job
rewrites up to 3.2 million progress rows (an edit raising the minimum
unsettles every one, none of them a HOT update, since `settled` is in the
unsettled index's predicate) -- minutes on the default instance, all of it
under the job's dispatch lock and every open claim. It took them without the
hold until the October 2026 audit, so every claim considering the job waited
out the dispatch lock's two seconds and every submission for one of its claims
its five, each on a pool connection: the stall the hold was built for. With
it, claims skip the job, submissions and declines for its claims are answered
`503` at once, and a second edit, a purge, a delete or a lifecycle action on
the job meanwhile is a `409`. The hold is released before the edit's own
finish check (an active job left with every rack settled), whose purge
witness would otherwise take the held job for one being purged and roll the
completion back. Three races with the edit are closed where they land:

- A finish check that read the job settled before the edit, and whose
  completion then waited on the edit's row lock, completed it with the racks
  the edit had unsettled. `complete_unless_purged` re-checks `racks_settled`
  against `total_racks` in its own `UPDATE`, which Postgres re-evaluates on
  the row the edit committed (`I-OR-EDIT-3`).
- An export finishing between the edit's `unfinalize` and its commit read the
  job's committed `completed` and was stored final for a job active again.
  `mark_ready` reads the job's status `FOR SHARE`, which waits for the edit
  and reads what it committed (`I-EXPORT-15`).
- That left the other order (the thirty-third audit's second pass): an export
  whose snapshot was read while the job was completed, still uploading when
  the edit reopened the job and when the few racks it unsettled had been
  analysed and the job had completed again. `mark_ready` then read
  `completed` and stored the corpus from before the edit's analyses as the
  job's final one, which the results stream redirected to until someone
  exported again -- and a large corpus uploads for minutes, while an edit that
  unsettles a handful of racks is re-completed within one reissue's analysis.
  `unfinalize` now also fails the job's running exports, so their
  `mark_ready` matches no row and removes their objects; the admin exports
  again (`I-EXPORT-16`). It does not deadlock with one waiting: `mark_ready`
  reads the job's row `FOR SHARE` before it locks its own, and re-reads that
  row once the edit commits.

**What an edit takes no hold for.** The request is checked, and one that
changes nothing is answered, before the hold or any lock is taken: a job that
is not an opening-rack one, settings creation would refuse, or the settings
the job already has. It is checked again under the locks, since the settings
can change between. Checked only under them, as it was until the same pass, a
refused edit -- a games job, a typo in the share -- or a double click's
unchanged second one held the job's claims and submissions off while it took
its locks, and then, since the hold always ends with the reclaim grace, kept
the job's lapsed claims from being reclaimed for a heartbeat timeout
(`I-OR-EDIT-5`). And once the edit has committed, a failure to queue the
reopened job's derived data, or to reload the job for the answer, is logged
rather than answered `503`, as the purge's are: a 5xx for a committed edit
skipped its finish check and invited a second edit.

The audit row is `job.consensus_changed`, its reason the changes and what they
leave ("min 1 -> 2, max 1 -> 3; 4 racks unsettled"), with the status change
beside it when there is one. The response is the job, its settings, the racks
left unsettled, whether it reopened, and why it reopened inactive. The admin
job page's **Consensus** card makes the same edit.

#### Games — On-demand

Each task represents one batch of games (`games_per_batch` from the job config) played starting at a given seed. MAGPIE seeds a random stream with S and draws each of the batch's N game seeds from it, so a task's games are fixed by S alone. Consecutive task seeds are spaced `games_per_batch` apart, which keeps them unique and ordered.

At claim time (all in one transaction):
1. Compute next seed: `SELECT COALESCE(MAX(seed) + $games_per_batch, 1) FROM tasks WHERE job_id = $job_id`. This yields seed 1 for the first task, then `1 + games_per_batch`, `1 + 2*games_per_batch`, etc. This read and the insert in step 2 run under the job's dispatch lock (see [The claim loop in full](#the-claim-loop-in-full)), so no two claims compute the same seed; the unique seed index is a backstop, and a violation would re-run selection.
2. `INSERT INTO tasks (job_id, seed, state) VALUES ($job_id, $next_seed, 'available') RETURNING id`; the claim in step 4 moves it to `claimed` in the same transaction, through the counter update every claim uses.
3. `INSERT INTO game_requests (task_id, variant, letter_distribution, board_layout, seed, num_games, capture_positions, player1_config_id, player2_config_id)` — denormalize the job's settings so the worker receives a self-contained request. Each player config carries its own lexicon. The job's `threading_mode` is not repeated per task: the request takes it from the job's config, which never changes.

   The seed crosses the wire to the worker as a **decimal string**, not a JSON number. It is a full `uint64`, and JSON numbers are doubles, so any client using a conventional JSON library would silently lose precision above 2^53. It is stored as a signed `BIGINT` and reinterpreted at the application layer as before.
4. `INSERT INTO task_claims (task_id, claim_token, state, claimed_by_..., deadline_at)`, the deadline from `settings.max_task_seconds` in the same statement.
5. Return the request + claim token to the worker, with the job's name and the claim's time limit.

Match-test and finish-condition checks run during result submission, not at claim time.

---

#### Game Pairs — On-demand

Same as games, except the batch size is `pairs_per_batch` from the job config. Each task seed is spaced `pairs_per_batch` apart: `SELECT COALESCE(MAX(seed) + $pairs_per_batch, 1) FROM tasks WHERE job_id = $job_id`. The job type sets MAGPIE's `-gp` flag, so both orderings of each seed are played in a single invocation.

Results are a `GameResultsResponse` — the same type games use — carrying the aggregate over every game played, the **pentanomial** over every completed pair, and the divergent subset. The match test runs on the pentanomial: the pair is the independent unit (the two games share a seed), and it is also the unit `min_pairs` and `max_pairs` bound, so the sample size and the progress count are the same number. The divergent aggregate is stored and displayed as a diagnostic of how often the two configs differ, and nothing is tested on it — see [The pentanomial, and why pairs are the unit](#the-pentanomial-and-why-pairs-are-the-unit).

---

#### Leave Generation — On-demand, partitioned generations

Leave generation has sequential phases: generation N must complete before generation N+1 begins. Within a generation, work is **partitioned across many parallel workers**: each task forces a different subset of the racks that still need occurrences for the current generation and plays a bounded batch of games. A generation is not "one worker, one task" — it's many small tasks that collectively drive every rack up to the generation's occurrence target. The job states one target per generation — `target_rack_counts`, MAGPIE's own `leavegen 100,200,500,…` shape — and the list's length is how many generations it runs; a later generation, playing better leaves, can be sampled harder.

**State**: Per-rack occurrence progress *within* the current generation is tracked directly in Postgres, in `leave_rack_progress (job_id, generation, rack, occurrence_count, equity_sum)`. An accepted result is **staged** (`leave_rack_staging`, one row per task) and folded into those totals by a **merge** (`leave_gen::merge_staged`) — periodically, when a claim finds the generation nearly done, and always before a generation closes — rather than by the submission itself; see "On result acceptance" and [What a merge costs](#what-a-merge-costs). The generation's live counters (`leave_generation_progress`) move with every accepted result. None of it needs anything from MAGPIE beyond what already exists. The output of a *completed* generation (a combined KLV, built server-side once every rack has reached target — see Aggregation below) is stored in S3 and referenced by `leave_generation_artifacts.artifact_key`; the next generation's tasks receive that artifact key as input.

**The racks are full racks.** MAGPIE's `RackList` forces, counts and reports full 7-tile racks, never leaves, and derives leave values from them itself. So `leave_rack_progress` holds one row per full rack the distribution can draw — 3,199,724 for English — seeded at zero, unranked in chunks and bulk-inserted: each generation's on a task of its own, started by the first claim that finds that generation's universe missing — generation 1's included, so creating or purging a job writes none of it. There is no `max_leave_size`: the leave domain of the KLV is always every leave of 1–6 tiles, and what is tracked is always every full rack.

**Two operations here outlast an ordinary HTTP request**, which is a deployment
constraint, not just a performance note. Writing a generation's 3.2 million rows
(`COPY` rather than batched `INSERT`s: 37 seconds against 54 on a developer
machine) — every generation's, the first included, by a task the first claim to
find them missing starts, so creating or purging a job writes none of them — and
a generation transition, which takes tens of seconds. A proxy that gives up on the
request makes axum drop the handler future, which would roll back a seeding
part-way, or abandon a transition part-way on *every* attempt — so a generation
whose transition outlasted the timeout would never close at all. Hence two things
together: the seeding and the transition each running on its own task rather than
inline in the request (see Aggregation below), and the ALB's `idle_timeout` set to 300
seconds rather than its 60-second default, which is comfortably above MAGPIE's own
120-second stall timeout (a request that makes no progress for two minutes fails;
one still uploading does not — see "A request fails on a stall" under
[Per-job-type executors](#per-job-type-executors)). SSE streams are
unaffected either way, since they send keep-alives every 15 seconds.

At claim time:
1. Determine the current generation: the lowest generation number that hasn't been marked complete. If none exists and as many generations as `target_rack_counts` lists are already done, return "no work."
2. Check that the generation's rack universe exists — every generation's, the first included, is written by a task the first claim to find it missing starts. That check is one indexed `EXISTS`. A claim that finds the universe missing rolls back, starts the seeding on its own task, and treats the job as having no work yet: the seeding takes the job's lock without waiting and holds it while it writes, so no claim reads a half-written universe, and a seeding a client or a deploy interrupts rolls back whole and is started again by the next claim. (It used to run inside the claim itself, where MAGPIE's 120-second request timeout could cancel it — on a database slower than that at writing 3.2 million rows, every claim restarted it and none finished.) Then select up to `racks_per_task` racks below the generation's own target (`target_rack_counts[generation]`), in one of two ways (`leave_gen::next_step`), chosen from the generation's summary row — how many racks were below target as of the last merge, the same age as the counts both selections read.

   **While many racks are below target — more than a hundred tasks' worth (`SWEEP_WHILE_TASKS_REMAIN`) — a sweep.** The generation's racks are handed out in primary-key order from a cursor remembered between claims (`leave_selection_cursors`, one row per generation, read and written only under the job's lock), one *lap* over the universe at a time, skipping racks already at target. A lap **starts only with no claim of the generation in flight and nothing staged**. From there every rack that is out — forced by an open claim, or by a result not yet merged — was handed out during this lap and so lies behind the cursor, and nothing ahead of it is out: a claim needs no list of what is out, and selection costs the same with one result staged as with ten thousand. (A task whose claim lapsed is reissued as it stands, before anything new is selected, so its racks stay behind the cursor with it.) Each selection reads one rack more than a task holds, so the task that takes a lap's last racks knows it and deletes the cursor in its own transaction; after that the job hands out nothing until the lap's last results are in and merged, and the next lap selects on exact counts. That pause is one task's duration and one merge per lap — some 6,400 tasks for English — and it is the wait that already precedes closing a generation, which is simply a lap that starts and finds nothing below target. (One task's duration when every worker holding one of the lap's last tasks is alive. When one is not, it is the heartbeat timeout for that claim to lapse plus a whole task for whoever is reissued it, with the job handing out nothing meanwhile; it is left that way on purpose for now — see [Known Limits and Open Questions](#known-limits-and-open-questions), KL-15.) A cursor lost to a purge or a partial restore is a lap not started: the same rule applies and nothing is handed out twice. A claim that hands out nothing commits rather than rolls back, so a lap found finished stays found.

   This replaced lowest-count-first selection for the bulk of a generation because of what that costs between merges: `leave_rack_progress` shows a finished task's racks at the counts they had before it played, so they are the *lowest* in the generation the moment their claim completes. They have to be held out — or they are handed straight back out, to every claim until the next merge — and holding them out meant every claim hashing and stepping over every staged rack, about a microsecond each, inside the dispatch lock: 160–290 ms with 400 results staged, a second at a hundred workers, where the lock's other claimants give up after two.

   **Once few remain, lowest count first**: `ORDER BY occurrence_count ASC` — on the count *alone*, ties falling in whatever order the index holds them (see [What a merge costs](#what-a-merge-costs) for why not by rack) — excluding the racks named in the `forced_racks` of an open claim for this generation, so concurrent claims are not handed overlapping subsets, **and the racks forced by a task whose result is staged but not yet merged**, for the reason above. A sweep would spend the end of a generation stepping over racks already at target; here the set below target is small by construction, so the excluded set is too. The turn from the first selection to the second is made only at a lap's boundary — nothing in flight, nothing staged — and then remembered, as the generation's cursor row with no rack, so every later claim of the generation goes straight to it and nothing goes back, since counts only grow. What the second excludes is then only what it has handed out itself, at most the racks below target: 67 ms a claim with all 50,000 of them out, at 400,000 racks and 500 a task. It used to turn wherever a merge put the summary under the threshold, mid-lap included, and every claim then hashed the racks of every sweep claim still out — 0.45 s a claim at a thousand workers, inside the dispatch lock, until those claims drained and a merge ran (thirty-first audit).

   Either way, if no rack is left to hand out *and* no `task_claims` row for this generation is still `claimed`: with results still staged, whether the generation is complete is **not yet known** — the claim starts a merge, off the request, and is told there is nothing here right now (`NeedsLeaveMerge`), and the next claim decides on exact figures; with nothing staged, the generation is complete — run generation transition (below) instead of dispatching a task. Claims in flight are read *before* what is staged, and the order matters: submissions are not serialized with claims, and read that way round a result is always one or the other. A claim that *is* handed racks, but fewer than `racks_per_task`, has found the generation (or the lap) nearly done, and asks for a merge too, at most once a minute per job: that is when stale counts cost most, since tasks go out forcing racks that may already be at target and the generation cannot close until a merge shows that they are.

   **The whole of step 2 runs under a per-job advisory lock** (`pg_advisory_xact_lock`, taken before anything is read and released when the claim transaction ends). Without it every read here is made against a view of the job that a concurrent claim may be in the middle of changing, and two races follow: a claim still being issued is not yet visible as in flight, so a generation could be closed while a task for it was going out — work that lands in a generation whose KLV is already built — and two claims could both find the generation complete and both start its transition, each streaming millions of rows and uploading a KLV. The lock is per job, so claims for other jobs never wait on it, and it is *not* held across the transition itself: a transaction held open across an S3 upload is what step 2 of the transition exists to avoid.

   **Reopened tasks are reissued here, not before.** For every other job type a task whose claim timed out is re-dispatched before anything new is generated. For leave generation that happens only after the lock is taken and the current generation determined, and only for a task whose `leave_requests.generation` is that generation *and* only while no transition for that generation is running; step 2 runs when there is none. The transition check is separate from the generation check and both are needed: a generation does not read as *closed* until its transition commits the artifact row, so throughout the tens of seconds a transition takes, the current generation is still the closing one and a reopened task of it would otherwise be handed straight back out — its occurrences folded into the very rows the transition is streaming, leaving the uploaded KLV irreproducible from the database. A transition past the takeover timeout does not count, or a job whose transition process died would stall forever instead of being taken over. Reissued before the lock, a claim would be invisible to the in-flight check exactly as a new one was. Reissued for any generation, a task from a generation that has since closed would be handed out again, played with an outdated KLV, and its result discarded. A task left over from a closed generation stays `available` and is never dispatched again, so the job list's task counts for a leave job can show a few such tasks as never completed.
3. Draw the task's seed and `INSERT INTO tasks (job_id, seed, state) VALUES ($job_id, $seed, 'available') RETURNING id`, claimed in the same transaction.
4. `INSERT INTO leave_requests (task_id, variant, letter_distribution, board_layout, generation, seed, forced_racks, num_games, previous_artifact_key, player_config_id)` — the request's `player` (and the top-level `lexicon`, the player's) come from the job's template, as every job type's players do; `seed` is drawn fresh for the task, and stored so a reissued task replays it; `forced_racks` is the chosen rack subset (see Schema); `previous_artifact_key` is the prior generation's combined KLV, which for generation 1 is the server-built zeroed KLV stored at generation 0, so it is never NULL.
5. `INSERT INTO task_claims (...)`.
6. Return the request and claim token.

**Worker behaviour**: The worker downloads the previous generation's combined leave file through `GET /api/worker/artifact` (a missing object is `404`, any other object-store failure `503` with `Retry-After`, which MAGPIE retries; the server keeps the last few KLVs it served in memory, verified against the hash it sends, so a generation's opening is one object-store read however many workers ask) — including generation 1, which fetches the server-built *zeroed* KLV stored as generation 0, so there is no first-generation branch and no fallback to the lexicon's shipped leaves. It seeds the run from the request's `seed`, hands the request's `forced_racks` to `leavegen` as an **in-memory rack list**, plays `num_games` games, and reads the rack-equity table out of `RackList`, submitting it as an inline `{rack, count, mean}` list in the `LeaveResponse`.

Neither the forced racks nor the results touch the filesystem: they arrive in the task's JSON request and go back in its JSON response. (There is no `-forceracksfile` / `-writerackequitycsv` round trip through scratch files — see [Leave generation on the client](#leave-generation-on-the-client).)

The reported list covers **every rack that occurred during the batch, forced or not**, since racks the games happen to draw naturally also count toward that rack's occurrence target. It therefore scales with distinct racks drawn per batch — potentially thousands of rows, not just `racks_per_task` — but that is still an ordinary-sized POST body (tens to low hundreds of KB), not something warranting object storage.

`num_games` is the only thing that ends the task. The generation's rack target is deliberately **not** sent: the server owns the running totals across every task in the generation, no single task can observe whether the target has been reached globally, and stopping early at the forced racks' own target would discard coverage the server would have folded in anyway.

**On result acceptance**: every reported rack must be a full 7-tile rack (plausibility refuses anything else), and is spelled as the universe spells it — its letters in code-point order, a blank first — before anything else reads it. MAGPIE writes a rack in its own machine-letter order with blanks last (`AEINST?`, and German's `AEINRSÄ` as `AÄEINRS`); matched exactly, every rack holding a blank went uncounted, and an English generation could never close (thirty-second audit). One rack under two spellings in one result is a duplicate, and refused. A result for a generation that has already been aggregated is credited to the worker and *not* folded in: its KLV is built and uploaded, so the occurrences would change nothing anyone reads, and adding them would leave the rows disagreeing with the artifact built from them — which is the one signal reserved for a corrupted or stale object (see [Artifacts: back up, or rebuild?](#artifacts-back-up-or-rebuild)). The claim flow no longer produces that state: a generation closes only when none of its claims is still `claimed`, a timed-out claim is abandoned and its late submission refused before it reaches this point, and a closed generation's tasks are never reissued (step 2). The check stays as a guard against state the flow never writes, such as a partial restore. Otherwise, within the same transaction that accepts the task result, the submission is **staged**: one row in `leave_rack_staging` carrying its racks, counts and equity sums as three parallel arrays (which Postgres compresses and stores out of line), and a single-row bump of the generation's live counters in `leave_generation_progress` (tasks completed, games played). It touches no per-rack row. A **merge** (`leave_gen::merge_staged`) later takes everything staged for the generation and applies its sum in one transaction — in passes over slices of the racks by hash, one pass per 400,000 racks of the generation, each an `UPDATE … FROM` over that slice's aggregated occurrences, then the `DELETE` of exactly the rows it read, so a staged result is either still staged or folded in, never both and never neither — logs any reported rack that matched no row, and refreshes the generation's summary (racks at target, the rack furthest from it) while the rows are warm. An update rather than an upsert: the universe is seeded, so a rack with no row is not a rack of this distribution and must not create one. Merges of one job serialize on an advisory lock, since two of them would update overlapping racks in whatever order their plans visited them; the transition *waits* for that lock and everything else gives up if it is held, because whoever holds it is doing the same work. Why it is built this way, and what it costs, is under [What a merge costs](#what-a-merge-costs).

**Generation transition (aggregation)**: once claim-time step 2 finds no rack below target, no claim in flight and nothing staged, the server first **drains** — a merge that waits for any merge already running, so the totals it is about to read hold every accepted result; nothing new can be staged meanwhile, since the generation closes only with no claim in flight and nothing is dispatched for it while its transition runs — and then streams that generation's full-rack results into a CSV, runs `magpie convert rackequity2klv` over it to build the generation's KLV artifact, uploads it to S3, records it in `leave_generation_artifacts` along with the builder that wrote it, and marks the generation complete.

The transition **does not write the next generation's rack universe.** That is millions of rows (3.2 million for English); inside the closing transaction it made every worker on the job wait the write out, and made the close and the copy stand or fall together, so anything that failed cost a full re-derive and re-upload as well. The universe is seeded when its generation *opens* instead — on a task of its own, started by the first claim that finds it missing, under the job's lock so two seedings cannot both do it — from the pinned letter distribution, through the same `seed_generation` for every generation, the first included. One implementation of what a generation's universe *is*, derived from the source of truth rather than from the previous generation's rows, and off the critical path.

The transition takes tens of seconds and runs *outside* the claim transaction, on its own task so that a worker or proxy giving up on the request cannot cancel it part-way. That leaves the deciding claim holding no lock while it works, so ownership is recorded instead: the claim transaction that finds the generation complete inserts `leave_generation_transitions (job_id, generation)` and **commits** — its only write is that row, and committing is both what makes the row visible to everyone else and what releases the job's advisory lock before the upload starts. The row's primary key is what means every other claim arriving meanwhile is told there is no work yet rather than starting the same transition again. `completed_at` is set in the same transaction as the artifact row, and setting it is **conditional on the row still being there and still open** — that is how a transition finds out it no longer owns anything. A purge deletes the transitions row along with the artifacts and progress rows, so a transition spawned before it would otherwise hand the purged job a generation-1 KLV derived from results it no longer has. When the close is refused nothing is written and the uploaded object is left behind; it is keyed by job and generation, so a later transition of the same generation overwrites it, and `GET /api/worker/artifact` serves no key that no `leave_generation_artifacts` row names. A transition that never finishes — the process died, or the object store refused the upload — is taken over by a later claim once `started_at` is older than the takeover timeout (30 minutes, far longer than any measured transition), and `attempts` records that it happened; a failure the server survives hands ownership back immediately instead of waiting out the timeout. So does a **restart**: a transition runs on a spawned task, the service is a single instance whose old task stops before the new one starts, and so at startup every open transition belongs to a process that is gone — exactly as every `running` import and export does. Startup backdates them (`leave_gen::release_orphaned_transitions`) and the next claim takes over; without that, every deployment that landed inside a transition left its job with nothing to hand out for half an hour.

The derivation is MAGPIE's own `rack_list_write_to_klv`. Each full rack `R` has a mean `m(R)` — `equity_sum / occurrence_count`, or 0 if it never occurred — and a weight, the ways to draw it from a full bag (the product over letters of `C(dist, R)`). The average is the weighted mean of `m(R)` over every full rack. Every proper, non-empty sub-multiset `L` of `R` receives `m(R)` weighted by the ways to draw the rest of `R` once `L` is held (the product of `C(dist − L, R − L)`), and a leave's value is its weighted mean minus the average, or 0 if nothing contributed.

**The server used to do this itself**, in a Rust translation of that function (`jobs/klv.rs`, 868 lines). It was a faithful translation, cross-validated against a real MAGPIE binary — but it had to change whenever MAGPIE's did and nothing made it, and its KLVs differed in bytes from MAGPIE's for the same leave values, because it built a plain trie where MAGPIE builds a minimized DAWG. Both are correct and they load to the same values; having two of them was a standing source of confusion, and the second one was the one nobody would notice going stale.

So the transfer is a CSV instead: `generation_klv` streams the generation's roughly 3.2 million `rack,count,equity_sum` rows into a scratch directory, `convert rackequity2klv` reads them into a `RackList` and writes the KLV, and the server reads the bytes back and uploads them. The server does not hold a generation in memory; MAGPIE's `RackList` does, some 375 MB at its peak for English in 32 seconds, in the web task (KL-44). Before `klv.rs` was deleted, the two were run against each other over the whole 149-rack test distribution and agreed on every leave value.

`rackequity2klv` is new on the MAGPIE side, along with a `RackList` setter that takes a rack's count and mean outright: `rack_list_add_rack` folds one game's equity at a time, which is what a `leavegen` run has and not what a whole generation's aggregate is. Every full rack must appear exactly once in the CSV — a rack the file omits would contribute a mean of zero at full weight to every leave it contains, which is a real leave value and indistinguishable from a measured one — so MAGPIE marks every rack unset before reading and refuses a file that leaves any of them that way.

The generation-0 zeroed KLV is `magpie createdata klv`, which builds exactly that from the letter distribution alone. The original design proposed a `convert zero2klv` for it; `createdata klv` already is it, through the same `klv_create_empty`, and one spelling is better than two.


**Dashboard progress**: two kinds of figure, and the page says which is which. *Live*, pushed as results land through the per-job SSE stream: tasks completed and games played in the in-progress generation, from the counters the submit transaction bumps. *As of the last merge*, shown with its time: racks at target and the rack with the fewest occurrences, from the summary each merge writes. Both are one row read (`leave_generation_progress`); counted from `leave_rack_progress` on read, as they were, the rack figures were a pass over 3.2 million rows on every detail view and every live push (210 ms measured). No heartbeat payload is needed for either. An admin who wants the rack figures current can merge on demand (`POST /api/admin/jobs/:id/merge-progress`, "Merge progress now" on the admin job page).

#### What a merge costs

A submission used to fold itself: `UPDATE leave_rack_progress … FROM UNNEST(…)` over every rack its games drew, tens to hundreds of thousands of rows scattered uniformly over a generation's 3,199,724 (258 MB of heap, 174 MB of indexes). Measured on a seeded full-size generation:

| One submission folding itself | Time in the submit transaction | WAL |
|---|---|---|
| 40,000 racks, first touch after a checkpoint | 2.7 s | 409 MB |
| 40,000 racks, same checkpoint cycle | 2.5 s | 147 MB |
| 200,000 racks (a 10,000-game task) | 5.5 s | 69 MB, plus its share of the page images |

None of those updates was HOT, and none can be: `occurrence_count` is an indexed column, because claim-time selection orders on it, so every row written is a new heap tuple, a new entry in both indexes and a dead tuple for autovacuum. The seconds were spent inside the transaction the worker waits on, with every other submission of the generation queued behind its row locks (taken in rack order, since the alternative was a deadlock). And the first change to a page after a checkpoint is logged as the whole 8 kB page, so with the racks scattered uniformly, each checkpoint cycle re-imaged most of the table.

Nothing needs the per-rack totals that promptly. Selection needs them roughly; closing a generation and building its KLV need them exactly, but only at that moment; the dashboard needs whatever is cheap. So:

| | Folding in the submission | Staged, then merged |
|---|---|---|
| Submit transaction, 200,000-rack result | 5.5 s, holding ~200,000 row locks | one insert and one single-row update: milliseconds |
| WAL per accepted result | 69–409 MB | **0.8 MB** (measured; the arrays compress) |
| Per-rack rows rewritten for thirty such results | 6,000,000 | 2,300,000 — the merge sums a rack's occurrences across results first |
| One merge of those thirty results | — | 61–88 s on a background task, holding no lock a claim or a submission wants |

**What the merge itself writes depends on two Postgres settings, and the Terraform sets both** (`infra/rds.tf`). A merge touches most pages of the generation whatever it carries, so it is mostly page images, and how many times over depends on how many checkpoints it spans — which WAL *volume* triggers. The same merge of thirty results, measured: **4.8 GB** at Postgres's default `max_wal_size` of 1 GB (five checkpoints, each re-imaging the table); **1.3 GB** with `max_wal_size` large enough to span it; **1.0 GB** with `wal_compression` on as well. At the default the merged design writes about as much WAL as the folds it replaces (≈ 4.6 GB for the same thirty results); with the two settings it is a fifth of that — about 2 GB an hour for a job completing a result a minute, against about 9. That WAL is kept for the whole point-in-time-recovery window, so the difference is storage as well as I/O.

The merge interval (`leave_gen::MERGE_INTERVAL`, thirty minutes) sets that volume and the dashboard's lag, and nothing else: selection holds a staged task's racks out of play rather than trusting stale counts, claims near a generation's end ask for a merge themselves (`TAIL_MERGE_INTERVAL`, a minute), and a generation never closes with anything staged. A process that stops with results staged loses nothing — they are rows — and the sweep's first tick, at startup, merges them.

**The selection index stays small, by giving up an order nothing needed.** For a while `leave_rack_progress_pick_idx` carried `rack`, so that selection's `(occurrence_count, rack)` order was an index walk rather than a sort of the generation. Measured on a full English generation that index was **180 MB where the one on the count alone is 22 MB** — nearly all of the narrow index's keys are equal, so Postgres deduplicates them, and unique keys cannot be — which made a generation 590 MB rather than 432, for the life of the job. Selection now orders on `occurrence_count` alone and lets ties fall in whatever order the index holds them, and the public feed pages by rack through the primary key; dispatch order among tied racks is no longer reproducible, and nothing depended on it.

**A merge writes no temporary files.** As one statement, a merge of 200 staged results of 150,000 racks each wrote 2.9 GB of them (159 s), and a backlog after an outage is larger — on a volume of 20 GiB. Two things spilled: `UNNEST(a, b, c)` in `FROM` materializes each array before it is read (1.5 GB for 200), and the sum over every element staged was a sort of all of them — or would have been a hash table of every rack, about 650 MB for English. The arrays are now unnested in the select list, where they stream, and the racks are summed a slice at a time, by hash, one pass per 400,000 racks of the generation (eight for English), each slice's hash table held in memory (`work_mem` 64 MB for the merge — a hash table may use it times `hash_mem_multiplier`, 2 by default, so 128 MB — and sorting off, since the planner cannot see how few racks a slice holds): 74–90 MB for a slice's hash table, about 130 MB for the database backend running the pass in all, and nothing written to disk. A merge sizes its passes from the generation's summary, and one with nothing staged runs none. Measured on a full English generation: 153 s for 200 staged results and 253 s for 600, no temporary files at either — linear in the backlog, each rack's row written once. Those figures had the generation in memory. On 1 GiB — the Terraform's `db.t4g.micro`, three generations of 3.2 million racks and thirty staged results each — a merge took 11 minutes, because each pass probed the primary key in hash order, a random read per rack; each slice's sums are now sorted by rack first, so the update walks the key in order: 132–145 s on the same machine (the unsorted single statement of before took 522). At most two merges run at once across every job, since each pass's backend holds about 130 MB (merges that would not wait give up, as they do at a job's own lock). Measure one on the real instance before the first leave job there. (A first version took a pass per fifty results; its spill was bounded, but each pass read everything staged, so its time grew as the square of the backlog: 7.7 minutes at a thousand.)

What is left unsolved is that a merge still rewrites most of a 432 MB relation as non-HOT updates. Removing that means taking `occurrence_count` out of the index selection uses — for instance selecting "any rack below target" through a partial index on a flag the merge maintains, rather than "the racks furthest below" — which changes the selection policy, and has not been decided.

### Position Capture From Games

**Status: implemented.** Verified end to end: MAGPIE captured 98 positions across
4 real games, with the CGP evolving turn by turn.

A worker playing a game already analyzes a position on every turn: it generates
candidate moves, ranks them, and picks one. Those analyses used to be discarded.
With `capture_positions` set on a `games` or `game_pairs` job they are kept, so a
job run to settle which player is stronger also produces a corpus of analyzed
positions.

#### What is capturable, and what it costs

This is the constraint that shapes everything else, and it splits by player type.

**A simming player's analysis is free.** `autoplay_worker->move_lists[player]` is
sized by that player's `num_plays` and, at the moment a move is chosen, holds
exactly that many candidates ranked by simulation. The work is already done and
thrown away; capturing it costs only serialization.

**A static player's analysis has to be made.** Static play calls
`get_top_move_for_player_on_turn`, which on its own forces `MOVE_RECORD_BEST`, so
the move list ends up holding one entry. With capture on, MAGPIE relaxes that
override: autoplay raises each seat's move-list size to the capture cap (player
1's `num_plays_recorded`, `position_play_cap`), and the static player generates
with `MOVE_RECORD_ALL` and sorts the list best-first before reading its move
(`autoplay_worker_create`, `get_top_move_for_player_on_turn`'s `record_all`). Every
turn of every game then records and sorts moves the player would have discarded
— a real slowdown on the job's primary purpose — and stores `num_plays_recorded`
rows per position rather than one.

| Player | Capture cost | What you get |
|---|---|---|
| Simming | Serialization only | The simulated ranking the player actually used |
| Static | Slower move generation on every turn | A ranked list the player did not need |

The feature is most defensible for simming players; for static players it is
possible but clearly marked as slowing the job down, and the job form's
Position Recorder help says so. Relaxing the `MOVE_RECORD_BEST` override was once
the one phase not yet done; it is done, and it is also where the eleventh audit's
"capturing static player played its worst move" came from (the list was read
before it was sorted). Until the thirty-third audit's pass 2 the form's help, and
this section, still said a static player records only the move it played.

The two kinds of player now store the same number of moves; what differs is what
the ranking costs. A `games` job pairing a simming player against a static one,
at `num_plays_recorded` of 6:

| Turn | Player | Ranked | Stored |
|---|---|---|---|
| 0 | simming | 6 (by simulation) | 6 |
| 1 | static | 6 (by equity) | 6 |
| 2 | simming | 6 (by simulation) | 6 |
| 3 | static | 6 (by equity) | 6 |

A turn with fewer legal plays than that ranks and stores what there is.

**One semantic worth knowing:** for a simming player the `rank` is the
simulation's ordering while the stored `equity` is the *static* equity, so the two
disagree — a captured position can show rank 1 at equity 1.11 and rank 6 at 14.89.
That is correct but easy to misread; exposing the simulated evaluation would need
`SimResults` access from the recorder.

#### Volume

Every position of every game is captured — there is no sampling. A game runs about
**22.5 turns** (measured), and a pair is two games, so these are actual row counts
rather than a worst case:

| Job | Positions | Move rows at 10 kept each |
|---|---|---|
| `max_pairs = 40,000` | 1,800,000 | 18,000,000 |
| `max_games = 400,000` | 9,000,000 | 90,000,000 |

Turning capture on roughly doubles the storage a job produces per unit of
strength information, and does so in the largest table in the schema. That is why it is off
by default.

**`games_per_batch` becomes the memory and payload control.** With no per-task cap,
submission size is a direct function of batch size: a batch of 20 games is about
450 positions and a few hundred KB of JSON, while a batch of 1,000 games is 22,500
positions and on the order of 15 MB. That is the existing knob rather than a new
one.

There is deliberately no sample rate, no turn limit and no per-task cap. Capture is
all-or-nothing per job, which removes the need for sampling to be deterministic and
removes any question of which positions a task sampled.

#### Keyed on the task

Captured positions are keyed on `(task_id, game_index, turn_number)` rather than
on the claim. When jobs could run a task more than once (redundancy, removed in
October 2026), they were inserted with `ON CONFLICT DO NOTHING`, so the first
accepted copy landed and the rest were no-ops; a task now has one slot, the
insert is a plain one, and the key is how a random position is drawn and what
would fail a duplicate. Opening racks keep their per-claim key, so the several
analyses of a rack an opening-rack consensus asks for can be compared.

#### Schema

The existing `position_analysis_records` / `_moves` / `_plies` tables are the right
home. This is the distinction already drawn for opening racks: the *request* is
job-type-specific, but what comes back **is a position analysis** regardless of what
produced it.

Two changes were needed. A **surrogate key**: the record was keyed
`(task_claim_id, rack)`, which cannot address an in-game position, since the same
rack recurs across turns and games — it is now `id BIGSERIAL PRIMARY KEY`, with
`position_analysis_moves` referencing that one column. And **provenance columns**,
null for opening racks: `position` (the CGP — NULL for an opening rack, where the
board is empty by definition and the rack is the whole position), `game_index` and
`turn_number`. The two partial unique indexes are what encode the different keying:
`(task_id, game_index, turn_number) WHERE game_index IS NOT NULL` for in-game
positions, `(task_claim_id, rack) WHERE game_index IS NULL` for opening racks.
The opening-rack one guards nothing the submission does not check first —
`opening_rack::check_batch_against_task` refuses a batch that names a rack twice —
and no query reads it. It is kept anyway, as defence in depth against a decoder
or handler regression, at the cost of an index entry per opening-rack record
(about 3.2 million, over 100 MB, for a full English job) and an index write per
record on the submit path (thirty-third audit).

How many ranked plays come back per position is player 1's config's
`num_plays_recorded`, for both players' positions, not a job setting — MAGPIE has
one cap for the whole run, and the server truncates with the same number. It
pairs with `num_plays`, which is how many the player simulates; note that with
capture on, MAGPIE raises each simming player's `num_plays` to at least that cap,
so job creation refuses a capture job whose simmers would be raised: each must
already consider at least that many. Per-ply statistics pair the same way:
`num_plies_recorded` against `plies`. The saved-positions read returns at most
the first two plies of each move (P1, the reply, and P2), whatever was
recorded, since the page never shows more. A position's inference is a row of
`position_analysis_inference`, keyed on the record, inserted beside its plies
and carried by exports and partial restores as they are.

#### MAGPIE changes

**The hook already existed.** `autoplay_results_add_move()` is called once per turn
from `autoplay.c`, immediately after the move is chosen and before it is played.
`Recorder` already carries an `add_move_func`, and three recorders already use it
(`leaves_data_add_move`, `fj_data_add_move`, `win_pct_data_add_move`). A positions
recorder is a fourth instance of an established pattern rather than new machinery.

**`RecorderArgs` needed three more fields.** It carried `game`, `move` and `leave`
— everything else zeroed — so a recorder could see *which move was played* but not
*what else was considered*, nor which game or turn it belonged to. Added:

```c
  const MoveList *move_list;  // the ranked candidates this turn
  int game_number;            // which game of the batch
  int pair_game_number;       // 0 for an unpaired game; 1 or 2 within a pair
  int turn_number;            // turn within the game
```

All are available at the call site: the candidates are
`autoplay_worker->move_lists[player_on_turn_index]`, and `game_runner` already
tracks the rest. Widening `autoplay_results_add_move`'s signature is the bulk of
the change, and it touches the three existing `add_move` recorders only insofar as
they ignore the new fields.

**The `positions` recorder** is `AUTOPLAY_RECORDER_TYPE_POSITION` alongside the
existing enum values, registered through `autoplay_results_set_recorder()` with the
same eight function pointers the others use, and selectable through the options
string — so `autoplay games,positions` works from the command line too, which makes
the recorder testable without birdtest in the loop. Per turn it records the CGP via
`game_get_cgp`, the rack of the player on turn, the provenance fields, and the top
`num_plays_recorded` entries of `move_list` formatted with
`string_builder_add_move()` exactly as the opening-rack executor does, with
`equity_is_convertible()` guarding the pass sentinel.

**A simming player's ranking is the simulation's, not the move list's.** This is
the one place where the obvious implementation stores the wrong rows: for a
simming player the candidates in `move_list` are in *static equity* order, and
reading them in that order while attaching each play's simulation statistics
stores "the top N by equity, annotated with simulation" — not the top N the player
actually chose between. Both the captured-position recorder and the opening-rack
executor therefore read the simulation's sorted display copies, as MAGPIE's own
`sim` output does, through one shared writer
(`autoplay_results_write_ranked_plays_json`). Sharing the writer is what keeps the
two job types reporting the same fields in the same order; it is also how the
equity-order bug was found, since only one of the two had it.

Two details that will otherwise bite: **the move list is reused across turns**
(`autoplay_worker->move_lists[]` is allocated once per worker and refilled every
turn, so the recorder must copy what it needs rather than retaining the pointer);
and **`MOVE_RECORD_BEST` leaves one entry**, so a static player's move list holds a
ranking only because capture switches it to `MOVE_RECORD_ALL` and sorts it (see
"What is capturable, and what it costs"); without that the recorder would capture
a one-move "ranking", as it once did.

**Threading and consolidation** follow `leaves_data_consolidate`: one recorder
instance per `AutoplayWorker`, accumulating a list per thread, concatenated on
consolidate. Because threads interleave games, the merged list is **not** in game or
turn order. Rather than sorting on consolidate, the output is left unordered and the
server keys on the position, which it must do anyway.

**A pair's two games are one `game_index` space.** The recorder tracks
`game_number` and `pair_game_number` separately, but what crosses the wire is a
single index over the batch: `game_number * 2 + (pair_game_number - 1)` for a
paired run, and `game_number` for an unpaired one. That is what lets the server
key on `(task_id, game_index, turn_number)` — one pair of columns rather than
three — and it is what makes the batch-size check meaningful, since a paired
batch of N pairs has game indices `[0, 2N)` and `all_games.games` is `2N`.

**Emitting it.** Positions are far too large for a `-hr false`-style summary, and
the client reads results in-process anyway, so nothing is parsed from text: each
recorder has a `json_func` beside its `str_func` and writes its own share of one
result object. The positions recorder keeps its captures per worker thread and
renders them on consolidate (`positions_data_consolidate`) into the `positions`
key, each position's plays through `autoplay_results_write_ranked_plays_json`;
the game recorder writes `all_games` and, for pairs, `divergent_games`.
`config_contribute_games` takes the whole object from `autoplay_results_get_json()`
and submits it as the task's result, and sets the recorder option when the task request asks
for it, so `capture_positions` on the birdtest job becomes
`autoplay games,positions` on the MAGPIE invocation, and with
`capture_first_divergence` on a pairs job, `autoplay games,divergentpositions`.

#### Wire format

`GameResultsResponse` gains an optional array alongside the aggregates it already
carries:

```json
{
  "all_games": { "...": "..." },
  "pentanomial": [12, 3, 140, 5, 40],
  "divergent_games": { "...": "..." },
  "positions": [
    { "game_index": 0, "turn_number": 3,
      "rack": "AEINRST",
      "position": "15/15/... AEINRST/ 0/0 0",
      "previous_move": "8D DOG", "previous_move_score": 10,
      "played_move": "8D RETAINS", "played_move_score": 74,
      "num_moves": 412,
      "moves": [ { "move": "8D RETAINS", "score": 74, "equity": 81.2,
                   "win_percentage": 62.1, "blended_utility": 0.64 } ] }
  ]
}
```

`previous_move` / `previous_move_score` are absent on turn 0 of a game, where
nothing preceded it. `played_move` / `played_move_score` are always present: the
move chosen from this position, written against this position's board (so a
tile it plays through is in parentheses). It need not be the top of `moves` --
a simmer can pick a play lower by equity, and a solver's pick is its own --
and with first-divergence capture no later row holds it as its previous move,
so it is stated rather than derived (October 2026). The job page draws it on
the board where it goes, beside the previous move's outline, and marks it in
the ranked list. `blended_utility` — the win%+spread blend, sometimes used to
rank moves instead of equity or raw win percentage — has the same nullability as
`win_percentage`: present only for a simming player. The whole array is absent when
capture is off, which keeps every existing client valid.

A simulated position whose player inferred first also carries `inference`
(absent otherwise; October 2026):

```json
"inference": { "num_leaves": 143, "total_draws": 52011, "average_equity": 12.41,
               "leaves": [ { "leave": "AEINST", "draws": 812, "equity": 30.2 } ] }
```

`leaves` is at most ten, most drawn first, each the tiles the opponent kept,
written in the job's letter distribution. MAGPIE keeps a list of them only
when asked to, so autoplay raises each seat's inference `leave_list_capacity`
to ten (`AUTOPLAY_CAPTURED_INFERENCE_LEAVES`) when it captures positions, and
leaves it at none otherwise. The server refuses (`plausibility::check_inference`)
an inference on a position that cannot have one -- not simulated, turn 0, or
no previous move -- and one that is not internally possible: more than ten
leaves, fewer distinct leaves found than listed, a leave drawn more often than
all the draws, leaves out of draw order, an equity that is not finite or past
MAGPIE's bounds, or a leave that is not one (malformed, or as many tiles as a
full rack). `contract-fixtures/result-games-inference.json` is
its fixture, written by hand; MAGPIE's
`test_inferring_players_report_their_inference` checks its own output carries
every key the fixture does.

**Server-side validation rejects positions outside the task's own games** — a
`game_index` beyond the batch, or a `turn_number` beyond any plausible game — since
the submission is otherwise unbounded input written straight into the largest table
in the schema. The natural bound is the batch's own size: at most `games_per_batch`
games, and a generous per-game turn ceiling.

#### Open questions

1. **Should captured positions share the opening rack tables?** An opening rack job
   analyzes turn 1 exhaustively; a games job captures turn 1 positions incidentally,
   under a different player config. Sharing a table means queries must always filter
   by job, or on `position IS NULL`. The alternative is a separate
   `game_position_analyses` table, which duplicates the moves table.
2. **What bounds a submission?** Settled: `POST /api/worker/result` refuses a body
   over 64 MiB before parsing it (`MAX_RESULT_BYTES`, `413`), and the compose Nginx
   allows the same. `games_per_batch` stays the admin's lever for staying under it.
   The other worker routes take 1 MiB (`WORKER_BODY_BYTES`): a claim lists the
   jobs its worker cannot run, which MAGPIE does not cap. How many bodies are
   held at once is bounded too (thirty-first audit), in two tiers
   (`extract::read_body`):
   - **small bodies** — every body declaring at most 1 MiB, or nothing — share
     no budget, so nothing another caller does can make one wait; each has 30
     seconds to arrive, and one declaring nothing is cut off at 1 MiB. What
     they hold together is what callers can send in 30 seconds;
   - **large bodies** — only results, from an identity the route has already
     checked — reserve their declared length whole from a 192 MiB budget
     (`extract::LARGE_BODIES`) before a byte is read, or are answered `503`
     with `Retry-After` at once (MAGPIE retries 5xx for a quarter of an hour);
     one identity may have 64 MiB reserved at once (machines sharing a key
     share it); results over 1 MiB are decoded three at a time
     (`registry::large_result_turn`); each has 30 seconds plus its size at
     64 KiB/s, and may not stall for 30. A result keeps its reservation until
     its handler returns;
   - the heartbeat, decline, result and artifact routes refuse a caller with
     no identity, or one the server does not know, from the headers
     (`auth::RegisteredWorker`), and a claim with no identity is rate-limited
     and held to 16 KiB before its body is read.

   Before this, a caller with no credentials held a dozen 60 MiB uploads open
   and took the web task's memory from 38 MB to 778 MB: the ALB streams `/api`
   to the backend unbuffered. The same flood is now answered `401` at once;
   twelve workers' 60 MiB results are three admitted (217 MiB resident) and
   nine refused at once, with a heartbeat answered in 4 ms throughout. The
   audit's first design put every body on one budget, charged as bytes
   arrived and waited for when spent: 192 identity-less claims declaring
   64 MiB filled it, and every heartbeat, login and ban waited ten seconds for
   a `503` — heartbeats are not retried, so in five minutes every claim in the
   fleet lapsed. Its own adversarial check found that, within the audit.

There is deliberately no consumer yet: this is a corpus being built for later use,
and it leaves the database as the second object of the job's [export](#exports).
That is a legitimate reason to capture everything rather than sample, but it does
mean the first real query against it may want an index that does not exist yet.

### Rust Implementation

Each job type is a struct implementing the `JobHandler` trait:

```rust
pub trait JobHandler {
    type Request: Serialize;
    type Response: DeserializeOwned;
    type Record;

    /// Read back a stored request. A task whose claim lapsed is re-dispatched
    /// through here rather than regenerated, so the request a worker sees is
    /// always the one recorded against the task. The job's immutable half --
    /// its players, its letter distribution, its run-wide settings -- comes
    /// from `template`, read once per process, so only the row that differs
    /// per task is read here.
    async fn load_request(
        conn: &mut PgConnection,
        template: &JobTemplate,
        task_id: Uuid,
    ) -> AppResult<Self::Request>;

    /// Normalize a worker submission into its stored form.
    fn process_response(response: Self::Response) -> AppResult<Self::Record>;

    /// `template` carries the job id, which every record table stores
    /// denormalized so a job's rows can be read without joining through
    /// `tasks`, and the per-player settings that decide how much of a result
    /// to keep.
    async fn insert_record(
        conn: &mut PgConnection,
        template: &JobTemplate,
        task_id: Uuid,
        claim_id: Uuid,
        record: &Self::Record,
    ) -> AppResult<()>;
}
```

There is **no `creation_strategy()` and no `CreationStrategy` enum**: every job
type is on-demand, so the axis disappeared along with the pre-populated strategy
(see [Creation Strategies](#creation-strategies)). Request *insertion* is not on
the trait either — it happens inside each job type's claim-time transaction,
where the request is generated — so the trait carries only the three operations
that are the same shape for every type: read a stored request back, normalize a
response, store the record.

Handlers take a `&mut PgConnection` rather than a `&PgPool` because every one of
these runs inside the caller's transaction: a claim inserts task, request and
claim atomically, and a submission writes the record and bumps the counters
atomically. They take the job's `JobTemplate` (`jobs/dispatch.rs`) rather than
its id because everything a handler needs about the job other than the task's
own row is immutable and already in memory: the players, the letter
distribution, the batch size, the per-position move cap.

A top-level `JobType` enum dispatches to each concrete handler. The compiler enforces exhaustiveness on all match arms, so no case can be silently forgotten.

**Adding a new job type requires:**

1. Add a variant to the `JobType` enum and to the `job_type` Postgres enum (migration).
2. Add the typed request and record tables to the migration.
3. Create a handler struct and implement `JobHandler` with its three associated types.
4. Add claim-time task generation for the type, and the variant to `JobType`'s match arms in [`registry.rs`](backend/src/jobs/registry.rs) — the compiler will reject a build that omits it.

---

## Worker Client

The contributor client is **MAGPIE itself** — a contributor needs MAGPIE and
nothing else, no Python, no Docker. `magpie contribute` polls for tasks,
executes them in-process, and submits results, handling authentication (API key
or anonymous UUID), heartbeating, data verification and the request/result cycle
automatically, all driven by a local `contribute.txt`.

This used to be a separate Python client (`worker/worker.py`) that shelled out to
a MAGPIE binary distributed in its own Docker image. That client is retired,
and **MAGPIE is the only production client there is.** What remains under
`worker/` is `fake_worker.py`, which is test tooling and nothing else: a
MAGPIE-free way to test the server itself. It speaks the worker API and submits
*synthetic* results, including the adversarial paths a real client cannot reach
on purpose -- malformed submissions, stale claim tokens, abandoned claims,
concurrent claimers. (Declines are tested in the Rust tiers; the fake's decline
modes, which nothing ran, were removed in the thirty-third audit.) Each
position it reports is shaped by its mover's config as MAGPIE's are --
simulated for a simmer, static otherwise -- since the server refuses an
analysis the task's players could not have run. Pointed at a real server,
every result it invents would be recorded as a genuine contribution, so it is
never run against anything but a disposable test stack.

birdtest's backend runs a pinned MAGPIE, built into its image from a recorded
commit. It has two jobs, and the second is the reason the first became worth
doing (see [MAGPIE on the server](README.md#magpie-on-the-server) in the README):

- **Reference copies of derived files.** A wordmap and a rack info table are
  built on each contributor's own machine and are far too large to ship. The
  server builds its own copy of each from the bytes a job pins, keeps the
  SHA-256, discards the file, and sends the hash with the claim; a worker uses
  its own copy only if the bytes agree. That is what lets `use_rit` mean
  anything — see [Wordmap and rack info table
  provenance](#wordmap-and-rack-info-table-provenance).
- **Leave-generation KLVs.** Aggregation used to build its KLV artifact with a
  Rust translation of MAGPIE's `rack_list_write_to_klv`, which had to be kept in
  step by hand and produced different bytes for the same values. `convert
  rackequity2klv` is the same derivation run by the code that defines it.

The image still carries no data directory: every conversion runs in a throwaway
directory written from the object store and from `input_data.content`.

### Status

Implemented on MAGPIE's `birdtest-contribute` branch and verified end to end
against a local birdtest instance: `magpie contribute` claims tasks, plays real
games, and submits results the server records and credits.

| Piece | State |
|---|---|
| `src/compat/chttp` (libcurl via dlopen, WinHTTP, wasm stub) | Done. Also now backs `get_gcg.c`, replacing its three `curl`-binary calls. |
| Vendored cJSON (`src/compat/cjson`, platform-conditional so it lives in `compat`) + `src/util/json` wrapper | Done |
| `src/util/http_client` (retry policy) | Done |
| `src/ent/client_state` (`contribute.txt`) | Done |
| `contribute` command and task loop | Done |
| `games` / `game_pairs` executors | Done, verified end to end |
| `opening_rack` executor | Done. Static players verified end to end in the audit. A simming player's moves are reported in the simulation's ranking with win%, blended utility and per-ply statistics up to `num_plies_recorded`, written by the same code that writes a position captured during a game (`autoplay_results_write_ranked_plays_json`), which now also ranks captured positions by simulation rather than move-list order |
| `leave_generation` executor | Done. Forces and reports full 7-tile racks, which is what the server tracks. Writes no per-generation files into the data directory. See [Leave generation on the client](#leave-generation-on-the-client) |
| Async GUI status surface | Not implemented |
| Windows WinHTTP backend | Written, not compiled or run on Windows |
| Data verification, decline, shutdown | Specified in [Capability negotiation](#capability-negotiation); MAGPIE side on the same branch |

Three things learned while building it are folded in below: a task that fails
does not count toward `maxtasks`, so the loop needs a consecutive-failure guard
or an unrunnable job spins forever; `arg_token_t` is private to `config.c`, so
the settings-file path is read there and passed into `impl_contribute` rather
than looked up inside it; and the worker UUID is minted by the **server**, not
the client.

### Goal

A contributor should need **only MAGPIE**. The immediate target is:

```
magpie> contribute
```

with everything it needs — server, credentials, limits — in a `contribute.txt`
beside it, so no API key ever reaches a command line or `settings.txt`. The
eventual target is a MAGPIE GUI button calling the same code path, which is why
the command runs asynchronously and exposes machine-readable status rather than
assuming a terminal.

**The second reason to do this** is that the old client's fragility was almost
entirely about **crossing the process boundary**. It invoked `autoplay`, `gen`
and `leavegen` as subprocesses and reconstructed results by parsing formatted
stdout. Every integration bug found lived there: invented flags, output formats
that turned out to be aggregate-only, a rack-equity CSV written under a name the
client did not predict, and MAGPIE resolving its board layout from `./data`
before parsing `-path`. Running in-process deletes that entire class of problem.

### Ground rules

**Platform-specific code lives only in `src/compat/`.** No file outside
`src/compat/` may contain `#ifdef _WIN32`, `#ifdef __APPLE__`, `#ifdef __wasm__`,
or any other platform test. The client work is the largest new source of
platform behaviour MAGPIE has taken on, so it must not be what breaks this rule.
The rule does not hold exactly, though: grepping the tree outside `src/compat/`
finds two platform tests. One comes from MAGPIE's main:
`src/ent/transposition_table.h`'s `#ifdef __EMSCRIPTEN__`, a smaller minimum
table in the browser. The other is this branch's own: `src/util/io_util.c`'s
`#if defined(__APPLE__)`, which picks `st_mtimespec` or `st_mtim` for the
data-file identity's nanosecond times. It breaks the rule, belongs in
`src/compat/`, and has no Windows arm yet (the thirty-third audit's pass 2 found
it). One new piece of platform behaviour is needed, and it goes in
`src/compat/`: HTTP + TLS, as `chttp.{h,c}`, exposing `chttp_request()`.

The vendored cJSON parser also lives in `src/compat/`, not because MAGPIE's own
code branches on platform but because the vendored source itself has a
`#ifdef _WIN32`-shaped block — the same reason it counts as platform-specific
under this rule. `src/util/json` wraps it in an `ErrorStack`-aware API and is the
only file that includes `cjson.h`.

HTTP is the only addition compat needs. The worker UUID is minted by the server
rather than generated locally, so the client never needs a random source of its
own; and the task loop's poll and backoff waits call `ctime_nap(double seconds)`,
the portable blocking sleep `src/compat/ctime.h` already exposes and MAGPIE
already uses elsewhere, rather than introducing a second sleep abstraction.

**The WASM build compiles everything.** `Makefile-wasm` compiles every `.c` under
`src`'s subdirectories, so every new file must compile under Emscripten.
Networking is meaningless there, so `src/compat/chttp.c` compiles a stub under
`#if defined(__wasm__)`: `chttp_request()` pushes an error onto the stack and
`chttp_is_available()` is false. Nothing above compat needs to know.

### Design overview

```
contribute
  |- client_state       read contribute.txt; adopt the server-minted UUID
  |- birdtest_api       typed wrappers over the six worker endpoints
  |    |- http_client   portable request/retry logic
  |    |    `- chttp    COMPAT: libcurl (POSIX) / WinHTTP (Windows) / stub (wasm)
  |    `- json          portable wrapper over vendored cJSON
  |- heartbeat thread   runs for the lifetime of a claim
  `- task dispatch      one executor per job_type, calling existing impls directly
```

#### HTTP: `src/compat/chttp` + `src/util/http_client`

One neutral entry point, with everything platform-specific behind it:

```c
typedef enum { CHTTP_GET, CHTTP_POST } chttp_method_t;

typedef struct ChttpRequest {
  chttp_method_t method;
  const char *url;
  const char *const *headers;   // "Name: value" strings
  int num_headers;
  const char *body;             // NULL for GET
  size_t body_length;
  int timeout_seconds;
} ChttpRequest;

typedef struct ChttpResponse {
  long status_code;
  char *body;                   // caller frees; NUL-terminated
  size_t body_length;           // body may be binary (KLV artifacts)
  int retry_after_seconds;      // from the header; -1 when absent
} ChttpResponse;

void chttp_request(const ChttpRequest *request, ChttpResponse *response,
                   ErrorStack *error_stack);
void chttp_response_destroy(ChttpResponse *response);
```

| Platform | Backend | Notes |
|---|---|---|
| Linux, BSD, macOS | **libcurl**, `dlopen`ed at first use | Linux ships no OS HTTP API; libcurl is what the platform provides, and macOS ships it too (`/usr/lib/libcurl.4.dylib`), so one implementation covers both. |
| Windows | **WinHTTP** (`winhttp.dll`) | Ships with the OS, no redistributable, Schannel trust store already configured. |
| wasm | Stub | Pushes `ERROR_STATUS_HTTP_UNAVAILABLE`. |

**`dlopen` rather than link-time binding.** Only eight symbols are needed
(`curl_easy_init`, `_setopt`, `_perform`, `_getinfo`, `_cleanup`, `_strerror`,
`curl_slist_append`, `curl_slist_free_all`). Resolving them at first use means a
machine without libcurl still runs every offline MAGPIE command, and `contribute`
fails with "libcurl was not found" rather than MAGPIE refusing to start at all.
It asks first (`chttp_is_available`), before claiming anything: left to the first
request, the missing library read as a transport error, and the claim spent its
whole retry budget on it before saying why (thirty-third audit, pass 2). Try
`libcurl.so.4`, then `libcurl.so`, then `libcurl.4.dylib`.

Requirements that hold on every backend: **TLS certificate verification is on and
cannot be disabled** — no flag, no environment variable; redirects followed,
bounded at 5; `timeout_seconds` bounds a connect and a stall, not the exchange
(`CHTTP_MAX_EXCHANGE_SECONDS`, an hour, does that — KL-83); the response body is
length-delimited rather than NUL-delimited, since artifacts are binary; and no
global process state at exit, because `contribute` may run many requests.

`src/util/http_client.c` holds everything not platform-specific: building the
header list, the `Authorization` / `X-Worker-UUID` choice, the JSON content type,
and the retry policy, applied uniformly:

| Response | Action |
|---|---|
| 2xx | Return it. |
| 204 | Return it; the caller decides (for `/task` it means "no work"). |
| 429 | Sleep `Retry-After` (default 1s, never more than 60s) and retry, on the same budget as a 5xx — without limit for a claim, twenty times otherwise; the heartbeat's single-shot request does not retry it. (It had a budget of its own, five, after which a claim ended the run and a submission threw away a finished task.) `Retry-After` was read from the wrong libcurl field until the eleventh audit — `CURLINFO_OFF_T + 52`, the connect time in microseconds — so a worker slept for hours on its first `429`. |
| 5xx, or a transport error | Exponential backoff from 1s, doubling to a ceiling of 60s, for 20 retries — about fifteen minutes — then fail. A task claim, once the server has answered this run at all, keeps retrying at the ceiling instead of failing. |
| 4xx other than 429 | Return it; the caller decides. Never retried. |

**The transient budget is sized to outlast a deployment, and it was not.** It
was five retries — 31 seconds — and a request that exhausts its retries ends the
`contribute` run. birdtest is a single instance whose old task stops before the
new one starts, so a routine deploy is a minute or two of refused connections
and load-balancer `503`s: every contributor that asked for a task in that window
stopped contributing until a person noticed, and one that was *submitting* lost
the finished task with it. Fifteen minutes covers a deploy, a database failover
and a short maintenance window.

**A task claim does not give up at all, once the run has reached the server.**
An outage has no length a client can know — a deployment is a minute, a restore
from backup (RUNBOOK §1) is most of an hour — and the two ways of being wrong
are not alike: a claim a minute against a server that is away costs nothing,
while a run that gave up is a contributor's machine lost until its owner happens
to look. So after the first claim this run has had *any* answer to, a claim that
meets a transport failure or a `5xx` is retried for as long as that lasts
(`http_client_post_json_persistent`), at the back-off's ceiling of a minute,
printing a line each time so a contributor can tell waiting from hanging.
Nothing is held while it waits: no claim, no heartbeat, no result. Two things
keep the finite budget on purpose. A run that has **never** been answered still
ends with an error after it, so a mistyped `server` line fails instead of
polling nothing for ever. And so does every request other than a claim: a
submission's claim lapses on the server whatever the client does, so waiting
longer than the budget buys nothing. A `retryminutes` setting in
`contribute.txt` was considered instead and not built — it asks every
contributor to know in advance how long the operator will be away.
The heartbeat is the exception, and goes out **once** with no retry
(`http_client_post_json_once`): its own thirty-second schedule is the retry, and
a heartbeat backing off through an outage would hold up the task's submission,
which waits for the heartbeat thread to stop.

#### JSON

cJSON is vendored **verbatim and unmodified**, so it can be updated by replacing
the files, and wrapped in `src/util/json.{h,c}` so the rest of MAGPIE sees
`ErrorStack` rather than cJSON's conventions and a future swap touches one file.
Two details that will otherwise bite:

- **`seed` is a `uint64`** and must not round-trip through a `double`. cJSON
  stores numbers as `double`, which loses precision above 2^53. The server sends
  `seed` as a decimal string and MAGPIE reads it with `strtoull`.
- **Equity values are floats and must be locale-independent.** Write with
  `"%.6f"` under the C locale, never `%g`, and never rely on the process locale.

### Contribution settings

**Nothing about contributing is passed on the command line.** All of it lives in
one settings file, for two reasons: an API key on a command line ends up in shell
history and in `ps` output, and contribution settings have no business mixed into
`settings.txt` alongside board layouts and simulation parameters.

`contribute.txt` sits in the current working directory, one setting per line as
`key value`. **The file is optional, and so is every setting in it**: a missing
file means every setting takes its default, and a file that sets only some (an
`apikey` alone, say) takes the defaults for the rest. An existing file that
cannot be read is still an error. Blank lines and lines beginning with `#` are ignored — but a run
with no `apikey` set names a comment that holds `apikey` then a key, since
appending the setting to a last comment line with no newline puts it there
(thirty-second audit, pass 18). Setting names are lowercase (one in the wrong
case is refused, saying so); whole numbers run to 2147483647; an `apikey` is
`bt_` then letters, digits and underscores (an empty one means none); a UTF-8 byte-order mark is
skipped; and no refusal quotes a value.

```
# birdtest contribution settings
server    https://birdtest.example
apikey    bt_<the 64 hex characters the account page shows>
threads   7
maxtasks  0
idlewait  5
uuid      6f3d7198-178a-47c8-9ccc-6aa6995a5a9c
```

| Key | Required | Default | Meaning |
|---|---|---|---|
| `server` | no | `https://birdtest.org` | birdtest base URL (MAGPIE's `CONTRIBUTE_DEFAULT_SERVER`) |
| `apikey` | no | absent | Attributes work to an account. Without it the worker is anonymous, identified by `uuid`. |
| `threads` | no | cores − 1 | Threads given to MAGPIE while working |
| `maxtasks` | no | `0` | Tasks to complete before stopping; `0` runs until stopped |
| `idlewait` | no | `5` | Seconds to wait after the server reports no work |
| `uuid` | no | assigned by the server | The anonymous worker identity |

An unknown key is an error rather than a silent ignore — a typo'd `apikey` should
not quietly downgrade someone to anonymous. Only a missing setting defaults; a
malformed one is still refused. `contribute` prints the settings it is using at
start, marking each defaulted one (`server https://birdtest.org (default)`) and
saying only whether an API key is set, never the key.

The file is **user-authored and MAGPIE does not rewrite it**, with exactly one
exception: once the server assigns a `uuid`, MAGPIE **appends a single line**,
creating the file (with a one-line header comment) if there is none. It never
writes a defaulted setting into it, so a later change of default reaches every
contributor whose file does not override it.
Appending rather than rewriting means comments, ordering and formatting the
contributor put there survive untouched.

#### The worker UUID

**The server mints it, not the client.** A worker with no `apikey` and no `uuid`
yet sends no identity at all on its first request; the server responds with a
UUID in the body of the first successful `/api/worker/task` claim — once there is
actually a task to hand out — and the client persists it and sends it as
`X-Worker-UUID` from then on, for the rest of this run and every run to come.
Only a UUID in canonical form (8-4-4-4-12 hex digits, either case) is taken
(a newline in one wrote settings every later run obeyed); a settings file that
cannot be written is said so, with the line to add by hand, since otherwise
every run would be a new worker; and if the last `uuid` line (the one that
counts) is not a UUID — the start of one, left by a save a full disk cut
short — startup fails naming the line to correct or delete, rather than
sending it; an empty one still means none (thirty-second audit, passes 14 to
16).

This is a deliberate reversal from letting the client generate its own UUID: a
client-generated identity trusts a value the server never gets to validate.
Having the server mint it costs one extra round trip for a brand-new anonymous
worker's first task and nothing after that, and means `contribute` needs no
cryptographically secure random source at all.

Because the file is resolved relative to the working directory, a contributor who
runs MAGPIE from a different directory has no `contribute.txt` there. That used
to stop `contribute`; since the file became optional it starts as a new
anonymous worker on the defaults instead, losing the link to the earlier
identity's history. The trade was made deliberately (2026-10): a first-time
contributor needs no file at all, and the settings printed at start --
`(default)` beside the server, no `uuid` -- show the mistake at once. (Without
`./data` it still stops, on loading its default board layout, and exits 0.)

**The API key needs no special file handling.** `contribute.txt` holds a bearer
credential when `apikey` is set, but nothing about that requires MAGPIE-side
permission handling: it is a plain text file the contributor already created and
controls the permissions of, on their own machine, the same as `settings.txt`.
MAGPIE does not `chmod` it, check who else can read it, or otherwise treat it as
special. The key still never appears on the command line, in `settings.txt`, or
in status output, logs, or error messages — the file is the only place it lives.

### The `contribute` command

Registered in `config.c` alongside the others, taking **no settings arguments** —
only an optional path to the settings file, defaulting to `contribute.txt`:

```
magpie> contribute                      # reads ./contribute.txt
magpie> contribute /path/to/other.txt   # a path is not a secret
```

Only the default `contribute.txt` may be missing. A path given explicitly must
exist (it may be empty, or set only some settings): a typo in a named file
would otherwise start a new anonymous identity against the default server
without a word.

Implemented as `impl_contribute(Config *config, const char *settings_path,
ErrorStack *error_stack)` in `src/impl/config.c`, following the other `impl_*`
entry points, which owns the loop; `src/impl/contribute.c` holds the protocol
it drives — claiming, heartbeating, submitting, the anonymous-UUID handshake.

**Threads** default to `num_cores - 1`, minimum 1, overridable by the `threads`
key. Contributing should leave the machine usable — this will eventually be a
background activity someone opts into on their daily driver, and a machine that
becomes unresponsive is a machine whose owner turns contributing off. This is
deliberately independent of the global `-threads` setting: contributing should
not silently inherit whatever a user last set for simulation.

**The loop:**

1. Load `ClientState` from the settings file. `uuid` may be absent.
2. `POST /api/worker/task`, identifying with the API key if set, the stored
   `uuid` if set, or no identity header at all if neither is. The body is
   **required** and carries this build's `magpie_version`, its compile-time
   `board_dim` and `rack_size`, and `unsupported_jobs`, the in-memory set of
   jobs this worker has already found it cannot run. A build other than 15 and
   7 is answered with an `unsupported_build` shutdown before any job is
   considered; otherwise the server filters on the version and the set before
   it picks a job, so a worker that cannot run the job furthest behind its
   share still gets offered the next.
   - `204`: sleep `idlewait`, repeat. This means "nothing right now", nothing
     more.
   - `200` with a `shutdown` object: every active job is out of reach until this
     worker changes something. Print the accumulated gaps, the server's message,
     and the remedy it names; exit cleanly. This is the opposite of a `204` and
     the two must never be conflated.
   - `200` with a task: if the response carries a `worker_uuid` and this worker
     had none locally, adopt it — update `ClientState` and the request identity
     used from here on, and append it to the settings file. Continue.
3. **Verify the input data.** Resolve every `expected_data` entry through
   `data_filepaths_get_readable_filename` — the same lookup the executor uses, so
   the check cannot certify a different file from the one that loads — hash it,
   and compare. Any file missing or mismatched: decline, record the job as
   unsupported, and claim again without starting the heartbeat or running the
   task. An assignment with no `expected_data`, or one naming an `algorithm`
   other than `sha256`, or listing a file without a role, name or digest, or
   with a role this build does not know, fails the claim like any malformed
   assignment: nothing runs on input data it cannot check.
   Full detail in [Capability negotiation](#capability-negotiation).
4. **Version cross-check.** The server has already filtered on the version sent
   in step 2, so an assignment whose `min_magpie_version` exceeds this build is a
   server bug or a race with a floor that was just raised. Decline it with reason
   `magpie_version` and carry on: it is one job this worker cannot do, not a
   reason to end the session. The same is true of a `job_type` this build does
   not recognise. Exit is reserved for the `shutdown` of step 2.
5. Start the heartbeat thread.
6. Dispatch on `job_type`.
7. `POST /api/worker/result`.
8. Stop the heartbeat — **after** the submission, not before it. A claim is
   only as alive as its last heartbeat, and a submission is not instant: a batch
   with captured positions is tens of megabytes on a contributor's uplink, and a
   server that is restarting is retried for a quarter of an hour. (The code
   stopped the heartbeat first for a while, so a claim could lapse and be handed
   to another worker while its own result was on the way.) If `maxtasks` is
   reached, stop; otherwise repeat.

**Digests are cached** by (resolved path, size, mtime, inode, ctime), with
nanosecond timestamps where the filesystem records them, so a 15 MB lexicon is
hashed once per run rather than once per task. A cached digest must never be the
reason a bad file passes.

**Stopping hands the task back.** A stop request (the REPL's `stop`, the API's)
cuts a running task short — autoplay starts no more games, simulations end
early — so what it would submit is not the task the server asked for. The loop
declines the claim (`task_failed`, not counted as a failure) and ends; a stop
while waiting between claims, or while a request waits to retry against a
server that is down, returns within a second. (It once submitted the truncated
result, and kept claiming.) A hard interrupt simply abandons the
claim, which the server's heartbeat timeout reclaims.

**Errors during execution** are reported and the claim is handed back with a `task_failed` decline, so
its slot does not wait out the heartbeat timeout — and so is a result the server
refuses; the loop continues. An error *claiming* or *submitting* is handled by
the retry policy above: a submission stops the loop if it exhausts its retries,
and a claim does only if the server has never answered this run.

Because it is a normal command it inherits `-mode async`, so a GUI can start it,
poll status, and stop it with the existing machinery.

**Its tasks run in a config of their own.** Every task sets its lexicon,
per-player settings and derived-file flags on the config it runs in. Run in the
caller's, the REPL's save after the command wrote the last task's lexicon into
`settings.txt`; undone afterwards by replaying a snapshot, a replay that failed
part way left a session that could not load (a wordmap or word info table this
machine lacks for the task's lexicon) and, one save later, that lexicon in the
file. `contribute` creates a config for its tasks instead
(`config_create_for_contribute`): the same data paths, the caller's thread
control (so `stop` and the output reach it), settings never saved. The caller's
session and settings file are left exactly as they were.

### Per-job-type executors

Each builds the in-memory configuration the equivalent command line would, calls
the implementation function directly, and reads results out of the result
structs. No subprocess, no stdout, no parsing.

**Player configuration** arrives as a JSON object per player and maps onto
MAGPIE's per-player settings, where `N` is 1 or 2:

| JSON field | Setting | Notes |
|---|---|---|
| `recorder_type` | `-rN` | `best` \| `equity` \| `all`. Read only by an opening-rack static analysis: a static opening-rack player that keeps more than one move per rack needs `equity` or `all`, and job creation refuses `best` there. Autoplay ignores it — games, game pairs and leave generation generate with MAGPIE's own record type (`best`, or `all` when capturing positions) — and a simmer ranks every play up to `num_plays` whatever its recorder |
| `sort_strategy` | `-sN` | `equity` or `score` for a static player. A simmer's candidates are the top plays by equity (autoplay generates them so whatever this says), so a simmer is always `equity` and config creation refuses `score` for one |
| `lexicon` | `-lN` | **Required.** Every player names its own; there is no job lexicon to fall back to. |
| `leaves` | `-kN` | **Required**, for the same reason. |
| `win_pct_model` | `-winpct` | Null for a static player, which never loads one |
| `max_iterations` | `-iN` | Null for a static player |
| `num_plies` | `-plN` | How many plies to simulate |
| `num_plies_recorded` | `shplies` | How many to report |
| `num_plays` | `-npN` | How many candidate plays to generate/simulate |
| `num_plays_recorded` | `maxnumdplays` | How many to report |
| `stopping_pct` | `-scN` | |
| `use_inference` | `-siN` | |
| `time_limit_secs` | `-tlN` | 0 for a simmer: no limit, so the iteration budget decides |
| `use_wordmap` | `-wN` | applied directly against `players_data`, not `-wN`'s own arg parsing, and **before** the task's lexicon loads, which is when MAGPIE decides whether to load a wordmap |
| `use_rit` | — | applied against `players_data` the same way, and **before** the lexicon loads |
| `rit_name` | — | The name to load the table under, `<lexicon>.<leaves>`. A table stores precomputed leave values, so it belongs to the pair rather than the lexicon; the server pins a hash for this exact name (see [Wordmap and rack info table provenance](#wordmap-and-rack-info-table-provenance)) |
| `use_wit` | `-witN` | applied against `players_data` the same way, and **before** the lexicon loads. The table is named for the lexicon and built from the `.kwg` alone; the server pins a hash for it as a `wit` entry |
| `min_play_iterations` | `-miN` | |
| `threshold` | `-thN` | `'none'` \| `'gk16'` |
| `sampling_rule` | `-saN` | `'round_robin'` \| `'top_two_ids'` |
| `inference_margin` | `-imN` | |
| `utility_w_winpct` | `-uwinN` | blended-utility weight on win% |
| `utility_w_spread` | `-uspreadN` | blended-utility weight on spread |
| `utility_spread_scale` | `-uspreadscaleN` | |
| `movegen_margin` | `-mmargin` | Read only by an opening-rack static analysis with an `equity` recorder; autoplay generates with a margin of 0 |
| `endgame_plies` | `-epliesN` | **Required** of every player, if only as 0; 0 solves nothing, and turns off the pre-endgame too. Games and game-pairs tasks; a leave player may not solve |
| `peg_max_bag` | `-pegbagN` | **Required** of every player, if only as 0: the largest bag a pre-endgame solve is tried at |
| `peg_stage_top_k`, `peg_scenario_stride`, `peg_opp_model`, `peg_nested` | `-pegtopkN`, `-pegstrideN`, `-pegpessN`, `-pegnestedN` | Stated exactly when `peg_max_bag` > 0, null otherwise (MAGPIE refuses a key a player does not use) |
| `peg_nested_cand_caps`, `peg_nested_max_depth`, `peg_nested_strides` | the player's `-pegncaps`, `-pegndepth`, `-pegnstrides` | Stated exactly when `peg_nested` is true. Applied to that player's own solver settings (`AutoplaySolverSettings`), though the command-line flags are run-wide |

A player whose `num_plies` is 0 is static: MAGPIE decides whether a player
simulates on plies alone, and birdtest refuses a config that sets other
simulation settings without plies. **No setting that can change a result is left
to the worker's build.** Player-config creation writes MAGPIE's default into every
setting the body leaves out (`backend/src/magpie_defaults.rs`), so every request
states `recorder_type`, `sort_strategy`, `num_plies`, `num_plays`,
`num_plies_recorded`, `num_plays_recorded` and `movegen_margin` for every player,
and every simulation setting for a simmer; a static player's simulation settings
are null, since nothing reads them. The run-wide `bingo_bonus` and `sim_cutoff` are
written onto the job at creation and stated at the top level of every request
(leave generation states only `bingo_bonus`: its bot does not simulate). MAGPIE
refuses a request that leaves any of these out rather than supplying its own
compile-time default: a result has to be a function of the task, not of which
release ran it, and a version floor — a minimum, not a pin — cannot exclude a
release that changed a default. Every per-player setting is still reset before a
request is applied, and so is every run-wide setting no request states — the
multi-threading mode, small plays and the sim margin forecast — so nothing is
inherited from an earlier task or the contributor's `settings.txt`. `letter_distribution` and `board_layout` are
**required** on every request, like the settings above: both change what a
task computes, and absent used to mean the build's defaults — a distribution
inferred from the lexicon's name and the layout named for the compile-time
board size — which made them the last two settings a request could leave to the
worker's build. birdtest states both on every request of every job type, so
MAGPIE refuses one that does not (`config_contribute_validate_common`).
`win_pct_model` is
carried on each player object but is really one shared MAGPIE setting for the
whole run, so birdtest validates that a job's two player configs agree on it
before the job is created — only where both state one, since a static player has
none — and the worker reads it from whichever player states it. `movegen_margin`
is stated per player too, but a games or pairs job's players may differ on it:
autoplay never reads it (nor `recorder_type`) — every move is generated with a
margin of 0 and MAGPIE's own record type — so the games executor does not apply
it. (Until the thirty-third audit's pass 2, job creation refused two margins, a
refusal over a difference that changed nothing played.)

#### Every setting that can change a result, and what fixes it

The argument trace every audit re-ran from both ends — every key the executors
read, and every field of MAGPIE's `Config` asked whether autoplay, move
generation or a simulation can read it on the contribute path — as it stands.
It is here so the next trace starts from a list rather than from nothing, and so
a new MAGPIE setting has a table it visibly is not in.

| Setting (MAGPIE flag) | Changes results? | What fixes it for a task |
|---|---|---|
| Lexicon (`-l1`/`-l2`), leaves (`-k1`/`-k2`) | Yes | Stated per player, pinned by digest in `expected_data`. An opening-rack task gives both seats its one player's; a leave task gives both the fetched KLV — read afresh from the file it was written to, see below |
| Letter distribution (`-ld`), board layout (`-bdn`), variant (`-var`) | Yes | Required on every request; the two files are pinned by digest |
| Board size and rack size (`BOARD_DIM`, `RACK_SIZE` — compile-time, `make magpie BOARD_DIM=… RACK_SIZE=…`) | Yes — another rack size draws other racks and scores every bingo differently | Stated in every claim (`board_dim`, `rack_size`); a build other than 15 and 7 is sent an `unsupported_build` shutdown and handed nothing |
| Win% model (`-winpct`) | For simmers | From whichever player states one; pinned by digest |
| Wordmap (`-w1`/`-w2`), rack info table (`-rit*`) | They must not — but a stale or mismatched file does | Flags stated per player and set *before* the lexical load; the file's bytes must match the hash the server built, or the task is declined. A table is loaded by its pair's name, never the lexicon's |
| Word info table (`-wit*`) | It must not — but a stale one prunes legal plays | Stated per player (`use_wit`, on unless a config opts out) and set *before* the lexical load; the file's bytes must match the hash the server built (role `wit`), or the task is declined |
| Recorder (`-r*`), sort (`-s*`) | The recorder only for an opening-rack static analysis (autoplay generates with its own, `config.c` `impl_move_gen` being the only reader); the sort, yes | Required per player |
| Plies, candidate plays, iterations, minimum play iterations, stopping condition, time limit, threshold, sampling rule, inference and its margin, utility weights (`-pl*`, `-np*`, `-i*`, `-mi*`, `-sc*`, `-tl*`, `-th*`, `-sa*`, `-si*`, `-im*`, `-uwin*`, `-uspread*`, `-uspreadscale*`) | Yes | `num_plies` and `num_plays` required of every player, the rest of every simmer; all reset to MAGPIE's defaults first. An opening-rack task copies the player's into the run-wide settings `impl_move_gen` and `impl_sim` read, and forces inference off (there is no previous play, and `game_history` is whatever the contributor last loaded). A time limit must be 0 |
| Bingo bonus (`-bb`), simulation cutoff (`-cutoff`) | Yes | Required at the top of every request (the cutoff where the job can simulate) |
| Movegen margin (`-mmargin`) | Yes, for an opening-rack static analysis with an `equity` recorder; games, game pairs and leave generation never read it — autoplay generates with margin 0 | Required per player; reset before every task, and applied only by the opening-rack executor, from its one player |
| Multi-threading mode (`-mtmode`) | For a simmer: `igp` gives one game's simulation every thread, which makes an iteration-bounded simulation reproducible; `pgp` plays games in parallel | The job's `threading_mode` (`igp` by default, or `pgp`), stated on every games and pairs request. Opening-rack and leave requests state none |
| Small plays (`-sp`), heat map | Yes / no | No request field; reset before every task |
| Seed (`-seed`) | Yes | Required on every request: a games batch steps from it, rack `i` is analysed from `seed + i`, a leave task plays from it |
| Game pairs (`-gp`) | Yes | From the request for games; a leave task does not reset it and does not need to — `autoplay_leave_gen` never creates a second game runner and forces the divergent report off |
| PlayChooser (`-pc1`/`-pc2`) | Yes — a different move-selection algorithm | No request field; reset to off (`-1`) before every task |
| Endgame and pre-endgame solving, per player (`-eplies1/2`, `-pegbag1/2`, `-pegtopk1/2`, `-pegstride1/2`, `-pegpess1/2`, `-pegnested1/2`, and the nested schedule `-pegncaps`, `-pegndepth`, `-pegnstrides`) | Yes | `endgame_plies` and `peg_max_bag` required of every player (0 solves nothing), the rest exactly when used; each player's solver settings are reset to off first (`autoplay_solver_settings_set_defaults`), so a contributor's `-eplies1` does not leak in. No time limit applies to a solve |
| Transposition-table size (`-ttfraction`) | No — how much memory a solve may use, not what it finds | No request field; the task config's default |
| Sim margin forecast (`-smargin`, `-sm1`/`-sm2`) | Yes, for a simmer — it changes a simulated play's equity and the utility's spread term | No request field; reset to off before every task, per player and run-wide (the run-wide one is what an opening-rack analysis reads) |
| Overtime penalty and period, the run-wide endgame and pre-endgame settings (`-eplies`, `-etlim`, `-etopk`, and the `-peg*` flags without a player number other than the nested schedule above) | Only through the PlayChooser | Unreachable while it is off; autoplay, `impl_move_gen` and `impl_sim` read none of them |
| Challenge bonus (`-cb`) | No | Read only by game-history code (GCG import, challenge events) |
| `maxnumdplays`, `shplies` | Not for what is stored: a simulation's display sort covers every play, and the writers take their caps as arguments from the request. With capture on, autoplay raises a simmer's `num_plays` to `maxnumdplays` | Set from player 1 on a games task; job creation refuses a capture job whose simmers would be raised. Not set on an opening-rack task, where they reach only printing |
| Leave-generation run shape (`leavegen_max_games`, the rack target, force-draw start, written files) | Yes | Set per task: the request's `num_games`, an unreachable target, the literal `0`, files off |
| Threads (`threads` in `contribute.txt`) | For a simmer's sampling and for a multi-threaded leave run | Deliberately the contributor's — see [Known Limits and Open Questions](#known-limits-and-open-questions), KL-14 |
| Output (`-hr`, print interval, board printing, game string options, `-ritmmap`) | No | `print_interval` is reset anyway: left over, it printed simulation progress per rack |
| Data paths (`-path`) | Chooses files | Every file a task loads is digest-verified through the same resolver the load uses, and the task is declined on a mismatch |
| Lexical data already in memory (a KWG, KLV, wordmap or rack info table of the name asked for) | Whatever that file held when it was read | Evicted and read again when its file has changed since this path read it, or when this path did not read it (settings.txt, an earlier command) — `config_contribute_evict_changed_data`, by file identity (size, inode, and mtime and ctime to the nanosecond) |
| The letter distribution and board layout in memory | Whatever those files held when they were read — they were reloaded only when their *name* changed | Read again when the file is not the one this path last read, by the same identity — `config_contribute_reload_changed_ld_and_layout` |

**What is verified is a file; what is played is what is in memory.** MAGPIE
caches loaded lexical data by *name*: a load that finds a KLV of the requested
name already loaded keeps it and never opens the file. Every check on this
path, though, is of the file on disk — its digest against the job's pin, or, for
a leave task's KLV, the artifact just fetched and written to disk (under one
fixed name then; named by its content since the twelfth audit, see the Worker
API Contract). Until the eleventh audit the two could disagree, and for leave generation they
always did: from the second leave task in a process the new generation's KLV was
written over the old one and the old one, still in memory under the same name,
was played — the task's games, its per-rack counts and its mean equities all
from the previous generation's leaves, with nothing on the server able to tell.
The same held for a wordmap or rack info table rebuilt mid-run after its pin
moved, and for a lexicon updated mid-run. Before each contribute load, data
whose file's identity has changed since this path read it is dropped and read
again; data whose file has not is kept, so an ordinary run reads each file
once.

**Every name a request carries is checked before it becomes a path** — each
player's lexicon, leaves and win% model, a rack info table's name (which may
join a lexicon and leaves with one `.`), and every `expected_data` file name
(`data_filepaths_is_safe_name`). Only the job-wide names were, before; a
server that sent `../` in a table's name could have had a worker write 1.9 GB
wherever it pointed, or hash any file and report the digest back.

**A request fails on a stall, not on its length.** The HTTP client's timeout was
libcurl's `CURLOPT_TIMEOUT` at 120 seconds, which bounds the whole exchange —
upload, the server's processing and the download together. The server accepts
results up to 64 MiB, and a capture batch is tens of megabytes; on an uplink
under about 4.5 Mb/s the largest could not be sent at all, and each of the
twenty retries uploaded it all again before the finished task was dropped. It is
now a 120-second connect timeout plus stall detection (no bytes moved for 120
seconds), under a one-hour ceiling, and a redirected `POST` stays a `POST`.

- **Opening rack analysis** — for each rack in the batch, load the CGP, apply the
  single player config to both seats (the opponent a simulation plays out needs its
  leaves and settings as much as the analysed player does), copy its simulation
  settings into the run-wide ones `impl_move_gen` and `impl_sim` read — those entry
  points ignore per-player settings — seed rack `i`'s simulation from the request's
  `seed + i`, run move generation (and simulation when the player's
  `num_plies` is above 0), and read the ranked moves out of `MoveList` /
  `SimResults` — in the simulation's ranking for a simming player — including
  win%, blended utility and per-ply `bingo_percentage` and `average_score` up to
  `num_plies_recorded`.
- **Games / game pairs** — set seed, batch size, both player configs, and `-gp`
  for pairs. Read counts and score moments out of the `GameData` the autoplay
  recorder already maintains: `total_games`, `p0_wins`, `p0_losses`, `p0_ties`,
  and the score `Stat` means and standard deviations. For pairs, read the
  pentanomial and the divergent `GameData` as well. When the job sets `capture_positions`, also
  serialize the positions recorder's output (see
  [Position Capture From Games](#position-capture-from-games)).

#### Leave generation on the client

`config_contribute_leave_gen` fetches the previous generation's KLV via
`contribute_fetch_artifact`, seeds the run from the request's `seed`, passes the request's forced-rack subset straight to
`config_autoplay` as an **in-memory rack list**, runs the existing `leavegen`
autoplay type for a single generation at an unreachable rack target so the run
ends on the `leavegen_max_games` cap alone, and reads results out of `RackList`
via `rack_list_get_rack_equity_json`.

**Neither the forced racks nor the results touch the filesystem.** They arrive in
the task's JSON request and go back in its JSON response. The one file a
leave-generation task does write is the *previous generation's* KLV, which is
fetched from `GET /api/worker/artifact` and has to be on disk for MAGPIE to
load it as leaves: it goes to `lexica/<lexicon>_birdtest_<16 hex>.klv2`, under
the same directory the shipped lexicon data lives in — named by the first 16
hex digits of its verified SHA-256, written to a per-process temporary name and
renamed into place, so a name always means the same bytes. One file stays per
generation a worker plays (3.6 MB for English); nothing removes old ones, and
none is ever wrong. The name starts with the lexicon's because MAGPIE checks leaves against their
lexicon by inferring a letter distribution from each name's prefix, and a bare
name was refused before a single game was played. So `./data` must be writable
for leave generation as well as for wordmap provisioning. The `-writerackequitycsv`
flag and the CSV writer behind it are gone: a worker rendering JSON, writing it to
disk, reading it back and parsing it, all to hand it to an HTTP POST, is a round
trip through the filesystem for data that never needed to leave the process — and
it made the task depend on a writable data directory for a reason unrelated to the
lexicon data. For the same reason `leavegen`'s own per-generation KLV, leaves CSV
and report are not written in contribute mode (`AutoplayArgs.leavegen_write_files`),
and a failed write in a hand-run `leavegen` is returned as an error from the run
rather than ending the process with `log_fatal`.

#### MAGPIE reports the pentanomial

No per-game autoplay recorder is needed, but a paired run does need one thing
autoplay did not report:

- **`games` jobs.** The match test consumes wins, losses and draws, which is exactly what
  autoplay already reports. Nothing downstream ever needed individual games.
- **`game_pairs` jobs.** The pair is the unit, so the counts have to be per
  pair. MAGPIE's `-gp` mode gains a **pentanomial**: five counts indexed by
  player 1's half-point score across the pair, emitted in the contribution JSON
  as `pentanomial` alongside `all_games`. It is accumulated in the game recorder
  at the one point both games of a pair are final, consolidated across worker
  threads like every other recorder statistic.

This is a small change, and it is one MAGPIE has to make rather than something
birdtest can derive: the two aggregates alone cannot reconstruct the split, since
a 2-0 pair and two divergent 1-1 pairs are indistinguishable in them. Making it
now is also the cheapest it will ever be — `contribute` is on the unreleased
`birdtest-contribute` branch, so no deployed worker speaks the old shape.

The divergent aggregate stays, and is still reported and stored. What changed is
its status: it is a **diagnostic** of how often two configs differ at all, not
the sample anything is tested on. Testing on it conditions the sample on its own
outcome — see [The pentanomial, and why pairs are the
unit](#the-pentanomial-and-why-pairs-are-the-unit).

**The two views cross-check each other.** The pentanomial and the game aggregate
describe the same games, so they must agree on both the pair count
(`sum(buckets) * 2 == games`) and player 1's total half-points
(`Σ i·bucket[i] == 2·wins + ties`). Both are enforced at submission *and* as
`CHECK` constraints on `game_results`, because a miscounting client produces
numbers that are individually plausible and only wrong in relation to each
other — exactly the failure that would otherwise silently bias every rating pool
the job feeds.

Verified against `main` at `e4eda01`, 20 pairs, seed 50, NWL23:

| Players differ by | Divergent games | Player 1 W-L-D | Score means |
|---|---|---|---|
| `-s1 equity -s2 score` | 40 / 40 | 25-14-1 | 429.5 / 403.4 |
| `-l1 NWL23 -l2 CSW21` | 40 / 40 | 14-26-0 | 412.3 / 466.5 |
| `-k1 NWL23 -k2 CSW21` | 26 / 40 | 20-20-0 | 426.9 / 420.0 |
| nothing (same config) | 0 / 40 | 20-20-0 | identical |
| `-r1 best -r2 all` | 0 / 40 | 20-20-0 | identical |

The last row is correct rather than a bug: move *record* type governs what is
recorded, not which move is played, and static play forces `MOVE_RECORD_BEST`.

birdtest matches this: `game_records` is gone, replaced by `game_results` storing
the two aggregates plus the pentanomial, with a pairs job's test computed from the
pentanomial and the divergent counts kept only as a diagnostic.

### Wordmap and rack info table provenance

The word info table (`use_wit`, role `wit`) goes through everything below the
way a wordmap does -- named for its lexicon, built from the `.kwg` alone, its
hash pinned by the server, and a claim that pins no hash for it declined, as
for a wordmap or a rack info table.

Whether either file is used is the **job's** decision, not the client's: both
are player settings like any other, sent as `use_wordmap` and `use_rit` on each
player object (for `leave_generation`, the one `player` the bot plays both seats
as; it never loads a rack info table, since each generation plays a new KLV and
a table caches one). The config decides; a
config created without saying gets a wordmap and a rack info table (see
"Creating a player config"). Games run dramatically faster with a wordmap, so most jobs will
ask for it — but the client neither assumes it nor builds one it was not asked
for, and a file already sitting in `./data` from an earlier job is not switched
on by its mere presence.

Neither file is ever transmitted. A wordmap is 179 MB and a rack info table
1.9 GB for CSW24, roughly ten and a hundred times everything else MAGPIE ships,
so the client builds what it needs from files it already has: a `.wmp` from the
`.kwg` (about 1.3 seconds), a `.rit` from a `.klv2` and a `.wmp` (one to three
minutes, and about 2.4 GB of memory).

#### The problem the hash solves

A derived file records nothing about what it was built from, and MAGPIE's CLI
finds both by **lexicon name alone**. Four ways that goes wrong:

1. **Leaves that do not match the table.** A player config pins NWL23 words and
   CSW21 leaves, a pairing birdtest accepts on purpose (`compat.rs` compares
   alphabets, not names). With a table on, MAGPIE would load `NWL23.rit`, built
   from `NWL23.klv2`, and rank every full-rack position on NWL23's leaves
   rather than the CSW21 leaves the job pinned and the worker just verified.
2. **Leave generation.** Each generation plays with a KLV fetched for that
   generation. A table built from the shipped leaves would replace exactly the
   values being generated, and the error would carry into every later
   generation.
3. **A stale local file with the right name.** `download_data.sh` overwrites
   `CSW24.klv2` or `CSW24.kwg` in place and leaves the old `.rit` or `.wmp`
   beside it. The names still match; the contents no longer do.
4. **A stale wordmap**, the same as 3: a `.wmp` from an older `.kwg` produces a
   different set of moves.

In every case the output looks normal, and a contribution computed this way
passes every plausibility check.

#### What the server does

The wordmap half was covered by a `<lexicon>.wmp.src` sidecar holding the
SHA-256 of the `.kwg` it was built from — which catches 3 and 4 and, crucially,
**still trusts the builder**. That is not a theoretical gap: a CSW24 wordmap
built in December 2025 and one built nine months later differ in 72,852,152
bytes with the same inputs and the same wordmap format version 3, because the
builder changed and the format did not have to. Recording what a file was built
*from* cannot see that; comparing the output can.

So birdtest's server builds its own copy of each derived file with a pinned
MAGPIE, from the exact bytes the job pins, keeps the SHA-256, and discards the
file. Each claim carries them under `expected_data.derived`:

```json
"derived": [
  { "role": "wmp", "name": "NWL23", "sha256": "214a…",
    "builder": "wmp-1", "build_target": "nehalem" },
  { "role": "rit", "name": "CSW24.CSW_quackle_leaves", "sha256": "157b…",
    "builder": "rit-1", "build_target": "nehalem" }
]
```

The worker hashes what is on its disk, builds the file if it does not match,
hashes it again, and uses it only if the bytes agree. On a mismatch it declines
with `derived_mismatch`, carrying both digests, so a disagreement between the
fleet's builders shows up in the admin view instead of being worked around
silently by every worker independently.

**A table is named for its pair, not its lexicon.** `CSW24.CSW_quackle_leaves`,
not `CSW24` — case 1 above is not a check to add but a name to make
unrepresentable. `klvwmp2rit` takes the KLV's and the wordmap's names separately
so the output does not have to borrow one of theirs.

**A hash is tied to its builder.** MAGPIE carries `WMP_BUILDER_VERSION` and
`RIT_BUILDER_VERSION`, separate from `MAGPIE_VERSION` because a builder change
need not touch a file format, and a pinned-hash test
(`test/builder_hash_test.c`) fails until a change that alters either builder's
output bumps its version. The server reads the versions from the binary it runs
(`magpie builders`) rather than from configuration, so the builder recorded
beside a hash is always the one that produced it.

**The build target is recorded, not enforced.** Measured on x86-64 with GCC 10:
`-march=native` and `-march=nehalem` produce byte-identical wordmaps and rack
info tables, for a two-letter test lexicon and for NWL23, and so do one thread
and eight. So a worker whose target differs builds the file and compares rather
than declining unseen — refusing on the field alone would lock out every
contributor who builds from source in exchange for nothing. The MAGPIE release
build and the server image both use `portable_release` regardless, because
being right by construction is better than being right by measurement.

#### What the worker does

Before running a task, for each derived file the claim pins:

1. Hash what is on disk, through the run's digest cache (a 1.9 GB table takes
   about nine seconds to hash, so this must be once per file and not once per
   task). If it matches, use it.
2. Otherwise build it — `convert dawg2wordmap` for a wordmap, `convert
   klvwmp2rit` for a table — and hash the result.
3. If it still does not match, record both digests and decline the task with
   `derived_mismatch`. The job is remembered as unsupported, so the worker does
   not spend another three minutes rebuilding a table it has just found it
   cannot match.

A claim for a player that uses a wordmap or a table always pins one: the
server does not dispatch the job until it has built and hashed it. MAGPIE
declines a claim that asks for either without pinning it (`derived_mismatch`,
`config_contribute_ensure_wordmap`, `config_contribute_ensure_rack_info_table`)
rather than play with a file nothing verified. A wordmap with no pin used to
fall back to a `<lexicon>.wmp.src` sidecar recording the `.kwg` it was built
from, for "an older server"; birdtest has never been one, so the fallback and
the sidecar went (October 2026).

`dawg2wordmap` replaced the `dawg2text` + `text2wordmap` pair the client used to
run. The two produce identical bytes, this is the one the server builds its
reference copy with, and it writes no intermediate `.txt`.

**Whether either file is used is decided as the lexicon loads**, so the client
states every flag before it loads a task's lexicon, never after. Set afterwards,
a flag applied to the *next* task: a task that had not asked for a wordmap got
the previous task's choice, and a table switched on by a contributor's
`settings.txt` stayed on.

Both write into `./data`, which is **assumed writable**. If it is not, that is a
clear error and `contribute` stops — there is no fallback location. A job that
asked for neither file never reaches this path.

MAGPIE can open a third file by lexicon name as it loads: a **word info table**
(`.wit`), a per-substring letter mask move generation prunes with. It is
opt-in on the CLI (`-wit`), but on by default here: a player config states
`use_wit`, which the API defaults to true (it speeds move generation and costs
a few seconds' build and ~122 MB for CSW24), and every request states the flag for each player, so a
contributor's `settings.txt` — or an earlier command in the same process —
cannot carry one into a task. Built from the lexicon on disk a table prunes
nothing legal; built from an older one, or by an older builder, it prunes plays
that exist. So it is handled like the other two: the server builds one from the
`.kwg` alone (`convert kwg2wit`, builder `wit-1`, about three seconds and
122 MB for CSW24), pins its hash as a `wit` entry in `expected_data.derived`,
and the worker builds its own and uses it only if the bytes agree. Unlike a
rack info table it holds nothing a leave generation changes, so a leave job's
player may use one. Until 2026-10 birdtest offered no setting for it, and
contribute switched it off for both players before every load.

#### Leave generation keeps tables off

Every generation plays with a different KLV, so a table — which caches leave
values — would have to be rebuilt per generation at 1.9 GB and several minutes
on every worker, to replace exactly the values being generated. The server pins
none for a `leave_generation` job and the client loads none.

#### Where the builds run

On the server, in a separate scheduled ECS task (`infra/derived.tf`), not in the
web task: a table build peaks at about 2.4 GB and writes a 1.9 GB file, against
the web task's 1 vCPU and 2 GB. It builds from a queue (`derived_data`), at most eight files a run, under a lease
and exits. **A job whose derived files are not built is not dispatched** — the
same wait as a leave-generation job whose universe is not seeded — because
dispatching without the hash would carry no hash for a file its player asks
for, which MAGPIE refuses (`derived_mismatch`), setting the job aside for the
run on every worker that claims it. `/admin/derived-data` is where
that wait is visible.

The gate is a query over the job's config, its players, `input_data` and
`derived_data`, and it runs before the dispatch lock for every candidate job on
every claim. Once a job has been found dispatchable the answer is remembered
in the process for good (`derived::DerivedCache`): what a job needs is fixed at
creation, a `derived_data` row only ever moves toward `built`, and the builder
the query matches on is a constant of the running binary, so nothing can make a
remembered answer wrong. A job still waiting is asked about on every claim,
which is what lets it be dispatched the moment its last file is built.

Files are queued when a job is created and when it is activated, and by the
first claim that finds one of a job's files with no row under the running
binary's builder (`derived::ready_for_job`). That last is what a deployment
whose MAGPIE bumped a builder version needs: every running job's files are
built under the old one only, and until the thirty-first audit nothing queued
them under the new one, so every such job handed out nothing, with nothing on
`/admin/derived-data` to say why, until an admin re-activated it.

On the worker, during task execution, after the heartbeat has started: a table
takes minutes, and the heartbeat is what keeps the claim alive through it.

### Heartbeat thread

`POST /api/worker/heartbeat` with `{"claim_token": "..."}` every 30 seconds for
the lifetime of a claim — through the result's submission, see the loop's step 8
— using `cpthread` and a stop flag. Each goes out once, with no retry: the next
one is thirty seconds away whatever happened to this one — a `429` included,
which is not waited out: a heartbeat asleep on a `Retry-After` would hold up the
task's submission, which waits for the heartbeat thread to stop. Failures are ignored:
the server treats a missed heartbeat as a lapsed claim and reassigns the
task, which is the designed behaviour.

The heartbeat starts *before* task execution, because wordmap generation and a
large batch both happen inside it — but *after* data verification, because hashing
takes single-digit milliseconds and a decline should not look like a worker that
started and died.

### Version negotiation replaces self-update

The Python client re-execed itself from a newer script the server offered. MAGPIE
cannot responsibly do that: it is a compiled binary, and an auto-updating
executable is a much larger security proposition. Instead the client states its
version on every claim and the server filters — see
[MAGPIE version negotiation](#magpie-version-negotiation).
`GET /api/worker/client-version` accordingly reports the minimum MAGPIE version
(and, for a human, where to get it) rather than a script to fetch. No worker
calls it: MAGPIE learns the floor from the shutdown directive of a claim it is
too old for. The admin new-job form reads the floor from it.

### The Worker API Contract

Six endpoints. Authentication on all of them is either
`Authorization: Bearer <api-key>` **or** `X-Worker-UUID: <uuid>`, never both. A
claim may also carry neither, which is how a new worker asks to be issued a
UUID; every other endpoint answers `401` without an identity, since each acts on
something a claim created.

#### `POST /api/worker/task`

The body is required:

```json
{ "magpie_version": "1.4.0", "board_dim": 15, "rack_size": 7,
  "unsupported_jobs": ["4c7b64ad-8e5e-4db7-aeb0-afc44ee1ebf5"] }
```

Every field is load-bearing, though only the first three are required:
`unsupported_jobs` is read as empty when it is omitted. The version drives the
per-job minimum filter — without it the server would have to assume one, which is a wrong answer dressed as
a safe one; `board_dim` and `rack_size` are the build's compile-time `BOARD_DIM`
and `RACK_SIZE`, and anything but 15 and 7 is answered `unsupported_build` (see
[MAGPIE version negotiation](#magpie-version-negotiation)); and `unsupported_jobs` is every job this worker has found it cannot
run, for any reason. It is attacker-controlled input flowing into a query, so it
is capped at **200** entries and silently truncated past that, keeping the
newest (far above any
honest client, since the list is bounded by the jobs a worker has actually been
offered) and bound as an array rather than interpolated. A claim whose body is
missing, malformed or without `magpie_version`, `board_dim` or `rack_size` is rejected `400` with a message
that names the fix — what to send, and that a MAGPIE which does not send them
predates the protocol and needs updating — rather than the parser's complaint
alone, because that error is what a stale MAGPIE build will show a contributor
after launch. (For a while it *was* a bare `422`, in plain text: the rejection
came from the framework, before any handler ran.)

`204` when there is no work right now — no body, so a request that arrived with no
identity is not assigned a UUID here; it tries again with no identity next time,
and gets one for keeps once a task is actually available.

`200` with a `shutdown` object when every active job is ruled out for this worker:

```json
{ "shutdown": {
    "reason": "data_out_of_date",
    "message": "Every active job needs input data you do not have.",
    "required_tarball_dates": ["20260101"],
    "required_magpie_version": null,
    "download_url": null } }
```

`reason` is `data_out_of_date`, `magpie_too_old`, `both`, or
`unsupported_build` (a build whose `board_dim` or `rack_size` is not 15 and 7,
answered with nothing to download and no data to fetch: only a rebuild with
MAGPIE's defaults fixes it).

`200` with a task:

```json
{
  "claim_token": "6f3d7198-178a-47c8-9ccc-6aa6995a5a9c",
  "job_id": "4c7b64ad-8e5e-4db7-aeb0-afc44ee1ebf5",
  "job_name": "NWL23 4-ply simmer vs static",
  "max_task_seconds": 3600,
  "min_magpie_version": "1.4.0",
  "worker_uuid": "6f3d7198-178a-47c8-9ccc-6aa6995a5a9c",
  "expected_data": {
    "algorithm": "sha256",
    "files": [
      { "role": "kwg", "name": "NWL23", "path": "lexica/NWL23.kwg",
        "sha256": "3e74af98...", "tarball_date": "20260925" }
    ]
  },
  "task_request": { "job_type": "games", "...": "..." }
}
```

`expected_data` lists every file this task will load — the deduplicated union over
the job and its players — with the digest the job pins. `role` and `name` are what
the client resolves through `data_filepaths`; `path` and `tarball_date` are for the
message it prints when something does not match. No entry states a size, here or in
`derived`: the client reads none (the thirty-third audit removed a `bytes` key that
every entry carried and nothing on the worker side read). A `leave_generation` task carries
exactly three entries: `kwg`, `letterdist`, `layout`.

`job_name` and `max_task_seconds` are always present. `job_name` is what the
worker calls the job when it says what it is running: the admin's name for it,
or for a job created without one its type and the start of its id
(`games job 1d4a7f60`), never empty. `max_task_seconds` is how long the worker
may run this task, a whole number of seconds from 600 to 86,400: the limit as it
stood when this claim was made, from which the claim's deadline was set. A
worker that reaches it stops the task, hands it back unfinished and declines it
`time_limit`; a minute past the deadline the claim lapses whatever the worker
does, and a result for it is answered `accepted: false` — either counted
against the job as a task that hit the limit, as the decline is, if the worker
was still alive (see [Task time limit](#task-time-limit)).

`min_magpie_version` is always present. `worker_uuid` is present **only** when the
request carried no identity at all and the server just minted one; the client
persists it and sends it as `X-Worker-UUID` from then on.

`task_request` is internally tagged by `job_type`, one of four shapes. **No
request carries a top-level `lexicon` except `leave_generation`**, which has one
bot and no player object to hold it; every other job type states each player's
lexicon on that player. Every shape states `letter_distribution` and
`board_layout` — the job-wide files the worker has just verified by digest — and
the worker applies both rather than whatever its own settings last loaded.

```json
{ "job_type": "opening_rack",
  "variant": "classic", "letter_distribution": "english", "board_layout": "standard15",
  "racks": ["AABCELT", "AABCELU"],
  "seed": "0",
  "bingo_bonus": 50, "sim_cutoff": 0.005,
  "player": { "name": "static", "recorder_type": "all", "sort_strategy": "equity",
              "lexicon": "NWL23", "leaves": "NWL23",
              "num_plies": 0, "num_plies_recorded": 2,
              "num_plays": 100, "num_plays_recorded": 10,
              "movegen_margin": 5.0,
              "use_wordmap": false, "use_rit": false, "rit_name": null,
              "win_pct_model": null, "max_iterations": null,
              "stopping_pct": null, "use_inference": null, "time_limit_secs": null,
              "min_play_iterations": null, "threshold": null, "sampling_rule": null,
              "inference_margin": null, "utility_w_winpct": null,
              "utility_w_spread": null, "utility_spread_scale": null } }

{ "job_type": "games",
  "variant": "classic", "letter_distribution": "english", "board_layout": "standard15",
  "seed": "1", "num_games": 10,
  "capture_positions": false, "capture_first_divergence": false,
  "bingo_bonus": 50, "sim_cutoff": 0.005, "threading_mode": "igp",
  "player1": { }, "player2": { } }

{ "job_type": "game_pairs", "...": "as games; num_games counts pairs",
  "num_games": 10 }

{ "job_type": "leave_generation",
  "lexicon": "NWL23", "variant": "classic", "letter_distribution": "english",
  "board_layout": "standard15",
  "generation": 2,
  "seed": "7",
  "forced_racks": ["AABCELT", "AABCELU"],
  "previous_artifact_key": "leaves/<job>/generation-1.klv2",
  "previous_artifact_sha256": "<the sha256 recorded when the server built it>",
  "bingo_bonus": 50,
  "num_games": 10000,
  "player": { } }
```

The player above is a **static** one, and it is the whole shape: the settings
every player states are numbers, never null — MAGPIE refuses a player without
`recorder_type`, `sort_strategy`, `num_plies`, `num_plays`, `num_plies_recorded`,
`num_plays_recorded` or `movegen_margin` — and the simulation settings are null
because `num_plies` is 0. A simmer states all of those as well. The committed
fixtures under [`contract-fixtures/`](contract-fixtures/) are the authority on
the shape; these are for reading.

`racks` is a batch, not a single rack: the rack space runs to millions and one
rack per task would spend a claim/submit round trip on each.

`previous_artifact_key` is **never null**, generation 1 included: the server
builds a zeroed KLV for it at `generation-0` when the job is created, so every
generation fetches its leaves the same way and the client has no first-generation
branch. `previous_artifact_sha256` is the hash of the bytes the object store
holds for that KLV — `leave_generation_artifacts.served_sha256`, which every
rebuild sets from the object as it finds or leaves it, else `sha256`, the hash
recorded when it was built; MAGPIE checks the fetched bytes
against it before playing a game, and writes them under a name made from it
(`<lexicon>_birdtest_<16 hex>`), so a name always means the same bytes — the
load's name-keyed cache cannot mix generations, and two `contribute` processes
sharing a data directory cannot overwrite each other's. It was one fixed name per
lexicon, overwritten every task, and taken on trust.

`num_games` is the only thing that ends a leave-generation task: play that many
games, then report. The generation's minimum rack target is **not** sent, and the
client must not stop early on it. Every game contributes occurrences for every
rack it draws, not just the task's `forced_racks`, and the server folds all of
them into its per-generation totals — so games played after the forced racks have
filled still produce coverage the server uses. The target belongs to the server,
which owns the running per-rack totals across every task in the generation and
decides on its own when the generation closes.

`threading_mode` is on every games and pairs request, and on no other: `igp`
gives all of the task's threads to one game's simulation at a time, which makes
a simulation bounded by iterations reproducible, and `pgp` plays the batch's
games in parallel, a thread each. It is the job's (`threading_mode` on its
config row, `igp` unless the job was created with `pgp`), and changes nothing
for static players.

`seed` is a **decimal string** on every request, because it is a `uint64` and
JSON numbers are doubles. Every task states one: games and pairs play their
batch from it, an opening-rack task analyses rack `i` from `seed + i`, and a
leave-generation task seeds its games from it. For `game_pairs`, `num_games` counts *pairs*; MAGPIE plays two games per
pair.

#### `POST /api/worker/decline`

```json
{ "claim_token": "6f3d7198-178a-47c8-9ccc-6aa6995a5a9c",
  "reason": "missing_data",
  "missing": [ { "role": "kwg", "name": "CSW24",
                 "expected": "3e74af98...", "actual": null } ] }
```

`204`. `reason` is `missing_data`, `magpie_version`, `unknown_job_type`,
`derived_mismatch` — the worker built the wordmap, rack info table or word info
table the job pins and got different bytes, or (role `klv`) the leave KLV it
fetched does not hash to `previous_artifact_sha256` or is not there (`404`);
that one is the server's to
fix, so the worker sets the job aside only for a while — sent as unsupported for
the idle interval, doubling per job to ten minutes, then claimed and its KLV
fetched afresh, so a repair is noticed — and goes on with other jobs meanwhile (a
`data_out_of_date` shutdown answering a claim that named such a job is waited
out, not obeyed; a `magpie_too_old` or `unsupported_build` one is obeyed at once) —
or `task_failed` — the worker ran the task and could
not produce a result the server accepted — or `time_limit` — the worker stopped
the task at the assignment's `max_task_seconds`, unfinished (the body is the
token and the reason, `missing` left out);
`missing` is present for `missing_data` and `derived_mismatch`, where `actual`
is the digest of what the worker built. `actual: null` means the file was not
found at all, and a hex string means it was found with different content.

The server derives the task and job from the token, releases the claim immediately
rather than waiting out the heartbeat timeout, and records the gap so an admin can
see what the fleet is missing. Declining is an ordinary outcome, not an error:
for `missing_data`, `magpie_version` and `unknown_job_type` the worker adds the
job to its unsupported set and claims again; `derived_mismatch` does not count
as a failure either — a file built here that does not match sets the job aside
for the run, the server's KLV for a while, doubling (above); `task_failed`, for
a task that failed or a result refused, counts as one, five in a row ending
its run (a task handed back because the worker is stopping is sent as
`task_failed` too, and counts as nothing). `time_limit` is counted against the
job, not the worker: three in a row with no task of the job completed between
— declines, and claims lapsed at their deadline with the worker alive, alike —
set the job aside ([Task time limit](#task-time-limit)). Outside leave generation, the server does not offer a worker a task
it declined within the hour (counted from the claim's last heartbeat); a
declined leave task is reissued as it stands.

#### `POST /api/worker/heartbeat`

`{"claim_token": "..."}` → `204`.

#### `POST /api/worker/result`

`{ "claim_token": "...", "movegens": 123456, "result": { } }` → `200` with
`{"accepted": true}`, or `{"accepted": false}` when the claim had already lapsed
— its heartbeats stopped, or it is a minute past its deadline — or the result
was already accepted — which is **not an error**: the work was reassigned or
is done. `400` when the result does not satisfy its shape, or
`movegens` is missing, not a whole number from 0 to `i64::MAX`, or more than a
million per millisecond the claim was held; `413` over 64 MiB. `movegens` is
the move generations MAGPIE performed for the task, on every thread, which its
contributor is credited with (see "The stats payload").

```json
{ "racks": [ { "rack": "ABDEELT", "num_moves": 412,
               "moves": [ { "move": "8D BEADLET", "score": 76, "equity": 81.5,
                            "iterations": 1000, "win_percentage": 61.2,
                            "blended_utility": 0.58,
                            "plies": [ { "ply": 0, "bingo_percentage": 0.0,
                                         "average_score": 24.0 } ] } ] } ] }

{ "all_games": { "games": 20, "wins": 11, "losses": 9, "ties": 0,
                 "p1_score_mean": 429.5, "p1_score_sd": 60.8,
                 "p2_score_mean": 403.4, "p2_score_sd": 55.9 },
  "positions": [ ] }

{ "all_games": { "...": "as above" },
  "pentanomial": [12, 3, 140, 5, 40],
  "divergent_games": { "...": "same shape, the divergent subset" } }

{ "racks": [ { "rack": "AABCELT", "count": 30, "mean": 1.5 } ] }
```

Server-side validation, so the client must satisfy it:

- `wins + losses + ties == games`, all non-negative.
- `game_pairs`: `games` is even and non-zero; `pentanomial` is required and must
  agree with the aggregate on the pair count and on player 1's half-points;
  `divergent_games`, if present, has consistent counts with `games` even and
  `<= games`, and is consistent with the whole: a pair that did not diverge
  played one game from both seats, so it is a win and a loss for player 1 or
  two draws, in pentanomial bucket 2. So the games outside the subset have as
  many wins as losses and an even number of draws, and no more pairs fall
  outside bucket 2 than diverged.
- `moves` and `racks` must be non-empty. `moves` carries at most the player
  config's `num_plays_recorded`; `num_moves` says how many were ranked and must
  not be below the number reported, and is required.
- A position's analysis is one its task's players could have run: a
  simulation (and an inference) only if a player simulates (and infers), an
  endgame or pre-endgame solve only if a player solves one, an endgame at no
  depth past the deepest `endgame_plies` and a pre-endgame at none past the
  deepest its schedule reaches (stage `s` at `s + 1` plies; 40 for the
  exhaustive one-stage `[2147483647]`). An unsimulated move — static or solved —
  carries no iterations and no per-ply statistics. A static analysis from a
  simming player is possible (a simulation with no plays to rank) and passes.
- `positions` is present only when the job set `capture_positions`, and each entry
  must fall inside the task's own games; a pairs job keeping first divergences
  sends exactly two per diverging pair, at one turn of one position, and as
  many pairs as `divergent_games` says diverged — see
  [Position Capture From Games](#position-capture-from-games).
- A leave result names full 7-tile racks only, each once, each with a `count` of
  at least 1 and a finite `mean`; and the counts together may not exceed
  `num_games` × 1,000, which no run of that many games can reach.

#### `GET /api/worker/artifact?key=<key>`

Returns `application/octet-stream`. Only keys the server itself minted resolve;
anything else is `404`. Used for a generation's KLV.

#### `GET /api/worker/client-version`

`{"min_magpie_version": "...", "download_url": "..."}` — the oldest MAGPIE a
client may contribute with, and where to get it. Not a self-update, and not
called by MAGPIE (which learns the floor from a claim's shutdown directive): it
is for the admin new-job form, which pre-fills a job's floor from it, and for
humans.

#### Rate limiting

Worker endpoints are limited to one request a second per credential for claims
and another for work in hand (heartbeats, declines, results, artifacts), each
with a burst of five. A `429` carries `Retry-After` in seconds. A task costs at least two
requests, so this is reached under normal operation and must be handled as
backoff, not as an error.

### Client security

- TLS certificate verification on by default, with no way to disable it.
- Nothing requires TLS: a `server http://…` is accepted. The deployment refuses
  `http://…/api/*` at the load balancer (`426`, saying why) rather than
  redirecting it — a followed redirect sent the credential in the clear on every
  request, silently — so a misconfigured worker fails on its first request, the
  one that has already disclosed its credential (KL-84).
- The API key is never accepted on the command line, never written to
  `settings.txt`, and never appears in status output, logs or errors; no
  settings error quotes a value either, since a key appended to a last line
  with no newline landed in another setting's (MAGPIE's `test_client_state`,
  thirty-second audit, pass 16). The status line does print the `server`
  value, so a bare key glued to it with no space would show there (KL-68).
- The worker UUID is minted by the server, never trusted from the client, so a
  client cannot pick or collide an identity on its own.
- **Every field of a task request is untrusted input.** It becomes file paths
  (`previous_artifact_key`) and numeric parameters. Lexicon and variant names
  are held to a safe character set before they reach `data_filepaths`, so they
  stay inside the data directory, and artifact keys containing `..` or a
  leading `/` are refused. The worker UUID the server assigns is taken only in
  canonical form: it becomes a header and a settings line, and a newline in it
  wrote settings every later run obeyed (thirty-second audit, pass 14).
- **Sizes are the server's to choose.** Batch sizes, play counts and rack
  counts are checked for sense (positive, present), and response bodies not at
  all; none is bounded:
  the worker trusts its server's sizes, as AUDIT_FINDINGS_19 decided, and a
  server that asks for too much ends the worker's run (KL-85).

### GUI integration surface

In async mode `contribute` must expose **state** (idle / claiming / working /
submitting / stopped / error), **progress** (current job type, games completed
within the current task), **totals** (tasks completed this session, and the
identity being credited), and the **last error** in a form suitable for display.
These go out through the existing `-hr false` machine-readable convention so the
GUI parses one format.

### The client stops being birdtest's code

Organisational as much as technical. The HTTP API is a **cross-repo integration
boundary** between two independently released programs:

- A client bug is a MAGPIE bug, fixed on MAGPIE's cadence, reaching contributors
  only when they update.
- A server change can break every deployed client. `min_magpie_version` is a
  floor, not a ceiling, so it does not stop an old server confusing a new client.
  The worker API should be treated as frozen and extended only additively — with
  the explicit exception of the pre-release window, during which the lexicon was
  removed from `GameRequest` and `OpeningRackRequest` in one coordinated change
  across both repositories, with no version gate and no shim. That window closes
  at launch, and it closes for every field: anything known to be wrong gets fixed
  before the first release.
- The contract used to be pinned by nothing, existing implicitly in MAGPIE's
  `config_contribute_*` functions and birdtest's `routes/worker.rs` agreeing.
  [`contract-fixtures/`](contract-fixtures/) is the cheap version of fixing
  that: one committed example of each message either side has to produce or
  read (eighteen files; `contract-fixtures/README.md` lists them) — an
  assignment of each of the four request shapes (games, game pairs, opening
  racks, leave generation) carrying `expected_data`, a new worker's first
  assignment with its issued UUID, the `expected_data` block alone, a
  heartbeat, a result of each kind (games with and without an inference),
  a claim carrying `unsupported_jobs`, `magpie_version`, `board_dim` and
  `rack_size`, a decline, and each shutdown reason.
  Opening racks earn their own fixture because theirs is the one request that
  carries `racks` and a single `player` rather than a player pair, so nothing
  else pins those two names.

  birdtest's half is enforced (`routes::worker::contract_fixtures` parses every
  fixture against the real wire types, comparing field structure rather than
  bytes so fields stay free to move before release). MAGPIE's half is now too:
  every JSON fixture is copied byte for byte into MAGPIE's
  `test/birdtest_contract/` (CI replaces that copy with the branch's own set
  before it runs the tests), and `test/contribute_test.c` fails if any key the
  executors read is missing from an assignment fixture, if a result serializer
  stops producing a key a result fixture carries, if the claim body lacks a key
  of `claim-request.json` or states another `board_dim`/`rack_size` than the
  build's, or if a shutdown reason is not waited out or obeyed as intended.
  What is still traced by hand is KL-48's.

### MAGPIE-side implementation notes

Four decisions in the MAGPIE `contribute` implementation that are not obvious from
the code and expensive to re-derive.

#### Why `AutoplayResults` carries `char *leave_results_json`

Because the producer and the consumer of that string are separated by a function
boundary with nowhere else to carry it, and the data it is built from is dead
before the consumer runs.

The string is produced in `postgen_prebroadcast_func` (`src/impl/autoplay.c`), the
checkpoint callback that fires when a leavegen generation closes — the *only*
moment the `RackList` is both fully populated for the generation and still alive.
It is consumed in `config_contribute_leave_gen` (`src/impl/config.c`), after
`config_autoplay` returns. Everything in between is gone by then:
`LeavegenSharedData`, which owns the `RackList`, is created inside `autoplay()`
and destroyed inside it, and the callback gets only a `void *` to
`AutoplaySharedData` with no handle on the caller.

So the question is really: where can a leavegen run park a string so the caller of
`autoplay()` can pick it up? The candidates:

- **A file.** What the code did before, via `-writerackequitycsv`. Gone now — a
  round trip through the filesystem for data that never needed to leave the
  process, and a dependency on a writable data directory for a reason unrelated
  to the lexicon data.
- **A file-static in autoplay.c.** Mechanically fine, but global mutable state:
  two `autoplay()` runs in one process would clobber each other, and MAGPIE has no
  other global like this. The `contribute` loop happens to run one task at a time
  today, which makes it safe today — a property nothing enforces and nothing
  states.
- **An out-parameter on `autoplay()` / `config_autoplay()`.** Threads a
  leavegen-only `char **` through two signatures every autoplay caller uses, so
  `autoplay games 100` grows a parameter only `leavegen` ever writes.
- **`AutoplayResults`.** The object that already exists for exactly this purpose.
  It already outlives the run (owned by `Config`, not by `autoplay()`), is already
  reachable from the callback (`LeavegenSharedData` holds
  `primary_autoplay_results` precisely so postgen can write into it), and the
  caller already has it in hand. No new lifetime, no new plumbing, no new global.

The cost is one pointer on a struct that non-leavegen runs leave NULL, plus a
`free` in `autoplay_results_destroy`. That is the cheapest of the four. One wrinkle
worth knowing: the field is not reset by `autoplay_results_reset`, which only
resets recorders. It is freed and replaced on every write, so a multi-generation
run keeps the last generation's string rather than leaking each one.

#### Fixed-size `CAPTURED_*_STRING_SIZE` arrays vs. dynamic allocation

First, a correction to the usual framing: these arrays are not on the stack. They
are inline members of `CapturedPosition` and `CapturedPlay`, and both live in
heap-allocated arrays. The real trade-off is *inline fixed-size field* vs.
*pointer to a separate allocation*.

Why the fixed size wins here:

- **It removes a malloc/free pair per string, at capture rate.** A position is
  captured on every turn of every game. At ~22.5 turns a game, a 100-game batch
  captures ~2,250 positions; each holds 3 strings and each stored play holds 1.
  With a play cap of 15 that is ~40,000 strings — zero allocator calls inline, or
  40,000 mallocs and frees on the hot path, all tiny, all contending across worker
  threads.
- **It makes the writers bounded.** `rack_get_string`, `move_get_string` and
  `game_get_cgp_string` all take `(char *dest, size_t dest_size)` and truncate,
  which is why `append_bounded` / `append_int_bounded` exist. There is no
  measure-then-allocate-then-format pass and no `StringBuilder` churn.
- **It keeps the array contiguous and the position a value.** `data->positions` is
  one block that `realloc`s by doubling; growing it moves bytes rather than
  chasing and re-pointing 3 pointers per element.
- **It makes the free path trivial** — one `plays` array per position, not four
  strings per position plus the arrays.

The code does not apply this dogmatically. The plays list per position **is**
dynamically allocated, because a position can legally have hundreds of ranked
plays and there is no honest fixed bound. Fixed size is chosen where a tight bound
exists (a rack is `RACK_SIZE` tiles, a move covers at most `BOARD_DIM` squares, a
CGP is at most a full board plus two racks) and rejected where it does not.

The bounds are sized for the longest human-readable letter any distribution MAGPIE
actually ships (`MAX_SHIPPED_LETTER_BYTE_LENGTH = 4`, Catalan's `L·L` with its
U+00B7 middle dot), not for `MAX_LETTER_BYTE_LENGTH = 6`, which is only the
parser's ceiling:

| Field | Formula | Bound | English worst case | English typical |
|---|---|---|---|---|
| `rack` | `RACK_SIZE * 4 + 1` | **29** | 8 | 8 |
| `previous_move` / `move` | `BOARD_DIM * (4+2) + 16` | **106** | ~36 | ~12 |
| `cgp` | `225*4 + 15 + 2*29 + 64` | **1037** | ~269 | ~130 |

`sizeof(CapturedPosition)` is 1,240 bytes (1,172 of it inline strings) and
`sizeof(CapturedPlay)` is 328 (106 inline `move`), so one position with 15 stored
plays is **6,160 bytes**. A pointer-based equivalent for the same English position
would be ~350 bytes and 18 allocator round trips, so the inline form costs roughly
**4–5 KB more per position**, or 4–5 MB per 1,000 — a 100-game batch sits around
6 MB instead of around 1 MB.

That is the honest number. Whether it is the right trade depends on the ceiling
rather than the average, and the ceiling here is bounded by the batch size the
server hands out. With the CGP bound sized to the shipped distributions rather
than the compile-time ceiling, most of what remains is the per-play `move` buffer
multiplied by the play cap; cutting further would mean capturing the CGP's letters
as machine letters, or sizing buffers from the loaded `LetterDistribution` at run
time, which brings back an allocation per capture.

#### Why `autoplay_results_reset(primary)` moved into the consolidate loop

It moved rather than vanished. `autoplay_results_consolidate` used to reset the
whole primary up front; it now resets each recorder inside the loop, after the
`continue` guard:

```c
for (int i = 0; i < NUMBER_OF_AUTOPLAY_RECORDERS; i++) {
  if (!autoplay_results_list[0]->recorders[i]) continue;
  recorder_reset(primary->recorders[i]);    // <- moved here
  ...
```

The change was made in `95a9f66e`, when the positions recorder briefly became a
single structure shared live across every worker thread. In that design
`positions_data_consolidate` was a genuine no-op — every capture had already
landed in the one shared list — so the primary's positions recorder was *not* a
blank merge target the way every other recorder is. It held the entire run's data,
and resetting it before "merging" would have thrown the run away.

The reset had to be scoped rather than deleted, because the other recorders (game
data, FJ, win%, leaves) genuinely do need clearing: consolidation sums per-thread
totals into the primary, so a primary carrying a previous consolidation's numbers
would double-count. Putting `recorder_reset` inside the loop says exactly the right
thing: *reset the merge targets you are about to merge into, and nothing else.*

That is still why it stays there. The positions recorder has since gone back to
per-thread arrays with a real consolidate step (`01a8e704`), so today the two forms
would behave the same for the recorders a run actually has. But the in-loop form is
the one that states the invariant, and the one that keeps working if a recorder ever
again holds state consolidation does not rebuild. It also stops the reset from
touching the primary's positions shared JSON on iterations that skip the merge —
`positions_data_reset` frees `shared_data->json` when the recorder owns the shared
data, which is a real side effect on an object other code reads.

#### Why contribute leavegen needs `leavegen_max_games`

Because `leavegen` has no max-games setting of its own. That is the whole reason
the field exists.

```
leavegen <min_rack_targets> <games_before_force_draw_start> [forced_racks_file]
```

Neither of the first two is a game cap. **`min_rack_targets`** is a comma-separated
list with one entry *per generation* — `100,200,500,1000,1000,1000` means six
generations at those per-rack occurrence targets, and `autoplay()` derives
`num_gens` from its length. **`games_before_force_draw_start`** is the one that
*looks* like a game count and is probably the source of the impression, but it is
how many games into a generation to play before forced draws begin — a warm-up, not
a limit. It only ever turns forcing *on*.

A generation ends when `rack_list_get_racks_below_target_count() == 0`. Nothing
else stops it:

```c
first_gen_num_games =
    args->leavegen_max_games > 0 ? args->leavegen_max_games : UINT64_MAX;
```

With `leavegen_max_games == 0` — the CLI's behavior, unchanged — the iteration cap
is `UINT64_MAX`. A hand-run `leavegen` is unbounded in games by design: you say
what coverage you want and it plays until it has it.

That is fine for an interactive run on a full rack universe. It is not fine for a
distributed task. A `leave_generation` task gets a *subset* of racks and the
generation's target belongs to the server, which is accumulating counts across
every task in the generation — no single task can reach it or even observe whether
it has been reached globally. Without a cap, a task whose forced-rack subset
happens to be slow to fill would run forever: holding its claim, missing no
heartbeat, never submitting.

It would be tempting to also stop early once the task's *own* forced racks have each
occurred some target number of times. That is wrong. A game contributes an occurrence for *every* rack it draws, not only the
forced subset, and the server's upsert into `leave_rack_progress` does not filter
against `forced_racks`. Games played after the forced racks are "done" still produce
coverage the server uses, and stopping early throws it away. The generation's rack
target is therefore server-only state and is not sent in the request at all; the
client passes `leavegen` a target it cannot reach, so termination is purely by game
count.

One subtlety: `leavegen_max_games` caps the **whole run**, not each generation.
`shared_data->max_iter_count` is set once from `first_gen_num_games` and never
raised between generations. For the contribute path that is exactly right, because a
task is a single generation, so whole-run and per-generation are the same thing. It
would matter if a multi-generation run ever set the field; nothing does today, and
the field's comment says so.

---
## API

All endpoints return JSON. State-mutating, session-cookie-backed endpoints (Auth, Account, Admin APIs) require a valid CSRF token; Worker API endpoints are exempt despite also being state-mutating, since they authenticate via bearer token or `X-Worker-UUID` rather than a cookie a browser would send automatically — see [Security](#security) for the full rationale. Worker endpoints accept either an `Authorization: Bearer <api-key>` header (authenticated workers) or no auth header plus an `X-Worker-UUID` header (anonymous workers).

### API Conventions

Shared by every endpoint, so individual routes below only state what differs.

#### Error responses

Every failure is JSON with the same shape, whatever the status:

```json
{ "code": "bad_request",
  "message": "registration details are invalid",
  "fields": [ { "field": "password", "message": "too weak — choose a longer, less predictable password" } ] }
```

`code` is a stable machine-readable string (`bad_request`, `unauthorized`,
`forbidden`, `not_found`, `method_not_allowed`, `conflict`, `payload_too_large`,
`rate_limited`, `unavailable`, `internal`) mapping one-to-one onto the status. An
unknown endpoint is `not_found` and a method an endpoint does not take
`method_not_allowed`, in the same shape. `unavailable` is `503` with a `Retry-After`:
every connection of the pool asked was busy for the whole acquire timeout, or a
display read outran its statement timeout (see [Two connection
pools](#two-connection-pools)). That is load rather than a fault, and MAGPIE's
client already backs off and retries a `5xx`. `fields` is omitted when empty and carries per-field
messages so form endpoints can mark individual inputs. A `rate_limited`
response also carries a `Retry-After` header in whole seconds.

**So is a path or query string that does not parse** (`extract::ApiPath`,
`ApiQuery`): a malformed id in a URL is `404 not_found` — it names nothing — and
a malformed query `400`, where axum's own extractors answered plain text.

**So is a NUL.** Postgres stores no NUL in text, and refuses a statement that
binds one (SQLSTATE `22021`, or `22P05` for a `\u0000` read from JSON as text).
Every text the server binds that could hold one is the caller's — results and
declines refuse it before the database — so that refusal is a `400
bad_request` ("the request holds a character that cannot be stored"), not a
`500`. Until the thirty-third audit's third pass a `?worker=%00` on a job's
public results feed, a NUL in a login name or a reset address, or a key label
was a `500` and an error line anyone could write.

`message` is shown to people. A `500`'s is replaced by "internal error" when it
came from the database; a `503`'s is not, since it says to try again.

**A body that does not parse is answered in the same shape.** It never reaches
a handler, so with axum's own `Json` extractor it was answered by axum: plain
text, and three statuses — `400` malformed, `415` no content type, `422` wrong
shape — that are not in the list above, which neither the frontend's error path
nor MAGPIE's expects. Every JSON route takes its body through
`extract::ApiJson`, whose rejection is this error type: `400 bad_request` naming
what the parser found, or `413 payload_too_large` past the route's limit. The
same extractor parses a body of 256 KiB or more on the blocking pool — a result
may be 64 MiB, and that much parsing on an async worker thread is the stall
described under [Exports](#exports).

Server errors are logged at `error` and everything else at `debug`; the
message a client sees is the same either way, and never includes a database
error or a stack trace — including a driver-level failure that carries no
SQLSTATE (a dropped connection, a protocol error), which is logged in full and
answered with the generic message.

A unique or foreign-key violation that reaches the handler maps to `conflict`
(409), not `internal` (500): "this name is taken" and "something still references
this" are answers the caller can act on, and a 500 invites a retry that will fail
identically. Where a foreign-key failure means the caller named a row that does
not exist -- a rating pool's anchor or new member -- the handler answers
`bad_request` on that field instead, and `not_found` for an unknown pool: "still
referenced" would say the opposite. Anything else from the database is a 500
with a generic message.

#### Pagination

List endpoints take `?page=` (zero-based, default 0) and `?per_page=` (default
**50**, clamped to 1–**500**) and return:

```json
{ "items": [ ], "total": 123, "page": 0, "per_page": 50 }
```

`total` is `-1` where an exact count would cost more than it is worth to the
caller — the per-job result feeds, which are effectively unbounded.

**One endpoint pages by cursor instead**, and it is the same one.
`GET /api/jobs/:id/results` returns `{ items, total: -1, per_page, next_cursor }`
and takes `?cursor=` in place of `?page=`. A job's corpus runs to millions of
rows, and `OFFSET` produces and discards every row before the page asked for, so
page *N* costs *N* pages; a cursor makes every page cost one. The exception is
made here and nowhere else because every other list is bounded by something that
does not grow the way a job's results do. The cursor is opaque — the last row's
sort key, hex-encoded — and one this server did not produce reads as "start at
the beginning" rather than as an error, since a caller cannot repair a token it
cannot read.

A leave-generation job's feed is its per-rack progress, newest generation
first and within one by rack, and its cursor is that pair: the primary key's
order, so each read is a seek. It is read one generation at a time because the
two directions differ. (It once ran furthest-from-target first, which no index
held; see [What these reads cost](#what-these-reads-cost-measured).)

Its key is worth stating, because the obvious one is wrong: `submitted_at`
defaults to `now()`, which is transaction time, so every record of one batch
shares it exactly and it is not a key on its own. The tiebreaker is
`position_analysis_records.id`, or `game_results.task_claim_id` where there is
no serial, and it is in the feed indexes for that reason.

#### Authentication and CSRF

| Surface | Credential |
|---|---|
| Auth, Account, Admin | `birdtest_session` cookie (httpOnly, `SameSite=Strict`, `Secure` when `SECURE_COOKIES`) |
| Worker | `Authorization: Bearer <api-key>` **or** `X-Worker-UUID`, never both |

CSRF is a double-submit check on the cookie-backed surfaces: a `birdtest_csrf`
cookie (readable by JavaScript, 24 random bytes hex) must equal an
`X-CSRF-Token` header. `GET`, `HEAD` and `OPTIONS` skip the check. Both cookies
are set on successful login. Worker endpoints are exempt because neither of
their credentials is something a browser attaches automatically.

Admin routes take an admin-only extractor rather than checking a flag in each
handler, so the authorization check cannot be forgotten in a new route: a
non-admin session gets `403`, and a request with no session — a worker
credential included — `401`.

#### Rate limits

In-memory token buckets, per process, reset on restart.

| Endpoint | Limit | Keyed on |
|---|---|---|
| `POST /api/auth/register` | 10 / hour | Client IP |
| The "that address already has an account" notice registration mails | 5 / hour | The address (`reg-em:`, on the reset limiter): past it the registration answers as usual and sends nothing, so it cannot bury an account holder in notices |
| `POST /api/auth/login` | 10 / minute, and 100 / minute | Client IP; and, separately, the account the name matched (or, for a name that matches none, the name) from anywhere — ten times the address's, so that one address cannot lock an account out |
| `POST /api/auth/reset-password/request` | 5 / hour | Client IP **and**, separately, the address asked for |
| `POST /api/worker/task` | 1 / second, **burst 5** | Per credential: the API key (`k:<the key's SHA-256>`) or the anonymous UUID (`a:<uuid>`). Claims only. A worker runs one task at a time, so this caps a job whose tasks take under a second — a small batch of static games — at about a task a second per worker (KL-93). |
| `POST /api/worker/{result,heartbeat,decline}`, `GET /api/worker/artifact` | 1 / second, **burst 5** | Per credential again, in a second bucket (`…#work`): work in hand never waits behind claims — idle machines sharing a key or a copied `uuid` took every token, and a busy one's heartbeats, sent once and never retried, were refused until its claim lapsed (thirty-first audit). So one credential makes up to two requests a second, five and five in a burst. Per key, not per account: keyed on the account, every machine a contributor ran under it shared one request a second, and six idle machines used it all. (An account-wide bucket beside it, 10 / second, was too tight for the hundred keys an account may hold: fifty idle machines filled it, and heartbeats, which are not retried, lapsed. Key churn is bounded at creation instead, below) |
| `PATCH /api/me/api-keys/:id` resuming a key (`is_active: true`) | 60 / hour, **burst 100** | The account (`key_changes`). Suspending is not limited: an owner shutting keys after a takeover is never held back by a thief who drained the bucket, and a back-and-forth needs both. Only a change writes an audit row |
| `POST /api/me/api-keys` | 10 / hour, **burst 100** | The account. Each key is a worker bucket of its own and revoking one frees a slot under the hundred-key cap, so unmetered churn was unmetered new capacity. The burst is the cap, so a contributor setting up a machine per key is not held back (at ten an hour from the start, fifty machines took five hours); what refills slowly is revoke-and-recreate |
| `POST /api/worker/task` with no identity | 5 / second, **burst 30** | Client IP, shared by every new contributor behind one address until each is issued a UUID |
| Any worker request with an API key or `X-Worker-UUID` that has not resolved in the last ten minutes | 5 / second, **burst 100**, charged **before the lookup**, match or not | Client IP. The identity lookup is a main-pool query; a credential that resolved recently skips this, so a bad neighbour behind a shared address does not lock out working machines (`ratelimit::CredentialGate`). The worker's own bucket (above) is charged before the lookup too, on the credential as presented |
| `POST /api/auth/confirm-email`, `POST /api/auth/reset-password/confirm` | 20 / minute | Client IP. The codes are too long to guess; this bounds cost (unauthenticated writes on the main pool) |
| A reset link's password scorings | 5 / hour | The link (`tok:`, on the reset limiter), from any address, charged once the link reads valid and a scoring turn is held (so a request turned away busy spends none): a link refused a weak password still works, and replayed from many addresses it bought a queue of scorings no per-address limit bounded. Its owner, after five weak tries, waits too |
| `GET /api/jobs/:id/stream` | 2,000 open at once, at most 32 from one address | Open streams, not requests: past either a `503`. The page backs off from 5 s to a minute between attempts |

"Client IP" is the `X-Forwarded-For` entry `TRUSTED_PROXY_HOPS` from the right —
the ALB's or Nginx's view of the caller — or the TCP peer when that is 0. Keying
on the peer behind a proxy would put the whole site in one bucket.

The burst matters: a task costs at least two requests, so a strict one-per-second
limit with no burst would throttle a well-behaved client.

A reset request is checked twice, and both halves are load-bearing. Without a
limit it is an unauthenticated endpoint that sends mail to any address it is
given: a way to probe which addresses have accounts, and a way to bury a known
contributor in reset emails at the operator's expense. Limiting by IP alone
stops neither, because IPs are cheap.

Every key here comes from outside — a worker UUID, a client address, a username
typed at the login form, an address typed into password reset — and a keyed
bucket map keeps one entry per key it has ever seen. That is unbounded memory
growth driven by unauthenticated input rather than by how many contributors
there are, so a background sweep drops buckets that have gone idle (ten
minutes, against buckets that refill in seconds to an hour). Forgetting a full
bucket changes no decision: the next request rebuilds it full.

#### Two connection pools

The process holds two pools against the same database, and which one a read
uses is decided by whether anything *waits on its answer to do work*:

| Pool | Size | Bounds | Used by |
|---|---|---|---|
| main (`AppState.pool`, `db::connect`) | 20 | none | Claims, submissions, heartbeats, declines, the finish check, every admin and account route, the background sweeps, exports and the admin results stream |
| display (`AppState.read_pool`, `db::connect_read`) | 8 | `statement_timeout` 15 s, acquire timeout 5 s | The public pages (`/api/jobs*`, `/api/users`, `/api/workers`, `/api/rating-pools*`), the SSE stream's first payload and every live push |

Every route on the display pool is unauthenticated and unmetered, and several
of its reads grow with a job's history. On one shared pool that made page views
a way to stall the fleet: twenty slow reads at once — enough people with a busy
job's dashboard open, or one caller in a loop — held all twenty connections,
and every claim and submission queued behind them until sqlx's thirty-second
acquire timeout failed it. Nothing on the display pool decides anything (no
statistic is read while dispatching or accepting), so its reads can queue among
themselves, behind a bound, and leave the path workers wait on alone. The three
bounds do different jobs: the size caps how many connections display can hold
at all, the statement timeout caps how long any one read holds one, and the
short acquire timeout turns a saturated pool into a quick `503` rather than a
request parked for half a minute. The admin results stream and the exports stay
on the main pool — they hold a cursor for minutes, which the statement timeout
exists to forbid — and are bounded by the two-stream cap and by being admin
actions instead.

#### Health and startup

`GET /health` returns `200 ok` and is what the container healthcheck and the ALB
use. On startup the process, in order: loads config from the environment
(`.env` locally, task-definition variables in ECS), asks the pinned MAGPIE
(`MAGPIE_BIN`) for its version and builders and fails unless it clears
`MIN_MAGPIE_VERSION`, connects the pool, **runs migrations before binding** so
a container never serves traffic against an out-of-date schema, connects the
display pool, takes its address (so a second process on a taken one exits
before touching the running one's work — though after migrating, which a newer
binary would do to the live schema), fails any input-data import or export left
`running` by a previous process, releases any leave-generation transition one
left open (so the next claim takes it over rather than the half-hour takeover
timeout), starts the five background loops (rating fits, rate-limit sweep,
import expiry, rating-run thinning, leave merges), and only then serves.

On the way out it **shuts down gracefully**: `SIGTERM` (what ECS sends before it
escalates to `SIGKILL` at the stop timeout) and `SIGINT` stop it accepting new
connections and let in-flight requests finish. The dashboards' SSE streams are
ended by the same signal (`state::Shutdown`): a stream is a request that never
finishes, so with one job page open anywhere the wait for "in-flight requests"
could only end at the `SIGKILL`, the whole stop timeout (now 120 seconds)
added to every deployment's gap. The page's `EventSource` reconnects by itself, to the new process. Without that, a deployment drops
whatever is in flight — and a worker that has just uploaded a completed batch
loses it, because the claim it was for is still `claimed` and stays that way
until the heartbeat timeout, so the retry is answered `accepted: false`.

#### Audit actions

Every significant action writes an `audit_log` row inside the same transaction
as the action itself, so an audit failure rolls back what it describes. The
exception is an artifact rebuild, which runs over many transactions and writes
its row *before* it rewrites anything (and a second, with counts, when it
ends), so that one that stops part-way is still on record. An export is
logged in the transaction that records it, before its work runs on a task of
its own; a refused one writes nothing. Claims
and submissions are the deliberate exception: `task_claims` already records who
claimed and completed which task and when, so a row per claim and per submission
duplicated it, cost a write each on the path a worker waits on, and made up most
of the log's growth. Sign-in attempts are the other: their number is the
caller's to choose, and a row each would let anyone with a list of usernames
grow the log at the rate limiter's pace. The limiter refuses a guesser, but
nothing records that one tried — not the log, not the service's own logs at
their deployed level, not the load balancer (KL-90). The credential changes an
account makes to itself — keys, a reset, a confirmation — are logged (they
were not until the audit's pass 22): they are the trail a takeover leaves, and
what RUNBOOK §1 re-applies after a restore undoes them — revocations,
suspensions, resets, confirmations; a key made since the restore point is
recorded by its id only, and its owner makes it again. A row's `created_at` is its
transaction's start: a multi-minute purge's rows sort before actions that
committed while it ran (`restore-job.sh` picks by `max(id)`, assigned at
insert).

| Action | Written by |
|---|---|
| `user.registered` | Registration |
| `task.declined` | A worker declining, with the reason in `reason` |
| `job.created` / `job.activated` / `job.deactivated` / `job.completed` | Admin job lifecycle; an activation or deactivation (an allocation change that switched the job on or off) carries the allocation from what to what in `reason` ("0% -> 40%"); `job.completed` also for a job the server completes (its stopping rule: the match test, its last generation), with no actor and the verdict in `reason` (`player1_better`, `player2_better`, `inconclusive`) — or `reached_target` for a games or pairs job without a test |
| `job.set_aside` | The server switching a job off because three of its tasks in a row hit the time limit with none completed between — `time_limit` declines and overruns alike (`worker::record_time_limit`): no actor, `active` -> `inactive`, and in `reason` the allocation it moved from and why ("50% -> 0%: 3 tasks in a row hit the 1-hour time limit …"), which the job's `set_aside_reason` repeats for its page |
| `settings.changed` | An admin changing a run-time setting (`PUT /api/admin/settings`), from what to what in `reason` ("max_task_seconds 3600 -> 1800"); a change to what it already is writes none |
| `job.allocation_changed` | An active job's allocation changed to another above 0% through `PUT /api/admin/jobs/allocations`, from what to what in `reason` ("20% -> 35%"); a change to or from 0% is its `job.activated` / `job.deactivated` row instead |
| `job.consensus_changed` | An opening-rack job's consensus settings changed, the changes and the racks left unsettled in `reason` ("min 1 -> 2, max 1 -> 3; 4 racks unsettled"); beside it `job.deactivated` (from `completed`) when the change reopened a completed job |
| `job.purged` / `job.purged.census` | Purge |
| `job.deleted` / `job.deleted.census` | Delete |
| `user.deleted` / `user.deleted.census` | Account deletion |
| `job.artifacts_rebuild_started` | An artifact rebuild, before it rewrites anything, with `force` |
| `job.artifacts_rebuilt` | Artifact rebuild, with counts, when it ends |
| `job.export_started` | An admin starting a results export |
| `input_data.import_staged` / `input_data.import_confirmed` / `input_data.import_nothing_new` | Tarball import; the last when every file was already known, so there was nothing to confirm (actor: the admin who started it) |
| `worker.banned` | Ban, with the free-text reason |
| `worker.unbanned` | Lifting a ban, naming the identity rather than the ban row, which is gone |
| `user.signed_out_everywhere` | "Sign out everywhere" on the account page |
| `user.password_reset` / `user.email_confirmed` | A password reset, which ends every session, and an address confirmed |
| `api_key.created` / `api_key.deactivated` / `api_key.reactivated` / `api_key.revoked` | An account's own API keys, by id — not the label, the owner's free text, which would outlive a deletion; a revoked key's row is deleted, so this is the only record it existed. A suspend or resume that changes nothing writes no row, and resuming is limited per account (`key_changes`: a burst of 100, then 60 an hour) — a back-and-forth needs a resume, while suspending and revoking stay free for an owner after a takeover |
| `rating_pool.created` / `rating_pool.member_added` / `rating_pool.member_removed` | Rating pool membership, each of which refits the pool |
| `rating_pool.anchor_changed` | The pool's anchor or anchor rating moved, old and new in `reason` (`anchor=<id>@<rating> -> <id>@<rating>`); the pool refits in the same transaction |
| `rating_pool.deleted` / `rating_pool.deleted.census` | Deleting a rating pool; the census names the pool and counts its members and runs |
| `derived_data.retried` | An admin re-queueing a failed wordmap, rack info table or word info table build |
| `input_data.deleted` | An admin deleting an input data row (and the derived rows built from it) |
| `player_config.deleted` | An admin deleting a player config |

The `.census` rows are the reason the destructive ones are worth having.
`purge_job` and `delete_job` each count what they are about to destroy — tasks,
claims, results, progress and artifact rows — and write that as a single line
into `audit_log.reason` **before** the delete runs, inside the same
transaction. After the delete commits, that row is the only surviving
description of what the job held, and it is what a selective restore is scoped
against. `delete_user` anonymizes rather than deletes, so its census says
which counts go (API keys, confirmation codes, reset tokens — and, uncounted,
the name, address and password) and which stay (claims, and of them the
completed ones). Deleting a rating pool writes one too — its name and how many
members and runs went with it — which is what RUNBOOK §0 finds and its
§2.6 ("A deleted rating pool") works from. `audit_log` deliberately has no foreign keys: one to `jobs` or
`users` would either block those deletions outright — every job has a
`job.created` row, every user a `user.registered` one — or rewrite the history the
log exists to keep.

---

### Worker API

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/worker/client-version` | The oldest MAGPIE a client may contribute with (`MIN_MAGPIE_VERSION`) and where to get it, for the admin new-job form and for humans; no worker calls it (MAGPIE learns the floor from a claim's shutdown directive). Not a self-update: MAGPIE is a compiled binary and the client cannot replace itself. |
| `POST` | `/api/worker/task` | Send a task claim. The body is **required** and carries `magpie_version`, `board_dim` and `rack_size` (each required) and `unsupported_jobs` (optional, empty when omitted). Returns a task assignment, a `shutdown` directive, or `204`. |
| `POST` | `/api/worker/decline` | "I claimed this and cannot do it" -- or, `time_limit`, "I stopped it at the limit". Releases the claim immediately rather than waiting out the heartbeat timeout, and records the gap. |
| `POST` | `/api/worker/heartbeat` | Keep-alive ping for a claimed task. Updates `last_heartbeat_at`. |
| `POST` | `/api/worker/result` | Submit the result for a claimed task. Requires the claim token. |
| `GET` | `/api/worker/artifact?key=<artifact-key>` | Download a stored artifact — in v1, a generation's combined KLV for a leave-generation task. Proxied through the server so contributors never need AWS credentials; only keys the server itself minted are reachable. |

The full request and response shapes are in [The Worker API Contract](#the-worker-api-contract).

### Auth API

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/auth/register` | Create a new user account. Sends a confirmation email. |
| `POST` | `/api/auth/login` | Create a session. Returns a Paseto token in an httpOnly cookie. |
| `POST` | `/api/auth/logout` | End the current session. |
| `POST` | `/api/auth/sign-out-everywhere` | Revoke every session of the account, the caller's included, by bumping `users.session_generation`. |
| `POST` | `/api/auth/confirm-email` | Confirm email address using the code from the confirmation email. |
| `POST` | `/api/auth/reset-password/request` | Send a password reset email. |
| `POST` | `/api/auth/reset-password/confirm` | Apply a password reset using the token from the reset email. |

### Account API

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/me` | `{ id, username, email, is_admin, tasks_completed }`. |
| `GET` | `/api/me/api-keys` | List API keys (label, active status, created/last-used timestamps; hashes are never returned). |
| `POST` | `/api/me/api-keys` | Generate a new API key. Returns the raw key exactly once. Rejected if the user already has 100 keys. |
| `PATCH` | `/api/me/api-keys/:id` | Set a key's active status (`{ "is_active": bool }`). |
| `DELETE` | `/api/me/api-keys/:id` | Permanently revoke an API key. |

### Admin API

All Admin API endpoints require the requesting user to have `is_admin = TRUE`. A non-admin session is rejected with `403 Forbidden`; a request with no session — worker credentials or not — gets `401`.

Every action on a job below — allocations, complete, consensus, purge, delete, export, merge-progress and rebuild-artifacts — answers `409` while a purge, delete or consensus change of that job is running ([Editing the consensus](#opening-rack-consensus)): each holds the job off the fleet, and an action that waited on it would hold a pool connection until it finished.

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/admin/player-configs` | List all player configurations, as stored (file ids, creator). The public `GET /api/player-configs` and `/:id` serve every setting with files by name, and not who made it. |
| `POST` | `/api/admin/player-configs` | Create a new player configuration, named as a job is (at most 100 characters, one line, stored trimmed). Refuses what MAGPIE would refuse or cut short: more than 25 plies (`MAX_PLIES`), more than 10 recorded plies (what a captured position keeps), more than 200,000 plays generated (MAGPIE allocates every one up front, and a static player ranking every opening play needs up to some 64,000) or 32,767 recorded (a stored rank is a `SMALLINT`), a margin that is negative, not finite or past MAGPIE's largest equity (2,147,483.645), as well as non-positive counts. |
| `GET` | `/api/admin/player-configs/:id` | Get a single player configuration. |
| `DELETE` | `/api/admin/player-configs/:id` | Delete a player configuration. Rejected if any job, rating pool, rating history or clone references it. |
| `POST` | `/api/admin/jobs` | Create a new job, with an optional `name` (at most 100 characters, one line; the form asks for it) shown first wherever jobs are listed and as the job page's title. A games or pairs job names its players as `player_config_ids`: one is self-play, n ≥ 2 (at most 12, none twice) a round robin of C(n, 2) jobs named "{name}: A vs B", created all or none. A games or pairs job may state `threading_mode`, `igp` (the default) or `pgp`, which every task's request carries (see [Task Request Types](#task-request-types)); anything else is a `400` on the field. Answers `201` with `{ "jobs": [...] }`, each created job as stored. Created inactive at 0% — `PUT /api/admin/jobs/allocations` sets its allocation and starts dispatching work. Refuses a board layout that is not 15×15 (the fleet's MAGPIE builds are 15×15 -- a claim from any other is answered `unsupported_build` -- so every worker would fail every task) and a match-test confidence outside (50, 100) (at 100% the interval never closes). |
| `PUT` | `/api/admin/jobs/allocations` | Set several jobs' allocations at once. Body: `{ "allocations": [{ "job_id": uuid, "allocation": int }] }`. Checked as a whole -- the active jobs must sum to at most 100% as the request leaves them -- under the activation lock, every named row locked in id order first. The only way an admin activates or deactivates a job: above 0% a job is active (activated if it was not), and a job the server set aside for its time-limit declines starts a new run of them, its reason cleared; 0% leaves it inactive at 0% (deactivated if it was active). A completed job (even at 0%), a job named twice or an allocation outside 0–100 refuses the whole request. Audited once per job that changes, from what to what in `reason`: `job.activated` / `job.deactivated` for a job switched on or off, `job.allocation_changed` for an active job's new share. Answers `{ "jobs": [...] }`, the named jobs as they now stand. The `/admin/allocation` page sends it. |
| `PATCH` | `/api/admin/jobs/:id/consensus` | Change an opening-rack job's consensus settings. Body: any of `{ "min_results_per_rack", "max_results_per_rack", "consensus_pct" }`; only those sent change. Refuses (`400`) what creation refuses, and any other job type. Restates every rack, then reopens a completed job left with unsettled racks (inactive at 0%, for the admin to give an allocation) and completes an active one left with none. Answers `{ job, config, unsettled_racks, reopened }`. `409` while a purge, delete or another consensus change of the job is running. Audited as `job.consensus_changed`. See [Editing the consensus](#opening-rack-consensus). |
| `POST` | `/api/admin/jobs/:id/complete` | Force-complete a job immediately, regardless of task progress, at 0% as every completed job is. Refused (`409`) for a job that is already completed. |
| `POST` | `/api/admin/jobs/:id/purge` | Delete every claim, result, leave-gen progress and staged-result row, selection cursor, artifact row and task for a job, reset its dispatch counter and baseline and leave it inactive at 0% whatever it was (an allocation change rejoins it at parity), then re-seed its initial state. Ratings are untouched: they belong to rating pools, and the sweep refits a pool whose evidence changed. Returns `{ tasks_reset }`. Writes a census of what it destroyed to the audit log first. `409` while a purge, delete or consensus change of the job is already running: each runs to completion on a task of its own, so a second click stacked a second behind the first's locks. |
| `DELETE` | `/api/admin/jobs/:id` | Delete a job and all its tasks. `409` while a purge, delete or consensus change of it is running, as above. |
| `DELETE` | `/api/admin/users/:id` | Delete a user account: anonymize it in place, keeping its claims and records so no donated compute is lost (see Admin API semantics). |
| `POST` | `/api/admin/workers/ban` | Ban a worker by user ID or anonymous UUID. One ban per identity: a second is `409`, so that unban means what it says. The reason is at most 1,000 characters and holds no NUL (`400`). |
| `GET` | `/api/admin/workers/bans` | Every ban in force, newest first, with the id lifting it takes. (Nothing listed them before the thirteenth audit; a mistaken ban needed SQL.) |
| `DELETE` | `/api/admin/workers/ban/:id` | Remove a ban, and with it the identity's only ban. |
| `GET` | `/api/admin/audit-log` | Query the audit log with filtering and pagination. |
| `GET` | `/api/admin/input-data` | List known input data rows — path, role, name, digest, tarball date. |
| `DELETE` | `/api/admin/input-data/:id` | Delete an input data row. A row referenced by a job, player config or rating pool cannot be deleted; the foreign key is the safety mechanism and the error reads "this file is pinned by N jobs, player configs or rating pools". |
| `POST` | `/api/admin/input-data/imports` | Start a tarball import. Returns `202` and an import id immediately; the fetch and diff run as a background task. |
| `GET` | `/api/admin/input-data/imports/:id` | Poll an import: progress while running, the staged diff once staged (or `nothing_new`, every file already known), or the failure reason. |
| `POST` | `/api/admin/input-data/imports/:id/confirm` | Insert the staged new and changed (collision) rows, in one transaction. |
| `GET` | `/api/admin/jobs/:id/data-gaps` | What workers reported they were missing for this job, from `worker_data_gaps`. |
| `GET` | `/api/admin/jobs/:id/derived-data` | The wordmaps, rack info tables and word info tables this job needs and where each build stands: `[{ role, name, state, error, attempts }]`, `state` one of `pending`, `building`, `built`, `failed` (a file not yet requested under this binary's builder is `pending`). A job is not dispatched until every one is `built`, so this is what an active job nothing claims from is waiting for; the admin job page lists it. `404` for no such job. |
| `GET` | `/api/admin/jobs/:id/results/stream` | Newline-delimited JSON (`application/x-ndjson`) of every record for the job, streamed straight from a database cursor so a download never buffers a whole job in memory. The source table follows the job type: position analyses (each with its ranked moves and plies nested), game results, or leave-rack progress — the export's own queries. `?positions=true` (games and game-pairs jobs) streams the positions the job captured instead of its result rows. At most two run at once (a third gets `429`); a completed job with a ready export gets a `303` to it instead. A stream is complete exactly when it ends cleanly: what can fail before the first row (a connection, a leave job's settle) is a status, and a query that fails part-way cuts the body off with an error rather than ending it, which a client over HTTP/1.1 or later reports as a failed transfer (HTTP/1.0 has no chunks, and a cut reads as an end there). |
| `POST` | `/api/admin/jobs/:id/export` | Build a job's results into one gzipped NDJSON object in the artifact store: a completed job's final corpus, or a snapshot of one still running. `202` with an id; the work runs on a background task. `409` while an export of the job is running, or for a completed job whose last claims are still in flight. |
| `GET` | `/api/admin/jobs/:id/export` | The newest export for the job — `is_final`, or a snapshot as of `snapshot_at` — with a presigned `download_url` once it is ready, and, for a games or game-pairs job that captured positions, a `positions_download_url` for the second object holding them. |
| `GET` | `/api/admin/workers` | The contributor list with anonymous workers' real UUIDs, which banning one needs; the public list carries pseudonyms only. |
| `GET` | `/api/admin/derived-data` | Every wordmap, rack info table and word info table the server has been asked to build: state, builder, hash, attempts and the last error. A job whose files are not `built` is not dispatched, and this is where that wait — or the failure behind it — is visible. |
| `POST` | `/api/admin/derived-data/retry` | Put one `failed` build back in the queue: `{ role, name, builder, kwg_id, klv_id, letterdist_id }`, as `GET /api/admin/derived-data` lists them (`klv_id` null for a wordmap); without the builder, `kwg_id` or `letterdist_id` it is a `400`, since rows can share a role and name, and a row that matches no failed build of this version's builders — a rack info table sent without its `klv_id` among them — is a `404`. Explicit rather than automatic: a failed attempt is tried again after 5 and then 15 minutes, which a passing outage survives, so a build that has failed three times failed for a reason a fourth attempt does not change — a missing or damaged input, a broken binary. |
| `GET` | `/api/admin/fleet` | What the field is running, from `task_claims.magpie_version`: workers and claims per version, over the claims completed in the last seven days and those made in that week and still open — an open claim older than a week, which lazy reclamation can leave behind long after its worker went (KL-1), is not counted (KL-32). |
| `GET` | `/api/admin/settings` | The settings an admin changes at run time: `{ max_task_seconds, updated_by, updated_at }`, `updated_by` the admin's name (null until anyone has changed them). |
| `PUT` | `/api/admin/settings` | Change them: `{ "max_task_seconds": int }`, 600 to 86,400 (ten minutes to a day — a first claim may build a rack info table that cannot be stopped; anything else is a `400` on the field, and the column's CHECK refuses it too). The limit applies to the claims made after the change; every claim keeps the deadline it was given. Audited as `settings.changed`, from what to what; a change to what it already is writes nothing. Answers the settings as they now stand. The `/admin/settings` page sends it. |
| `GET` | `/api/admin/backups` | Recent backup runs and how stale the newest successful one is. Read-only: backups are performed by a scheduled task, never by the server — see [Backups and Restore](#backups-and-restore). |
| `POST` | `/api/admin/rating-pools` | Create a rating pool: name (as a job's: at most 100 characters, one line, stored trimmed), scope, and the anchor config that fixes the scale. The anchor joins as a member automatically. |
| `POST` | `/api/admin/rating-pools/:id/members` | Add a player config to the pool and refit it. Returns the new run id; `run_id` is `null` for a config that is already a member (a second click, or the anchor), which is neither logged nor refitted. |
| `DELETE` | `/api/admin/rating-pools/:id/members/:config_id` | Remove a config and refit. Refused for the pool's anchor, which every other rating is measured against (move the anchor first); `404` for a config not in the pool, which is neither logged nor refitted. |
| `PATCH` | `/api/admin/rating-pools/:id` | Move the anchor (`anchor_player_config_id`, added as a member if it is not one) and/or its `anchor_rating`, and refit in the same transaction (trigger `anchor`). Logged as `rating_pool.anchor_changed` with old → new in the reason. `run_id` is `null` when nothing changed. |
| `DELETE` | `/api/admin/rating-pools/:id` | Delete a pool with its members, runs, ratings and residuals, after logging its census. The games stay with their jobs. `204`; `404` for a pool that does not exist. |
| `POST` | `/api/admin/rating-pools/:id/recompute` | Force a refit without changing membership. |
| `POST` | `/api/admin/jobs/:id/merge-progress` | Leave-generation jobs only. Fold the job's staged results into its per-rack totals now rather than at the next half-hourly merge, waiting for a merge already running. Returns `{ folds_merged, racks_updated }`. Nothing needs it — claims ask for a merge near a generation's end and a transition drains before it reads — so it is for an admin who wants the page's rack figures current, and for the end-to-end suite. |
| `POST` | `/api/admin/jobs/:id/rebuild-artifacts` | Leave-generation jobs only. Recompute each generation's KLV from `leave_rack_progress` and report whether the stored object is still present and still matches its recorded hash. Rewrites only missing objects unless `?force=true`. |

#### Admin API semantics

The table above says what each route is; these are the rules a reimplementation
would otherwise have to guess.

**Creating a player config.** Enumerated fields are validated against their
allowed values rather than trusted: `recorder_type` is `best` | `equity` | `all`,
`sort_strategy` is `equity` | `score`, `threshold` is `none` | `gk16`,
`sampling_rule` is `round_robin` | `top_two_ids`. Each of `kwg_id`, `klv_id` and
`winpct_id` is checked to be a row of the **matching role** — every one of those
foreign keys points at the same table, so the database cannot express it and it
is validated wherever a role column is written. Configs are immutable: there is
no update endpoint. Deletion is refused with `409` while any job, rating pool,
rating history or clone references the config. Numbers are range-checked — play,
iteration and recorded counts at least 1, `num_plays` at most 200,000 and
`num_plays_recorded` at most 32,767, plies at most 25 (10 recorded),
`stopping_pct` strictly between 0 and 100, margins and weights finite and
non-negative and margins at most MAGPIE's largest equity, 2,147,483.645 — and a config with any
simulation setting must simulate at least one ply, because MAGPIE decides whether
a player simulates on plies alone: a "simmer" without plies would play
statically on every worker. A simmer must also set `max_iterations`, and
`time_limit_secs` of 0: MAGPIE applies a time limit only above 0 (a null means
its 60-second default), and a limit makes how far a simulation gets depend on the
contributor's hardware, so the iteration budget bounds it instead. And it must
generate at least two candidates (`num_plays`): autoplay's move list holds
`num_plays` plays, and with one MAGPIE plays it without simulating, every turn
-- the other way to a "simmer" that plays statically (thirty-third audit, pass 3). `use_rit` is
accepted: a rack info table carries precomputed leave values that move
generation uses in place of the leaves the config pins, which is why it was
refused until the server could build the table for this exact (lexicon, leaves)
pair and pin its hash. A job whose players ask for one is not dispatched until
it is built.

Whatever the body leaves out is filled from MAGPIE's defaults
(`backend/src/magpie_defaults.rs`) before the row is written, so the stored config
is exactly what every request built from it states: `sort_strategy` (`equity`),
`num_plies` (0, static), `num_plays` (100), `num_plies_recorded` (2),
`movegen_margin` (5), `use_wordmap` and `use_rit` (both true -- the two
defaults that are birdtest's rather than MAGPIE's: each is a large speedup,
workers build their own on demand, and the admin form has both checked; a rack
info table costs a contributor about 1.9 GB of disk and of memory), and for a simmer every simulation
setting. A static player may not set a simulation setting at all — nothing would
read it — and a CHECK on the table holds the two sets apart. `num_plays` is not a
simulation setting: an opening-rack analysis sizes its move list from it,
simulating or not. The values are written rather than applied at dispatch, so a
later change to a default reaches new configs only, and an existing config keeps
playing, and being rated, as it did.

A clone onto newer data is not a separate endpoint — it is an ordinary create
that sets `cloned_from_id`. The convention for the name (`base@tarball_date`) is
the caller's to follow.

**Creating a job.** Always created `inactive`, with no allocation. Defaults, when
the body omits them:

| Field | Default |
|---|---|
| `min_magpie_version` | the server-wide floor |
| `games_per_batch` | 2 — and it must be even (see "How the interval is computed") |
| `pairs_per_batch` | 1 |
| `racks_per_batch` | 500 |
| `rack_size` | 7 |
| `test_enabled` | **false** — the job plays its `max_games` / `max_pairs` and stops; `min_*` and `confidence_pct` are refused without it |
| `min_games` / `min_pairs` | none: required when `test_enabled` is true |
| `confidence_pct` | 95 |
| `min_results_per_rack` / `max_results_per_rack` / `consensus_pct` | 1 / 1 / 100 |
| `target_rack_counts` (leave generation) | none: required, one occurrence target per generation, 1 to 100 of them, each 1 to 1,000,000 |
| `capture_positions` | false |
| `bingo_bonus` | 50, MAGPIE's default (`magpie_defaults::BINGO_BONUS`); 0 to 500 |
| `sim_cutoff` | 0.005, MAGPIE's default (`magpie_defaults::SIM_CUTOFF`); a leave-generation job may not set it |

A job's run-wide MAGPIE settings, `bingo_bonus` and `sim_cutoff`, are optional
fields of the body: what the body leaves out is written onto the job from
MAGPIE's defaults at creation, and either way the value is stated on every
request, for the reason player configs state theirs. Both change results.
`bingo_bonus` must be between 0 and 500 (the column's CHECK): a negative bonus is
a typo, not a variant anyone plays, and the plausibility rules' score bounds are
absolute and assume an ordinary bonus -- real variants use 0 to 50 -- so a bonus
in the thousands made every honest batch implausible and wedged the job (the
thirty-third audit's pass 2); `sim_cutoff` must be finite and between 0
and 100 (MAGPIE's `-cutoff` range and the column's CHECK), and is refused on a
leave-generation job, which never simulates (the column still holds the
default; its requests never carry it). The new-job form has an input for each
(the cutoff only where the job can simulate). Rating
pools do not scope by them (KL-75).

Beyond role matching, creation enforces seven rules the schema cannot express:

- **Settings a worker can run and a test can evaluate.** `variant` is `classic` or `wordsmog`; batch sizes at least 1 (`racks_per_batch`
  at most 10,000; `games_per_batch` at most 10,000 games, 1,000 when the job
  captures positions, and `pairs_per_batch` half of that); `rack_size` 1–7; `max_*` at least 1;
  with `test_enabled`, `min_*` given, at least 1 and at most `max_*`, and
  `confidence_pct` strictly between 50 and 100 (at 100 the interval's
  logarithm of 1 − confidence is infinite and it never closes), and without it
  neither `min_*` nor `confidence_pct`; an opening-rack job's consensus as
  `consensus_problems` checks it, for creation and an edit alike (a share
  above 50 and at most 100, a fewest of 1–100, a most from the fewest to 100,
  and, by the player check, a most above 1 only for a simmer); `min_magpie_version`, when given, a
  version (`major.minor[.patch]`, digits only — read loosely, a typo was 0.0.0,
  the most permissive floor). Every violation is reported at once as
  a field error. A zero batch would make every claim regenerate the seed the last
  one took and retry forever.

- **A board MAGPIE loads.** The layout is checked as MAGPIE's loader checks it,
  never accepting one it refuses (and stricter only on its parser's quirks):
  a start square inside the board, then exactly 15 rows of 15 known bonus
  squares, since every MAGPIE build the fleet runs has `BOARD_DIM` 15 (a claim
  from any other is answered `unsupported_build`). A 21×21
  layout ships in the data release beside the super-board lexica, and a job on
  it failed every task on every worker (thirty-second audit).

- **Cross-player compatibility**, ported from MAGPIE's own name-prefix rules.
  Both players' lexicons must be compatible with each other and each with its own
  leaves, and both with the job's single letter distribution. birdtest must not
  be able to build a job MAGPIE would refuse to load.
- **Shared-option agreement.** `win_pct_model` is sent per player but is really
  one MAGPIE setting for the whole run, so two *different* configs must agree on
  it where both state one, since a static player has none and
  static-versus-simmer is the mix a games job most often wants. Skipped when
  both slots name the same config, which is a legal and useful degenerate case.
  `movegen_margin` is not compared: autoplay generates every move with a margin
  of 0 and never reads it (until the thirty-third audit's pass 2, two margins
  were refused here).
- **A recorder that can rank, for opening racks.** A static player with
  `recorder_type = 'best'` and `num_plays_recorded` above 1 is refused: `best`
  records one move, so the job would store one move per rack while claiming to
  store ten. A simmer ranks every play up to `num_plays`, whatever its recorder.
  Any player with `num_plays` below `num_plays_recorded` is refused too. See
  [How much of an analysis is kept](#how-much-of-an-analysis-is-kept).
- **A capture job's simmers consider at least what is captured.** With
  `capture_positions` on, MAGPIE raises each simming player's `num_plays` to
  player 1's `num_plays_recorded`, so a smaller `num_plays` would make turning
  capture on change the games. Refused, naming the player. Both players must
  also agree on `num_plays_recorded` and `num_plies_recorded`: MAGPIE keeps
  player 1's for the whole run, so player 2's would be shown on the job page and
  never applied. Refused on `capture_positions`, giving both pairs.
- **A leave job's player plays statically on equity, with no rack info table.**
  A generation's leave values are the mean equity of the racks the bot drew, as
  MAGPIE's `leavegen` computes them: a simmer ranks on something else and plays
  orders of magnitude slower, a score sort ignores the very leaves each
  generation feeds back, and a rack info table caches the values of one KLV
  where every generation plays a new one. `num_plies > 0`, a `sort_strategy`
  other than `equity` and `use_rit` are each refused, naming the player. Its
  lexicon must be a `kwg` the job's distribution can spell; its leaves are not
  checked, because they are never loaded — every generation plays the server's
  KLV — and they, with any win% model, are left out of the job's
  `expected_data`, so a worker is not turned away for lacking files no task
  reads.

**Round robins.** A games or game-pairs request names its players as
`player_config_ids`, a list, rather than a player 1 and a player 2. One config
is a self-play job, under the name as given. **n ≥ 2 configs are a round
robin**: C(n, 2) jobs, every pairing once, each seated in the order the configs
were listed (player 1 the earlier) and named "{name}: A vs B" — or "A vs B" with
no name — after the configs' names; two configs are the one job between them,
named the same way. The seat matters little (a pair swaps seats within itself,
and a games batch is even, so each player moves first in half of every task's
games), but a fixed rule keeps "A vs B" A's player-1 side wherever it is shown.
At most twelve configs (66 jobs), none named twice — a config twice would pair
with itself mid-round-robin, and self-play is asked for by naming it alone —
each refused on `player_config_ids`. **All or nothing:** every pairing is
checked first (the win% model, the files MAGPIE loads by name, the capture
settings: everything above that concerns two players), and a refusal names its
pairing ("sim-a vs sim-other: player configs disagree on the win% model"); then
every job is inserted in one transaction, so a check only the insert can make
(two files under one name, a wordmap for too many blanks) refuses the lot. A
name that its pairing would push past 100 characters is refused on `name`.
Every job is created inactive at 0%: the form then goes to `/admin/allocation`
with the new jobs marked and listed first, where they are started, and their
rows are stamped `clock_timestamp()` so lists ordered by creation show the
pairings in order.

The response is `{ jobs }`, every job the request made as stored: one, or one
per pairing. Creation writes no rows up front for any job type: no tasks, and
no leave-generation rack universe, which the first claim seeds as it does
every generation's.

Job creation also writes the zeroed KLV that generation 1 starts from (stored as
generation 0), **after** the transaction
commits rather than inside it: it is a multi-megabyte build and an object-store
write, and holding a transaction open across it would be wrong.

**Setting allocations** (`PUT /api/admin/jobs/allocations`, the only way a
job is activated or deactivated) requires that the active jobs sum to at most
100% as the request leaves them — checked here rather than as a database
constraint, because the intermediate states an admin passes through while
rebalancing would violate a constraint even when the end state is fine. The
error names the total and what the jobs not named already hold. An allocation
of 0 leaves the job inactive at 0%: offered to nobody until it is raised.
Every change above 0% also resets the job's `claims_baseline`, so it joins
level with the lowest among the jobs being served rather than with a lifetime
deficit to work off at the others' expense — and a changed share takes effect
from that moment rather than being applied retroactively to every claim the
job ever issued. A completed job cannot be given an allocation. Allocation
changes are serialized with an advisory lock, so two at once cannot jointly
exceed 100%. Activating a leave-generation job whose generation-0 KLV was never
written — creation writes it after committing, so an object-store failure there
leaves the job without one — builds it first, and activating any job requests
its derived files under the deployment's builder, which may have moved since
creation.

**Deleting a user** anonymizes the account rather than removing it. Personal
data and credentials go: the username becomes `deleted-<id>`, the email
`<random uuid>@deleted.invalid` (random, not the id: ids are public and the
address is unique, so a registered `<id>@deleted.invalid` made the account
impossible to delete), the password hash an unusable value, `is_admin` false,
API keys, confirmation codes and reset tokens are deleted, `session_generation`
is incremented so every session ends, and `deleted_at` is set. Login, password
reset and `CurrentUser` all refuse a deleted account, and `/api/users` omits it.
Contributions stay: the account's claims and results are kept under the
tombstone and **no counter is rolled back**, so no donated compute is lost —
including leave-generation occurrences that could not have been subtracted
anyway. Open claims are left to time out; nothing can submit for them once the
keys are gone. `jobs.created_by`, `player_configs.created_by` and
`worker_bans.banned_by` keep pointing at the tombstone, and `audit_log` records
the census taken before the change. Deleting an already-deleted account is a
404, and an admin cannot delete their own account. What deletion does not
reach: the nightly dumps taken before it keep the account as it was for as
long as they are kept (`backup_retention_days`, 365 by default, and up to 90
days more as a noncurrent version, in the replica too; "Backups and
Restore"), and a restore to a
point before it brings the account back — RUNBOOK §1 deletes again, as the
admin route does, the accounts deleted since the restore point (from the
damaged instance's `user.deleted` rows, reviewed first; the audit's passes 22
and 23).

**Rebuilding artifacts** recomputes each generation's KLV from
`leave_rack_progress`, compares against the recorded digest, and reports per
generation whether the object is present, whether the hash matches, and whether
anything was rewritten. A **missing** object is rebuilt; one that is present but
*differs* is left alone unless `?force=true`, because a mismatch is equally
consistent with a corrupted object and with a deliberate change to the KLV
builder, and overwriting destroys the only copy of whichever it was. Generation 0
is rebuilt as a zeroed KLV rather than from progress rows, which for generation 0
do not exist.

### Public API

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/jobs` | List jobs with status and summary stats. Paginated; `?status=active` (or `inactive`, `completed`) lists only those, with a matching total. |
| `GET` | `/api/jobs/:id/config` | Everything the job runs with, public: the job's settings (variant, letter distribution and board by name, bingo bonus, sim cutoff, oldest MAGPIE), its type's (a games or pairs job's batch, whether it runs a match test, its minimum, cap and confidence; an opening-rack or leave-generation job's own), and every setting of each player config, with its files by name and its id. No creator, no user ids. |
| `GET` | `/api/player-configs` | Every player config, newest first, public: every setting with files by name, the config it was cloned from and when it was made. No creator. |
| `GET` | `/api/player-configs/:id` | One player config, in the same shape. |
| `GET` | `/api/jobs/:id` | Job detail, configuration, and aggregate statistics; for a completed job, how it was completed (`completion`: when, whether an admin forced it, and the server's reason — the match test's verdict (`player1_better`, `player2_better`, `inconclusive`), `reached_target` for a games or pairs job without a test, `last generation built`, or none when an opening-rack job's racks ran out). |
| `GET` | `/api/jobs/:id/results` | Task records for a job, paginated by cursor (`?cursor=`; see [Pagination](#pagination)). `?worker=` filters to one contributor by username or anonymous pseudonym (`anon_id`), resolved to an identity before the job is read; a name that is nobody's is an empty page. `?rack=` is opening-rack jobs only and switches to a single-rack lookup, in one page: every analysis of the rack numbered, each ranked move with its win percentage (`null` for a static analysis) and its first two plies' statistics (`plies`, `[]` for a static one). The analyses' lists share 32,767 moves (what one analysis can record) between them, each cut to its best `32,767 / analyses`: a rack analysed once is listed whole, and a consensus job's hundred analyses of 32,767 moves are not 3.3 million rows from a public route. |
| `GET` | `/api/jobs/:id/rack-samples` | Up to `?n=` (10 unless given, at most 50) distinct racks an opening-rack job has analysed, drawn at random, public: what the job page offers as "Analysed racks to try". A job of at most 1,000 analyses is read whole and sampled exactly; a larger one is probed — a rack drawn uniformly from the job's rack space, then the first analysed rack at or after it in `position_analysis_records_job_rack_idx (job_id, rack)`, wrapping — four probes a rack asked for, duplicates dropped, never `ORDER BY random()`. Another job type is a `400`. |
| `GET` | `/api/jobs/:id/positions` | **Signed in.** A games or game-pairs job's captured positions where the player to move held `?rack=` (required), newest first, at most 20 a page by cursor, each with its CGP, game, turn, rack, previous move and ranked moves (each with its win percentage and at most its first two plies' statistics), and its `inference` (`null` when it has none). The rack may be typed in any case and order, a multi-letter tile bracketed or not; it is spelt as MAGPIE spells one (the job's machine-letter order, blanks last) before the lookup, so a rack with a tile the job's distribution lacks finds nothing. No rack, or another job type, is a `400`. |
| `GET` | `/api/jobs/:id/positions/random` | **Signed in.** One of a games or game-pairs job's captured positions at random, in the same shape, or `null` before it has any. Drawn through two index probes — a random seed's next task (`tasks_seed_unique_idx`), then a random turn of that task (`position_analysis_records_in_game_idx`) — never `ORDER BY random()` over millions of rows; a task with no positions yet is passed over, and after eight such draws the job's newest position is taken. Another job type is a `400`. |
| `GET` | `/api/jobs/:id/board` | What a position is drawn on, public: the job's layout parsed (`start` as `[row, column]`, `squares` row by row — `normal`, `double_letter` … `quadruple_word`, `brick`) and every letter of its distribution in machine-letter order with its blank's spelling and its score. Read from the bytes the job pins, by the same parsers job creation checks them with. |
| `GET` | `/api/jobs/:id/stream` | SSE stream of live stat updates for a job. Pushes an event after accepted results, coalesced to at most one per `JOB_STATS_CACHE_SECONDS` (an admin's change, a completion or a generation closing at once). |
| `GET` | `/api/users` | List all registered user accounts with contribution stats. Paginated. |
| `GET` | `/api/workers` | Contributor stats for all workers (anonymous and authenticated) — movegens, compute time, tasks and the last result — paginated, ranked by movegens or by `?sort=movegens\|compute\|tasks`. |
| `GET` | `/api/workers/movegens` | The site's work by job type, `{opening_rack, games, game_pairs, leave_generation}`, each `{movegens, compute_seconds, tasks}`, every type present: the jobs' running totals (`jobs.movegens`, `compute_ms`, `tasks_completed`) summed, which add up to the contributor list's columns. |
| `GET` | `/api/workers/user/:id/movegens` | One contributing account's work by job type, same shape, from its claims; a deleted account under its tombstone too. `404` for an account that is not on the contributor list. |
| `GET` | `/api/workers/anon/:anon_id/movegens` | The same for a contributing anonymous worker, by its pseudonym (never its UUID). `404` for a pseudonym of nobody on the list. |
| `GET` | `/api/rating-pools` | Rating pools with their conditions, member counts and last fit time. |
| `GET` | `/api/rating-pools/:id` | One pool: its members now (`members`: config id and name, the anchor among them) and its latest fit, with run provenance, each rated config's rating with uncertainty, and the cross table (`head_to_heads`: every head-to-head from both sides, each with its pairs, win score, standard error, average spread and the score the ratings predict). The two sets can differ: a member added since the fit (or whose refit failed) has no rating yet, and one removed since is still rated. |
| `GET` | `/api/rating-pools/:id/history` | Stored runs' ratings, oldest first, thinned to at most 500 runs evenly spaced over the pool's history, for the six current members rated highest in the newest run. No page draws it now. |

**Rack lookup** canonicalizes the query before matching: uppercased, whitespace
trimmed, letters sorted. A rack is a multiset of tiles, so `AEINRST` and
`TSRNIEA` are the same rack and a user typing either should find it. It matches
only opening-rack records (`game_index IS NULL`), so an incidentally-captured
in-game position with the same rack does not surface as an opening-rack analysis.
The full ranked move list is returned in one page rather than paginated. A rack
an opening-rack job analysed more than once, to reach a consensus, has one
record per analysis, and the lists come back one record after another, each in
rank order, rather than interleaved by rank.

---

## Frontend Routes

SvelteKit uses file-based routing under `frontend/src/routes/`. Each directory with a `+page.svelte` is a page. Layout files (`+layout.svelte`) apply to all routes nested beneath them.

### Public Routes

| Route | Page |
|---|---|
| `/` | Landing page — brief description of birdtest, links to the job list and the worker setup guide. |
| `/jobs` | Job list — all jobs with type, status, allocation, and completion counter. Loaded on visit rather than live: there is no job-list stream, only a per-job one. |
| `/jobs/[id]` | Job detail — four headline cards (allocation, tasks completed, active contributors -- the identities holding a live claim, open and heartbeated within the heartbeat timeout (`JobStats.active_contributors`) -- and estimated time left; the admin page has the same), job-type-specific stats and per-worker contribution table with the job's movegens under it, then the settings cards last. Live-updated via SSE. The status card says nothing beside an active job's badge; an inactive job's says "Paused: …" (and where its significance test stands), a completed job's "Finished …: …" and why. Beside the lexicon and variant, how each player searches ("4-ply sim, 1,000 iterations vs static, by equity"); a Job settings card — the job's settings and its type's, every row, with no toggle, the significance test one row ("no" or "yes (95%)") — then a Player settings card, the players side by side (`PlayerSettingsTable`: one column per player, each name linking to its config), showing two players only the settings they differ in ("These players' settings are identical." when none) and one player its key rows (Lexicon, Leaves, Sorted By, Move Recorder, Moves Generated, Plies, Uses Inference, Uses Preendgame, Uses Endgame), with an "All settings" toggle for every row, and a JSON download (`GET /api/jobs/:id/config`). Which player rows are key is `jobSettings.ts`'s (`PLAYER_ROWS`, `keySettings`). The admin job page has the same cards. |
| `/users` | Registered user list — all user accounts with contribution stats. |
| `/workers` | **Contributions** (the nav's and the heading's name; the URL stays) — the site's totals (movegens, compute time, tasks) and the same by job type, then the contributor leaderboard — all workers (anonymous and authenticated) ranked by movegens (shown as "1.2M"; the exact count on hover), or by compute time or tasks at a click on the column; on a phone only the ranked column is shown beside the name, and a "Rank by" row above the list chooses it. |
| `/ratings` | Rating pool list — each pool's conditions, member count and last fit. |
| `/player-configs` | Every player config, newest first: its name, how it searches, its lexicon and leaves. Public, like the job pages that already show players' settings. |
| `/player-configs/[id]` | One config: a table of its key settings (its files, how moves are sorted, recorded and generated, plies, and whether it infers and solves the pre-endgame and endgame) that "All settings" grows to every setting — the job page's `PlayerSettingsTable` with one column — a JSON download, and the config it was cloned from. The job page's Settings card links each player here. |
| `/ratings/[id]` | [The ratings page](#the-ratings-page) — ratings with uncertainty, and the cross table with its residuals (no history chart: `GET /api/rating-pools/:id/history` serves API callers). Admin controls for membership appear inline for admins. |

### Auth Routes

| Route | Page |
|---|---|
| `/register` | Registration form — username, email, password with client-side strength feedback. |
| `/register/check-email` | Static holding page shown after successful registration — instructs the user to check their inbox. |
| `/confirm-email` | Email confirmation landing — reads `?code=` from the URL, auto-submits to the API, shows success or error. On success redirects to `/login`. |
| `/login` | Login form. On success redirects to `/account` or the originally requested page. |
| `/reset-password` | Password reset request form — enter email address. |
| `/reset-password/confirm` | Password reset apply form — reads token from URL, shows new password field. |

### Authenticated Routes

Protected by a layout guard (`/account/+layout.svelte`) that redirects unauthenticated users to `/login`.

| Route | Page |
|---|---|
| `/account` | Account overview — username, email, API key list (labels only), generate/revoke API keys. (No confirmation status: an unconfirmed account cannot sign in, so every account that sees this page is confirmed.) |

### Admin Routes

Protected by a layout guard (`/admin/+layout.svelte`) that requires `is_admin = true`; redirects non-admins to `/`.

| Route | Page |
|---|---|
| `/admin` | Admin overview — redirects to `/jobs`, the job list; a job's page links ("Manage") to its admin page, `/admin/jobs/:id`. There is no `/admin/jobs` list; `/admin/jobs/new` creates a job. |
| `/admin/allocation` | Every active and inactive job with its allocation, set together and saved in one request (`PUT /api/admin/jobs/allocations`): a running total that turns red above 100% and holds the save, "Share equally", and only the jobs changed are sent. |
| `/admin/jobs/new` | Create job form — job type selector, then type-specific config fields; a games or pairs job's players are a checklist — one ticked is a self-play job, several a [round robin](#admin-api-semantics) of a job per pairing, seated in the order ticked — with a live preview of the matchups ("4 configs → 6 jobs" and each job's name), and creating one goes to `/admin/allocation` with the new jobs marked, where they are started (any other job type goes to its own page); a games or pairs job can be set to save the positions it plays (`capture_positions`), which caps its batch at 1,000 games or 500 pairs, and a pairs job saving them to keep only where each pair first diverges (`capture_first_divergence`); a games or pairs job's **Threading** is "Intra-game parallelism (all threads on one game)" (`igp`, the default: every thread on one game's simulation, which makes an iteration-bounded simulation reproducible) or "Per-game parallelism (one game per thread)" (`pgp`), which matters only when a player simulates. The letter distribution and board layout start empty ("Choose…") and must be picked, here and on the rating-pool form: the first of each imported is no default worth having. A games or pairs job's **Significance Test** checkbox, ticked, shows its **Confidence %** (95) and minimum; an opening-rack job's consensus is three fields, Minimum and Maximum Analyses Per Rack and Consensus %. Every field is named as the settings tables name it, in Title Case. |
| `/admin/jobs/[id]` | Admin job view — the public page's four headline cards (status, allocation, tasks completed, ETA), the job's progress, its settings, match score and Significance Test cards as the public page has them, for an opening-rack job a Consensus card that changes its consensus settings (`PATCH .../consensus`), contributors and data gaps (what workers declined it for) plus the job's allocation, read-only with a link to `/admin/allocation` (the only place a job is switched on or off), and controls: force-complete, purge, delete (each asks first: none can be taken back), an artifact check and "merge progress now" for leave generation, and the export panel — a completed job's final export or a snapshot of a running one; start, poll, download. |
| `/admin/player-configs` | Player config list — name, recorder type, sort strategy, sim parameters. |
| `/admin/player-configs/new` | Create player config form, each field named as the settings tables name it with its MAGPIE argument beside it ("Move Recorder (-r)", "Sorted By (-s)"). No number box on either form has spinner arrows. |
| `/admin/rating-pools/new` | Create rating pool form — name, variant, letter distribution, board layout, anchor config and rating, and optionally the other members to add once it exists. Linked from `/ratings` for an admin; membership is managed on the pool's page after that. |
| `/admin/users` | User account list — delete accounts. (Contribution stats are shown publicly at `/users`.) |
| `/admin/workers` | Worker ban management — ban / unban workers by user ID or anonymous UUID. |
| `/admin/audit-log` | Audit log viewer — filterable by action and target type; paginated. (Not by actor: the page has no actor filter.) |
| `/admin/input-data` | Input data browser and import wizard — pick a tarball date, watch the import, review the staged diff, confirm. |
| `/admin/fleet` | What MAGPIE versions have claimed work recently, from `task_claims.magpie_version`. |
| `/admin/derived-data` | The wordmap, rack info table and word info table build queue: what is built, pending or failed, and a retry for the failures. It reads the queue again for as long as it is open — every 3 s while anything is pending or building, every 10 s otherwise, not while the tab is hidden (`lib/poller.ts`) — so a build queued later shows up by itself; the admin job page's list of the files a job waits on does the same. Polled rather than pushed: the builder is a separate process on a five-minute schedule, and a push would need Postgres `LISTEN`/`NOTIFY` for little gain. |
| `/admin/backups` | Recent backup runs and the staleness of the newest successful one. |
| `/admin/settings` | The settings an admin changes at run time (`GET`/`PUT /api/admin/settings`): the task time limit, in seconds, with who changed it last. |

---

### The ratings page

`/ratings/[id]` is where a pool's fit is read. Three panels, each answering a
different question, and the design choices in them are load-bearing:

**Ratings, as a dot plot with error bars.** Deliberately not a bar chart: a bar
encodes magnitude from zero, and a rating has no meaningful zero — only gaps
mean anything, and the level is wherever the pool's anchor was pinned — so bar
length would imply a ratio that does not exist. A dot on a common scale encodes
position, which is what a rating is. The error bar matters as much as the dot, because a config with two
hundred pairs and one with two million otherwise produce identical-looking
numbers and only the interval says which to believe. A config with no path to the
anchor is listed beneath the chart as **unrated** rather than drawn at a number.

**The cross table**, straight after the plot. Every rated config against every
other: win %, its standard error and the average spread, the rating last ([The
cross table](#the-cross-table)). A cell's background is its record as the cell
shows it -- green over 50.0%, red under, none at 50.0% (`recordSide`: the
rounded figure, never the float, so the colour cannot disagree with the
number). Under it one line saying that a gap means what it does between WESPA
players and the absolute level only where the anchor was pinned ([The scale is
WESPA's](#the-scale-is-wespas)). Its hover carries what the ratings predict, so this
is the panel that makes non-transitivity visible instead of letting it quietly
distort the ranking: a cell the ratings predict badly has amber text (on its
record's tint: it can be both), and when enough
head-to-heads are badly mispredicted the page says outright that the ratings
should be read as a summary rather than a ranking. It replaced a separate
residual table, listed largest disagreement first, which said the same about
the model in a list nobody could read against the results.

**A table of every config**, for admins only, after the cross table: the
exact rating to a decimal, ± SE as a number and pairs played per config, with
the membership controls. A visitor has the interval on the plot and the rating
in the cross table's last column, which said the same thing twice.

Admin controls live inline on this page rather than under `/admin`, because
adding or removing a config is an act whose consequence — every other rating
moving — is only legible next to the ratings themselves.

## Directory Structure

```
birdtest/
├── .github/
│   └── workflows/
│       ├── ci.yml                  # per pull request: clippy + backend tests (with Postgres),
│       │                           # frontend check/build, the three images, tier 5's
│       │                           # journeys, terraform fmt/validate/test, dev-restore's
│       │                           # SCRUB rule, runbook-check over RUNBOOK and README, the
│       │                           # fake worker's fixtures check, MAGPIE's half of the
│       │                           # message contract, and its derived-file builder hashes
│       └── nightly.yml             # tier 6: a real MAGPIE runs one task of every job type;
│                                   # also the opt-in (ignored) Rust tests against real MAGPIE,
│                                   # the restore round trip, the selective-restore and re-apply
│                                   # checks, and the backup drill
├── docker-compose.yml               # the whole local stack: Postgres, MinIO (S3 stub), backend,
│                                    # frontend, plus a `dev` profile — see Development
├── docker-compose.e2e.yml           # the end-to-end suite's overlay: its own project, file mail, a
│                                    # GitHub stand-in, and the fake workers (defined only here)
├── .env.example                     # compose port overrides and MAGPIE_ROOT
├── rust-toolchain.toml              # the pinned Rust toolchain
├── README.md                        # running it locally, and MAGPIE on the server
├── TESTING.md                       # the seven test tiers and every test id
├── RUNBOOK.md                       # operating production: restores, rotations, incidents
├── JOURNEYS.md                      # the manual pre-launch checklist
├── SETTINGS_COMPARISON.md           # an earlier design record, superseded by this document
├── contract-fixtures/               # one committed example of each worker API message, parsed
│                                    # by both sides' tests (mirrored in MAGPIE)
├── fixtures/                        # tier 5's GitHub stand-in: MAGPIE-DATA tarballs, served by
│                                    # nginx.conf for the import tests
├── e2e/                             # tier 5: Playwright journeys (tests/), helpers (lib/), and
│                                    # run.sh, which brings an isolated stack up and down
├── docker/
│   └── Dockerfile                  # backend (with a pinned MAGPIE built in), derived-builder and
│                                   # fake-worker targets; only the fake worker needs no MAGPIE
├── scripts/                        # backup, restore and local-snapshot shell scripts —
│                                   # see Backups and Restore
│   ├── backup.sh                   # the nightly pg_dump, its manifest, and the `backups` row
│   ├── restore-drill.sh            # the monthly automated restore drill
│   ├── restore-roundtrip.sh        # dump -> drop -> restore -> verify, against the local stack
│   ├── prod-sql.sh                 # run SQL against production from inside the VPC (infra/ops.tf)
│   ├── prod-shell.sh               # an interactive psql-capable shell there, over ECS Exec (--attach to return)
│   ├── dev-dump.sh                 # snapshot the local Postgres + MinIO state
│   ├── dev-restore.sh              # put it back
│   ├── dev-restore-check.sh        # its SCRUB rule against a stub compose (CI)
│   ├── runbook-check.sh            # every bash block in RUNBOOK.md and README.md parses,
│   │                               # and each that calls aws turns its pager off (CI)
│   ├── restore-job.sh              # copy one purged or deleted job back from a scratch restore
│   │                               # (RUNBOOK §2.2; also embedded in the ops task, infra/ops.tf)
│   ├── restore-job-check.sh        # restore-job.sh through its failure and re-run cases (nightly)
│   ├── reapply-check.sh            # RUNBOOK §1's security re-apply step against a restored copy (nightly)
│   ├── backup-drill-check.sh       # backup.sh and restore-drill.sh against a local stack (nightly)
│   ├── e2e_magpie_native.sh        # tier 6 natively: real MAGPIE, throwaway Postgres and MinIO
│   ├── capture_contract.py         # capture the worker API's contract fixtures
│   ├── fake-worker-fixtures.sh     # re-emit the fake worker's captured submissions; --check in CI
│   ├── dev.py                      # bring the stack up with real MAGPIE contributors
│   ├── seed.py                     # seed an admin, input data, player configs and jobs
│   ├── e2e_magpie.py               # tier 6: one real `magpie contribute` task per job type
│   └── scrub.sql                   # strip emails, password hashes and tokens after a local restore
├── backend/                        # Axum web server (Rust)
│   ├── Cargo.toml
│   ├── build.rs                    # rebuilds when a migration file is added or removed
│   ├── .config/nextest.toml        # the per-test time limit
│   ├── .env.example                # DATABASE_URL, SESSION_SIGNING_KEY, MAIL_BACKEND=console, etc. for local dev
│   ├── migrations/                 # sqlx migration files — a single one until release;
│   │   └── 0001_initial.sql        # see Development for why
│   ├── tests/                      # tiers 2-3: a cloned database per test (TEST_DATABASE_URL)
│   └── src/
│       ├── main.rs                 # binary: config, pool, migrations, sweeps, serve
│       ├── bin/
│       │   └── build-derived.rs    # the derived-file builder: drains `derived_data`, then exits;
│       │                           # a scheduled ECS task on the same image (infra/derived.tf)
│       ├── lib.rs                  # module tree and router assembly, shared with tests/
│       ├── clientip.rs             # the caller's address behind TRUSTED_PROXY_HOPS proxies
│       ├── config.rs               # config from env (ECS injects SSM values as env vars)
│       ├── state.rs                # AppState shared by every handler
│       ├── db.rs                   # the two pools (main, and the bounded display pool) and migrations
│       ├── error.rs                # AppError type, IntoResponse impl
│       ├── extract.rs              # ApiJson: the JSON body with AppError as its rejection,
│       │                           # large bodies parsed on the blocking pool
│       ├── version.rs              # semver parsing and comparison for the MAGPIE floor
│       ├── compat.rs               # MAGPIE's lexicon/leaves/letter-distribution compatibility
│       │                           # rules, ported to Rust — see Input Data
│       ├── inputdata.rs            # tarball fetch, untar, per-file digest, diff against input_data
│       ├── magpie.rs               # the pinned MAGPIE binary as a subprocess, and its scratch
│       │                           # data directories — see README.md, "MAGPIE on the server"
│       ├── magpie_standard15.txt   # the board layout every scratch directory carries so MAGPIE starts
│       ├── magpie_defaults.rs      # MAGPIE's defaults, written into player configs and jobs at creation
│       ├── derived.rs              # wordmaps, rack info and word info tables: what a job needs, the build
│       │                           # queue, the builds, and the dispatch gate
│       ├── backups.rs              # reads the `backups` table; never performs a backup
│       ├── board.rs                # a board layout as MAGPIE reads it: creation's check, and the
│       │                           # saved-positions board
│       ├── auth/
│       │   ├── mod.rs              # CurrentUser / AdminUser / WorkerIdentity extractors
│       │   ├── session.rs          # Paseto token creation / validation
│       │   ├── api_key.rs          # API key, password and code hashing
│       │   └── csrf.rs             # CSRF double-submit verification
│       ├── email.rs                # SES, console and file mail backends
│       ├── artifacts.rs            # S3 (MinIO in dev) artifact store; multipart upload and presigned reads
│       ├── exports.rs              # a job's results as one gzipped NDJSON artifact: final for a
│       │                           # completed job, a snapshot of a running one
│       ├── ratelimit.rs            # in-memory governor token buckets
│       ├── audit.rs                # append-only audit log writes
│       ├── scheduler.rs            # job selection, lazy reclamation, task claiming
│       ├── jobstats.rs             # aggregate job stats (REST + SSE payload)
│       ├── ratings.rs              # rating pools: evidence, fits, snapshots
│       ├── stats/
│       │   ├── mod.rs
│       │   ├── match_test.rs       # the match test: player 1's score interval and its verdict
│       │   ├── outcomes.rs         # Tally, Pentanomial and Sample, which the test and the ratings read
│       │   └── bradley_terry.rs    # batch anchored rating fit (Newton)
│       ├── jobs/                   # job type system
│       │   ├── mod.rs              # shared request/record helpers
│       │   ├── handler.rs          # JobHandler trait plus wire types
│       │   ├── plausibility.rs    # impossibility checks on submissions
│       │   ├── registry.rs         # JobType dispatch (exhaustive matches)
│       │   ├── dispatch.rs         # a job's immutable template, read once per process
│       │   ├── racks.rs            # letter distributions, rack/leave enumeration, CGP
│       │   ├── opening_rack.rs
│       │   ├── game.rs
│       │   ├── game_pair.rs
│       │   ├── leave_gen.rs
│       │   └── testdata/           # letter distributions and fake-worker results the unit tests
│       │                           # include
│       ├── models/                 # SQLx row types
│       │   ├── mod.rs
│       │   ├── job.rs
│       │   └── user.rs
│       ├── routes/                 # Axum handlers (one file per API section)
│       │   ├── mod.rs              # pagination helpers
│       │   ├── worker.rs           # /api/worker/*
│       │   ├── auth.rs             # /api/auth/*
│       │   ├── account.rs          # /api/me/*
│       │   ├── admin.rs            # /api/admin/*
│       │   ├── ratings.rs          # /api/rating-pools/* (public reads, admin writes)
│       │   └── public.rs           # /api/jobs/*, /api/users, /api/workers, /api/player-configs/*
│       └── sse.rs                  # SSE broadcaster (job result push)
│
├── frontend/                       # SvelteKit app
│   ├── Dockerfile                  # static build served by Nginx — the same artifact ECS runs
│   ├── docker/default.conf.template # SPA fallback + /api proxy (SSE needs proxy_buffering off); the
│   │                               # backend's address filled in at start (BACKEND_UPSTREAM).
│   │                               # The proxy is compose's: in production the ALB routes /api
│   ├── package.json
│   ├── svelte.config.js
│   ├── vite.config.ts
│   └── src/
│       ├── app.html
│       ├── app.css
│       ├── lib/                    # each .ts beside a .test.ts (tier 1F); two tests check files
│       │                           # outside lib: contributeDocs (the pages' contributor
│       │                           # instructions) and nginxConfig (the nginx template
│       │                           # against infra/ecs.tf)
│       │   ├── api.ts              # typed fetch wrappers for every API endpoint
│       │   ├── auth.ts             # session store (current user, is_admin)
│       │   ├── accountRules.ts     # the account forms' checks, run before a request
│       │   ├── sse.ts              # SSE subscription helper
│       │   ├── poller.ts           # re-reading while a page shows it (the derived-data pages)
│       │   ├── importWatch.ts      # polling an input-data import until it stops
│       │   ├── format.ts           # shared display formatting (job type labels, durations)
│       │   ├── cgp.ts              # reading MAGPIE's CGP and move strings for the board
│       │   ├── moveList.ts         # a move list's simulation columns and inference
│       │   ├── compare.ts          # a two-player table row, better green and worse red
│       │   ├── matchScore.ts       # the match score's rows
│       │   ├── matchTest.ts        # the significance test card's interval and sentence
│       │   ├── consensus.ts        # an opening-rack job's consensus settings and standing
│       │   ├── roundRobin.ts       # a games or pairs request's matchups, previewed as the server makes them
│       │   ├── jobSettings.ts      # a job's and a player config's settings tables
│       │   ├── charts/             # pentanomial.ts (the pair-outcome table), ratingDotPlot.ts,
│       │   │                       # residuals.ts, labels.ts: the arithmetic behind the charts
│       │   └── components/         # shared UI components
│       │       ├── JobStatusBadge.svelte
│       │       ├── JobStatusCard.svelte  # where a job stands, a row above the headline figures
│       │       ├── JobStatsRow.svelte    # the four headline figures
│       │       ├── JobSettings.svelte    # a job's settings and its players'
│       │       ├── PlayerSettingsTable.svelte # player configs' settings, a column per player
│       │       ├── TaskCounts.svelte     # a job's tasks by state
│       │       ├── MatchScore.svelte     # the match score (and its divergent-games table)
│       │       ├── MatchTestCard.svelte  # the significance test
│       │       ├── SavedPositions.svelte # captured positions, one at a time
│       │       ├── PositionPane.svelte   # one position: board, ranked moves, CGP
│       │       ├── Board.svelte          # a position on the job's own board
│       │       ├── ConsensusEditor.svelte # changing an opening-rack job's consensus
│       │       ├── DerivedDataStatus.svelte # the derived files a job waits on
│       │       ├── WorkerTable.svelte
│       │       ├── Pagination.svelte
│       │       ├── ProgressBar.svelte
│       │       ├── RatingDotPlot.svelte  # ratings with error bars (not a bar chart: a rating has no zero)
│       │       └── PlayerCompareTable.svelte # two players side by side, higher green
│       └── routes/
│           ├── +layout.svelte      # global layout (nav bar, footer)
│           ├── +layout.ts          # a static SPA: no SSR, no prerendering
│           ├── +page.svelte                        # /
│           ├── jobs/
│           │   ├── +page.svelte                    # /jobs
│           │   └── [id]/
│           │       └── +page.svelte                # /jobs/[id]
│           ├── ratings/
│           │   ├── +page.svelte                    # /ratings
│           │   └── [id]/
│           │       └── +page.svelte                # /ratings/[id]
│           ├── users/
│           │   └── +page.svelte                    # /users
│           ├── workers/
│           │   └── +page.svelte                    # /workers
│           ├── player-configs/
│           │   ├── +page.svelte                    # /player-configs
│           │   └── [id]/
│           │       └── +page.svelte                # /player-configs/[id]
│           ├── register/
│           │   ├── +page.svelte                    # /register
│           │   └── check-email/
│           │       └── +page.svelte                # /register/check-email
│           ├── confirm-email/
│           │   └── +page.svelte                    # /confirm-email
│           ├── login/
│           │   └── +page.svelte                    # /login
│           ├── reset-password/
│           │   ├── +page.svelte                    # /reset-password
│           │   └── confirm/
│           │       └── +page.svelte                # /reset-password/confirm
│           ├── account/
│           │   ├── +layout.svelte                  # auth guard: redirect to /login if no session
│           │   └── +page.svelte                    # /account
│           └── admin/
│               ├── +layout.svelte                  # auth guard: redirect to / if not is_admin
│               ├── +page.svelte                    # /admin (redirects to /jobs)
│               ├── allocation/
│               │   └── +page.svelte                # /admin/allocation — every job's share at once
│               ├── jobs/
│               │   ├── new/
│               │   │   └── +page.svelte            # /admin/jobs/new
│               │   └── [id]/
│               │       └── +page.svelte            # /admin/jobs/[id]
│               ├── player-configs/
│               │   ├── +page.svelte                # /admin/player-configs
│               │   └── new/
│               │       └── +page.svelte            # /admin/player-configs/new
│               ├── rating-pools/
│               │   └── new/
│               │       └── +page.svelte            # /admin/rating-pools/new
│               ├── users/
│               │   └── +page.svelte                # /admin/users
│               ├── workers/
│               │   └── +page.svelte                # /admin/workers
│               ├── audit-log/
│               │   └── +page.svelte                # /admin/audit-log
│               ├── input-data/
│               │   └── +page.svelte                # /admin/input-data — import wizard
│               ├── fleet/
│               │   └── +page.svelte                # /admin/fleet
│               ├── derived-data/
│               │   └── +page.svelte                # /admin/derived-data — the build queue
│               └── backups/
│                   └── +page.svelte                # /admin/backups
│
├── worker/                         # fake_worker.py only; the contributor client is MAGPIE itself
│   └── fake_worker.py              # synthetic results, no MAGPIE; test stacks only, never production
│
│                                   # There is no `data/` directory. The server used to read letter
│                                   # distributions off disk from DATA_PATH; it now reads them out of
│                                   # the `input_data` row the job pins — see Input Data.
│
└── infra/                          # Terraform
    ├── main.tf
    ├── variables.tf
    ├── outputs.tf
    ├── ecs.tf                      # ECS cluster, task definition, service
    ├── derived.tf                  # the derived-file builder: task definition, role, schedule
    ├── rds.tf                      # RDS Postgres instance, security group, PITR retention
    ├── s3.tf                       # artifact bucket: versioning, lifecycle, cross-region replication
    ├── backup.tf                   # backup bucket (KMS, Object Lock, CRR), the nightly dump task,
    │                               # its schedule, the alarms, and the monthly restore drill
    ├── ses.tf                      # SES domain and sending identity
    ├── ops.tf                      # the ops task: psql and the restore script inside the VPC
    ├── ssm.tf                      # the two SSM parameters' names and ARNs (never managed or read)
    └── tests/
        └── variables.tftest.hcl    # S-TF-1..3: every variable validation and the
                                    # deploy-failed alert, against mock providers
                                    # (`terraform test`, CI)
```

---

## Schema

The authoritative copy is [`backend/migrations/0001_initial.sql`](backend/migrations/0001_initial.sql),
reproduced here in full. Until birdtest is deployed this is the *only* migration and
schema changes edit it in place (see [Development](#development)), so the two must be
kept in step by hand -- if they ever disagree, the migration is right.

```sql
-- Users

CREATE TABLE users (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Unique whatever its case: users_username_lower_idx, below, which a
    -- plain UNIQUE here only duplicated (an index write per user update).
    username             TEXT NOT NULL,
    email                TEXT NOT NULL UNIQUE,
    password_hash        TEXT NOT NULL,
    email_confirmed_at   TIMESTAMPTZ,
    is_admin             BOOLEAN NOT NULL DEFAULT FALSE,
    -- Embedded in every session token and compared on every request. Bumped by
    -- a password reset, "sign out everywhere" and account deletion, which is
    -- what revokes every session minted before.
    session_generation   INT NOT NULL DEFAULT 0,
    -- Set when an admin deletes the account. Deletion anonymizes rather than
    -- removes the row: username, email and password are replaced by
    -- tombstones and API keys are deleted, but the account's claims and
    -- results stay, so no donated compute is lost.
    deleted_at           TIMESTAMPTZ,
    -- Completed claims by this account, and when the last one landed. Running
    -- totals rather than a COUNT over task_claims: the contributor lists order
    -- by this, so counting it meant computing every user's whole history before
    -- a LIMIT could apply. Maintained in the submit transaction.
    --
    -- Unlike the counters on `jobs`, this one spans jobs, so it is not enough
    -- to zero it when a job goes: purge_job and delete_job decrement it by what
    -- they are about to destroy. `last_completed_at` is deliberately NOT
    -- rewound by those, since finding the new maximum means the scan the
    -- counter exists to avoid; it only ever moves forward, and is a display
    -- figure.
    tasks_completed      BIGINT NOT NULL DEFAULT 0 CHECK (tasks_completed >= 0),
    -- The rest of the contribution, kept the same way and given back the same
    -- way: the time each accepted claim was held, claim to submission, in
    -- milliseconds (MAGPIE reports no CPU or thread counts, so this is the
    -- only measure of time there is); and the move generations those claims
    -- performed, as MAGPIE counts and reports them with each result -- every
    -- call to its move generator, in sims, endgame and pre-endgame searches
    -- and autoplay alike, on every thread. Movegens are the measure of work
    -- done, independent of how fast a machine is or how long it held a
    -- claim, and what the contributor list ranks by. Summed from each claim's
    -- own figure (`task_claims.movegens`), every claim counted: unlike a job's
    -- progress, which counts an opening rack once however often consensus has
    -- it analysed again, a contributor did the work of each claim.
    -- Integer milliseconds rather than fractional seconds so that what a purge
    -- takes back is exactly what the submissions added.
    compute_ms           BIGINT NOT NULL DEFAULT 0 CHECK (compute_ms >= 0),
    movegens             BIGINT NOT NULL DEFAULT 0 CHECK (movegens >= 0),
    last_completed_at    TIMESTAMPTZ,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Serves /api/users, which ranks accounts by contribution. Partial because a
-- deleted account is never listed.
CREATE INDEX users_contribution_idx ON users (tasks_completed DESC, created_at ASC)
    WHERE deleted_at IS NULL;

-- Serve the account half of /api/workers, one per order that list offers --
-- movegens (its default), compute time and tasks -- with the list's own
-- predicate, so whichever it is sorted by it is the same contributors (see
-- anonymous_workers_contribution_idx). A deleted account keeps its place
-- there: its work was done, and it is listed under its anonymized name.
CREATE INDEX users_worker_rank_idx ON users (tasks_completed DESC, id)
    WHERE tasks_completed > 0;
CREATE INDEX users_worker_compute_idx ON users (compute_ms DESC, id)
    WHERE tasks_completed > 0;
CREATE INDEX users_worker_movegens_idx ON users (movegens DESC, id)
    WHERE tasks_completed > 0;

CREATE TABLE email_confirmations (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash   TEXT NOT NULL,
    expires_at  TIMESTAMPTZ NOT NULL,
    used_at     TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE password_reset_tokens (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash  TEXT NOT NULL,
    expires_at  TIMESTAMPTZ NOT NULL,
    used_at     TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Confirmation and reset look a token up by its hash, and the stale-account
-- release (and a user's delete) reaches both tables by user through the
-- cascade; neither table is reaped, so each was a sequential scan growing
-- without bound, on unauthenticated routes.
CREATE INDEX email_confirmations_code_idx   ON email_confirmations (code_hash);
CREATE INDEX email_confirmations_user_idx   ON email_confirmations (user_id);
CREATE INDEX password_reset_tokens_hash_idx ON password_reset_tokens (token_hash);
CREATE INDEX password_reset_tokens_user_idx ON password_reset_tokens (user_id);

-- One account per username whatever its case: "Josh" and "josh" side by side
-- on a public leaderboard is an impersonation. Login matches the same way, so
-- whoever registered "Josh" can sign in as "josh"; this index serves it.
CREATE UNIQUE INDEX users_username_lower_idx ON users (lower(username));


CREATE TABLE api_keys (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    key_hash     TEXT NOT NULL UNIQUE,
    label        TEXT,
    is_active    BOOLEAN NOT NULL DEFAULT TRUE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ
);
-- A user's keys: the key list, the hundred-key check, and the cascade when a
-- user is deleted or an expired unconfirmed account is released.
CREATE INDEX api_keys_user_idx ON api_keys (user_id);
-- Enforce the 100-key limit per user at the application layer, not via a DB constraint.

-- Workers

CREATE TABLE anonymous_workers (
    uuid          UUID PRIMARY KEY,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Any request from this identity touches this, at most once a minute: it
    -- answers "is this worker still around".
    last_seen_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- The contribution counters, mirroring users.tasks_completed, compute_ms,
    -- movegens and last_completed_at; see there for what
    -- each counts, why they are counters and who decrements them.
    -- `last_completed_at` is distinct from `last_seen_at` above: one is the
    -- last task finished, the other is the last request of any kind, and the
    -- contributor list shows the first.
    tasks_completed   BIGINT NOT NULL DEFAULT 0 CHECK (tasks_completed >= 0),
    compute_ms        BIGINT NOT NULL DEFAULT 0 CHECK (compute_ms >= 0),
    movegens          BIGINT NOT NULL DEFAULT 0 CHECK (movegens >= 0),
    last_completed_at TIMESTAMPTZ
);

-- Serve the anonymous half of /api/workers, which merges both kinds of
-- identity in one ranking, one per order it offers (users_worker_*_idx is the
-- other half). Partial: an identity that has completed nothing is not a
-- contributor and is not listed, whatever the list is sorted by.
CREATE INDEX anonymous_workers_contribution_idx
    ON anonymous_workers (tasks_completed DESC, uuid) WHERE tasks_completed > 0;
CREATE INDEX anonymous_workers_compute_idx
    ON anonymous_workers (compute_ms DESC, uuid) WHERE tasks_completed > 0;
CREATE INDEX anonymous_workers_movegens_idx
    ON anonymous_workers (movegens DESC, uuid) WHERE tasks_completed > 0;

CREATE TABLE worker_bans (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID REFERENCES users(id) ON DELETE CASCADE,
    anon_uuid   UUID REFERENCES anonymous_workers(uuid) ON DELETE CASCADE,
    reason      TEXT,
    -- SET NULL, like jobs.created_by: deleting the admin who issued a ban must
    -- neither fail nor lift the ban.
    banned_by   UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT ban_has_single_target CHECK (
        (user_id IS NOT NULL)::int + (anon_uuid IS NOT NULL)::int = 1
    )
);

-- One ban per identity. A second row for an identity already banned carries no
-- information -- enforcement is an EXISTS, so the second reason is never read
-- -- and it breaks unban, which deletes by row id: an admin lifts a ban, one
-- row goes, the identity stays banned, and nothing says why. A duplicate is a
-- 409 through the usual unique-violation mapping instead. "Ban again with a
-- different reason" survives as unban-then-ban, which the audit log records as
-- both halves.
CREATE UNIQUE INDEX worker_bans_user_idx ON worker_bans (user_id)
    WHERE user_id IS NOT NULL;
CREATE UNIQUE INDEX worker_bans_anon_idx ON worker_bans (anon_uuid)
    WHERE anon_uuid IS NOT NULL;

-- Input data
--
-- Must precede jobs and player_configs: both reference input_data.

-- Every input data file birdtest knows about, identified by content.
--
-- A row is a (path, sha256) pair: the same path with different bytes is a
-- different row, which is the entire point. `tarball_date` records the
-- versioned tarball a row was FIRST seen in -- provenance, not membership. A
-- file unchanged between two tarballs stays one row labelled with the older
-- date, because it is the same bytes and a job pinning it is pinning those
-- bytes regardless of which tarball the contributor installed.
CREATE TABLE input_data (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Path relative to the data root, including basename: 'lexica/NWL23.kwg'.
    path         TEXT NOT NULL,
    -- Derived from `path` at import and stored because dispatch and the client
    -- protocol address files by (role, name), not by path: MAGPIE resolves a
    -- name through its own data_paths search list.
    role         TEXT NOT NULL CHECK (role IN ('kwg','klv','winpct','letterdist','layout')),
    name         TEXT NOT NULL,          -- 'NWL23', 'winpct', 'english', 'standard15'
    sha256       TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    bytes        BIGINT NOT NULL CHECK (bytes >= 0),
    -- YYYYMMDD name of the versioned tarball this content was first imported
    -- from. Text, not DATE: it is the artifact's name, and it appears verbatim
    -- in the message a contributor is told to act on.
    tarball_date TEXT NOT NULL CHECK (tarball_date ~ '^\d{8}$'),
    -- The file's bytes, for the roles the SERVER itself reads. birdtest
    -- enumerates rack universes and builds KLVs from the letter distribution,
    -- so those bytes must be the pinned ones -- there is no server-side disk
    -- copy of the data any more. Lexica stay out: a 15 MB .kwg in a row is a
    -- different proposition and nothing server-side reads one.
    --
    -- The check is an equivalence, not a nullable convenience: a letterdist or
    -- layout row without bytes cannot exist, and a kwg/klv/winpct row with
    -- bytes cannot either. Server-side code therefore has no "fall back to the
    -- filesystem" branch to write.
    content      BYTEA
                 CHECK ((role IN ('letterdist','layout')) = (content IS NOT NULL)),
    -- Object-store key for the bytes of the roles the server *builds* from, as
    -- opposed to the ones it parses. A reference wordmap needs the .kwg and a
    -- reference rack info table the .klv2 as well (see `derived_data`), so
    -- those two go to the object store rather than into `content`: a 6 MB
    -- lexicon and a 3.7 MB KLV in a row is a different proposition from a
    -- 489-byte distribution, there are fifty of each per tarball, and nothing
    -- queries their contents.
    --
    -- Keyed by digest, so the same bytes under two paths are one object and a
    -- file unchanged between two tarballs is uploaded once. NULL for every
    -- other role, and for a row whose bytes were never stored -- a derived
    -- build from one of those fails naming the remedy rather than finding a
    -- missing key. Not an equivalence CHECK like `content` above, because
    -- nothing stops a row being written by something other than the import.
    object_key   TEXT,
    imported_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    imported_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    UNIQUE (path, sha256)
);

-- Staged imports. Phase 1 (a spawned background task) writes; phase 2 reads and
-- commits. Rows here are proposals, not data -- nothing dispatch or job creation
-- reads. birdtest runs as a single instance, so a task needs no lease and
-- startup may fail any row still 'running'.
CREATE TABLE input_data_imports (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tarball_date   TEXT NOT NULL CHECK (tarball_date ~ '^\d{8}$'),
    commit_sha     TEXT NOT NULL,
    -- NULL until the download completes: the row exists from the moment the
    -- background task is spawned.
    tarball_sha256 TEXT,
    -- 'nothing_new': staged, every file already known, so there is nothing
    -- to confirm; it goes there straight from 'running' rather than waiting
    -- a day in 'staged' to be expired.
    state          TEXT NOT NULL DEFAULT 'running'
                   CHECK (state IN ('running', 'staged', 'nothing_new', 'confirmed',
                                    'cancelled', 'failed')),
    -- What the poller renders while state = 'running'.
    progress_bytes   BIGINT NOT NULL DEFAULT 0,
    progress_entries INT    NOT NULL DEFAULT 0,
    -- Why it failed, shown verbatim to the admin.
    error          TEXT,
    requested_by   UUID REFERENCES users(id) ON DELETE SET NULL,
    requested_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    confirmed_at   TIMESTAMPTZ
);

CREATE TABLE input_data_import_rows (
    import_id  UUID NOT NULL REFERENCES input_data_imports(id) ON DELETE CASCADE,
    path       TEXT NOT NULL,
    role       TEXT NOT NULL,
    name       TEXT NOT NULL,
    sha256     TEXT NOT NULL,
    bytes      BIGINT NOT NULL,
    -- 'new' | 'known' | 'collision' (same path, different sha256 already known)
    disposition TEXT NOT NULL,
    -- Carried from phase 1 for letterdist/layout entries so confirmation
    -- inserts input_data.content without re-downloading the tarball.
    content     BYTEA,
    -- Likewise for kwg/klv entries, whose bytes go to the object store while
    -- the archive is still in memory. An import the admin then cancels leaves
    -- an object nobody references, which is keyed by digest and so is exactly
    -- what the next import of the same file would have uploaded anyway.
    object_key  TEXT,
    PRIMARY KEY (import_id, path, sha256)
);

-- Derived files: the wordmaps, rack info tables and word info tables the
-- server builds a reference copy of, and the hash a worker has to reproduce.
--
-- None of them is ever shipped -- 179 MB and 1.9 GB for a CSW24 wordmap and
-- rack info table -- so every machine that needs one builds it from files it
-- already has. What travels instead is the SHA-256 the server's own pinned
-- MAGPIE got from the same inputs: a worker builds its own copy and uses it
-- only if the bytes agree, and declines the task otherwise. See README.md,
-- "MAGPIE on the server".
--
-- The key is the whole identity of the file rather than a surrogate, because
-- what makes two derived files the same file is that they were built from the
-- same inputs by the same builder. A wordmap and a word info table depend on a
-- .kwg and the letter distribution they are built against; a rack info table
-- depends on a .klv2 as well, because its entries carry precomputed leave
-- values.
--
-- `builder` is separate from the MAGPIE version on purpose. A CSW24 wordmap
-- built in December 2025 and one built nine months later differ in 72,852,152
-- bytes with the same inputs and the same wordmap format version: the builder
-- changed and the format did not have to. MAGPIE carries WMP_BUILDER_VERSION
-- and RIT_BUILDER_VERSION for exactly this, a test pins their output so a
-- change cannot pass without bumping them, and the server asks the binary it
-- runs (`magpie builders`) rather than being told in configuration.
CREATE TABLE derived_data (
    role          TEXT NOT NULL CHECK (role IN ('wmp','rit','wit')),
    -- What the worker loads the file as. A wordmap's and a word info table's
    -- is its lexicon's name; a rack info table's is '<lexicon>.<leaves>',
    -- because a table belongs to a (.kwg, .klv2) pair and two jobs on CSW24
    -- with different leaves must not share one.
    name          TEXT NOT NULL,
    builder       TEXT NOT NULL,          -- 'wmp-1', 'rit-1', 'wit-1'
    kwg_id        UUID NOT NULL REFERENCES input_data(id),
    -- NULL for a wordmap and a word info table, which are built from the
    -- lexicon alone. The partial
    -- unique indexes below are what make (role, name, builder, kwg, NULL) a key
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
    -- A wordmap and a word info table are built from the lexicon and the
    -- distribution; a rack info table additionally from the leaves. A wmp or
    -- wit row carrying a klv_id would be claiming a dependency it does not
    -- have.
    CONSTRAINT derived_data_inputs_match_role CHECK (
        (role = 'rit') = (klv_id IS NOT NULL)
    )
);

-- The identity of a derived file, one index per role. Partial indexes because
-- a wordmap's and a word info table's klv_id is NULL and NULLs are distinct in a
-- UNIQUE constraint, which would silently permit duplicate wordmap rows.
CREATE UNIQUE INDEX derived_data_wmp_idx
    ON derived_data (name, builder, kwg_id, letterdist_id)
    WHERE role = 'wmp';
CREATE UNIQUE INDEX derived_data_rit_idx
    ON derived_data (name, builder, kwg_id, klv_id, letterdist_id)
    WHERE role = 'rit';
CREATE UNIQUE INDEX derived_data_wit_idx
    ON derived_data (name, builder, kwg_id, letterdist_id)
    WHERE role = 'wit';

-- The builder task's queue: oldest request first, so a job that has been
-- waiting is not starved by one created since.
CREATE INDEX derived_data_queue_idx
    ON derived_data (requested_at)
    WHERE state IN ('pending','building');

-- Jobs

CREATE TYPE job_type AS ENUM (
    'opening_rack',
    'games',
    'game_pairs',
    'leave_generation'
);

CREATE TYPE job_status AS ENUM (
    'active',
    'inactive',
    'completed'
);

CREATE TABLE jobs (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- What the admin called it, shown first wherever jobs are listed. '' for
    -- a job created without one (through the API; the form asks for it).
    name       TEXT NOT NULL DEFAULT '' CHECK (char_length(name) <= 100),
    job_type   job_type NOT NULL,
    -- Every active job's share of the fleet: the scheduler hands each claim
    -- to the active job furthest behind
    -- `(claims_issued - claims_baseline) / allocation`, and the active jobs
    -- may allocate at most 100% between them. There is no priority: a job
    -- that should get nothing for now is at 0%, and a job that should get
    -- everything is the only one above 0%.
    allocation INT NOT NULL DEFAULT 0 CHECK (allocation BETWEEN 0 AND 100),
    -- Jobs start inactive at 0%. The allocation is the only switch: setting
    -- one above 0% activates a job and setting it to 0% deactivates it, so
    -- `inactive` and 0% are one state rather than two that could disagree --
    -- an active job at 0% was on offer to nobody while every page called it
    -- running. A completed job is not active, so it holds 0% too: nothing
    -- of its old share is kept to come back to (see `jobs_allocation_is_status`).
    status     job_status NOT NULL DEFAULT 'inactive',
    -- SET NULL if the creating admin's account is deleted.
    created_by           UUID REFERENCES users(id) ON DELETE SET NULL,
    -- Settings every job type has, regardless of what it does. The lexicon is
    -- NOT here: it lives on the player (player_configs.kwg_id), because MAGPIE
    -- scopes it per player and two players may run different ones.
    variant       TEXT NOT NULL,                            -- 'classic' | 'wordsmog'; a rules setting, not a file
    letterdist_id UUID NOT NULL REFERENCES input_data(id),  -- one per job: MAGPIE takes one -ld for the whole game
    layout_id     UUID NOT NULL REFERENCES input_data(id),  -- 'standard15' unless a job says otherwise
    -- Run-wide MAGPIE settings every request states: the bingo bonus, part of
    -- every play's score, and the simulation cutoff. Written at creation from
    -- MAGPIE's defaults (backend/src/magpie_defaults.rs) rather than left for
    -- each worker's build to supply, so a task means the same thing on every
    -- MAGPIE release. Leave generation states only the bingo bonus: its bot
    -- does not simulate.
    -- The bonus is bounded because the plausibility rules' score bounds are
    -- absolute and assume an ordinary one: a typo of 5000 for 50 would
    -- refuse every honest batch. Real variants use 0 to 50.
    bingo_bonus   INT NOT NULL CHECK (bingo_bonus BETWEEN 0 AND 500),  -- -bb
    sim_cutoff    DOUBLE PRECISION NOT NULL CHECK (sim_cutoff >= 0 AND sim_cutoff <= 100),  -- -cutoff
    -- Minimum MAGPIE version workers must have to execute tasks for this job,
    -- as sortable parts. Semver in TEXT compares lexically, where '1.10.0' <
    -- '1.9.0' -- a bug that appears only once a minor version reaches double
    -- digits, i.e. long after it is written.
    --
    -- Not nullable: every job pins input data, and a client too old to
    -- understand expected_data contributes unverified rather than declining,
    -- so "no floor" is not a state worth being able to express. 0.1.1 is
    -- the `birdtest-contribute` version the backend image pins; the branch's
    -- version moves whenever a change can alter what a task computes, and the
    -- floor moves with it. The default here is the same value as the server's
    -- MIN_MAGPIE_VERSION, which create_job writes explicitly; the two are kept
    -- equal so a row written any other way (a restore, a hand insert) does not
    -- floor a job below the server.
    min_magpie_major INT NOT NULL DEFAULT 0 CHECK (min_magpie_major >= 0),
    min_magpie_minor INT NOT NULL DEFAULT 1 CHECK (min_magpie_minor >= 0),
    min_magpie_patch INT NOT NULL DEFAULT 1 CHECK (min_magpie_patch >= 0),
    -- Every claim ever issued for this job, abandoned and declined ones
    -- included: the deficit the scheduler orders on. Kept as a counter rather
    -- than counted, because counting task_claims on every claim request costs
    -- time proportional to the job's whole history. Only ever incremented,
    -- except by a purge, which deletes the claims it counts.
    claims_issued   BIGINT NOT NULL DEFAULT 0 CHECK (claims_issued >= 0),
    -- Where this job's share is measured *from*. The scheduler orders on
    -- `(claims_issued - claims_baseline) / allocation`, and the baseline is
    -- reset -- on activation and on an allocation change (a purge leaves the
    -- job inactive, to be reset when it is activated again) -- so that
    -- the job's ratio equals the lowest ratio among the other jobs being
    -- served (see `last_claimed_at` below): it joins at parity and takes its
    -- share from then on.
    --
    -- Without it the deficit was measured over a job's whole life, so a job
    -- activated today beside one that had issued two million claims took
    -- *every* claim until it had issued two million of its own, and the older
    -- job got nothing for as long as that took; a purge (which zeroes
    -- claims_issued) and a raised allocation did the same. May be negative: a
    -- job with no claims yet joining a busy fleet is credited the claims that
    -- put it level.
    claims_baseline BIGINT NOT NULL DEFAULT 0,
    -- When this job last issued a claim; NULL until it has. It rides the
    -- `UPDATE jobs` every claim already makes, so it costs nothing to keep.
    -- `join_at_parity` reads it to tell the jobs that are *being served* from
    -- the ones that are merely on offer: a job whose derived files are still
    -- building, or that the fleet cannot run yet, stands still while the others
    -- climb, and a newcomer put level with *it* then took every claim from the
    -- jobs that were actually running until it had caught up with them.
    last_claimed_at TIMESTAMPTZ,
    -- The match test's verdict a games or game-pairs job was completed on, as
    -- the finish check saw it, with player 1's score interval and the units it
    -- had then: NULL for every other job, for one completed any other way (by
    -- an admin, or at its cap in the claim path), and for one that runs no
    -- test -- there is no verdict to keep, and its `job.completed` audit row
    -- says `reached_target` instead. The live figures are recomputed from
    -- every accepted result, and the claims in flight when a job completes are
    -- still played and accepted -- so without this the page of a job that
    -- decided could drift back to "running" with no record anywhere of the
    -- decision that stopped it. A purge clears it.
    test_decided_status TEXT CHECK (test_decided_status IN
                            ('player1_better', 'player2_better', 'inconclusive')),
    test_decided_lower  DOUBLE PRECISION,
    test_decided_upper  DOUBLE PRECISION,
    test_decided_units  BIGINT,
    -- Tasks of this job that hit the time limit (`settings.max_task_seconds`):
    -- those a worker stopped and handed back, declining them `time_limit`,
    -- and those whose claim the server took back at the deadline while the
    -- worker still heartbeat (`task_claims.overrun`, counted once the job's
    -- row is next taken). The job page says how many, since the cure is a
    -- smaller batch. And how many of those came in a row with no task of the
    -- job completed between, which an accepted result zeroes: at three the
    -- job is set aside -- inactive at 0%, with `set_aside_reason` saying why
    -- -- since a job whose one unit always outlasts the limit would otherwise
    -- be handed out, run for the limit and handed back for ever. Giving it an
    -- allocation again starts the run afresh and clears the reason; a purge
    -- zeroes all three.
    time_limit_declines BIGINT NOT NULL DEFAULT 0 CHECK (time_limit_declines >= 0),
    time_limit_streak   INT NOT NULL DEFAULT 0 CHECK (time_limit_streak >= 0),
    -- When the run `time_limit_streak` counts began: the last accepted result,
    -- or the allocation that put the job back; NULL for a job whose run has
    -- not been broken. An overrun is counted after the fact, so whether it is
    -- part of the run is a question of when it happened -- its deadline --
    -- not of when it was counted: one that overran before the completion or
    -- the allocation that ended its run counts toward the total alone.
    time_limit_streak_since TIMESTAMPTZ,
    -- Why the server switched the job off, when it did; read only while the
    -- job is inactive.
    set_aside_reason    TEXT,
    -- The invariant above, for every status: active exactly when above 0%,
    -- which holds an inactive or completed job at 0%.
    CONSTRAINT jobs_allocation_is_status CHECK ((status = 'active') = (allocation > 0)),
    CONSTRAINT jobs_test_decided_together CHECK (
        (test_decided_status IS NULL) = (test_decided_lower IS NULL)
        AND (test_decided_status IS NULL) = (test_decided_upper IS NULL)
        AND (test_decided_status IS NULL) = (test_decided_units IS NULL)
    ),
    -- Progress totals the dashboard reads, maintained in the submit transaction
    -- rather than counted on read (PLAN.md, "What these reads cost"), once per
    -- accepted result: a task has one slot, so one result.
    --
    -- games_completed counts GAMES for both games and game_pairs; a pairs job's
    -- unit count is half of it, exactly as the read derived it. racks_analyzed
    -- counts distinct opening racks with an accepted analysis, which is a plain
    -- sum because each task covers its own disjoint slice of the rack space.
    --
    -- Neither is authoritative for anything that decides: the match test still reads
    -- game_results, so a drifted counter shows a wrong number on a page and
    -- cannot stop a job early. A purge zeroes them; a partial restore
    -- recomputes them (RUNBOOK 2.3).
    games_completed BIGINT NOT NULL DEFAULT 0 CHECK (games_completed >= 0),
    racks_analyzed  BIGINT NOT NULL DEFAULT 0 CHECK (racks_analyzed >= 0),
    -- An opening-rack job's racks that need no more analysis (see
    -- job_opening_rack_config's consensus), and those of them settled by
    -- reaching their most analyses without a consensus. Racks settle at their
    -- first analysis when the job wants one, so for such a job the first is
    -- racks_analyzed. The job is done once every rack is settled.
    racks_settled            BIGINT NOT NULL DEFAULT 0 CHECK (racks_settled >= 0),
    racks_without_consensus  BIGINT NOT NULL DEFAULT 0 CHECK (racks_without_consensus >= 0),
    -- Tasks created, and tasks that reached `completed`. The job list shows
    -- both for every job on the page, and counting them meant two COUNT(*)s
    -- over `tasks` per job per page view -- linear in each job's whole history,
    -- on the site's index. Same reasoning and same caveats as the two counters
    -- above: nothing decides anything from them, a purge zeroes them, and a
    -- partial restore recomputes them (RUNBOOK 2.3).
    tasks_total     BIGINT NOT NULL DEFAULT 0 CHECK (tasks_total >= 0),
    tasks_completed BIGINT NOT NULL DEFAULT 0 CHECK (tasks_completed >= 0),
    -- The move generations the job's accepted claims reported, added in the
    -- same submission and `UPDATE jobs` that credit the claim's contributor;
    -- a purge zeroes it and a delete takes it, as each gives the contributors
    -- theirs back. A partial restore recomputes it (RUNBOOK 2.3).
    movegens        BIGINT NOT NULL DEFAULT 0 CHECK (movegens >= 0),
    -- The compute time the same claims were credited with, in whole
    -- milliseconds: the Contributions page's site totals by job type, beside
    -- `movegens` and `tasks_completed`. Added, zeroed and taken with
    -- `movegens`.
    compute_ms      BIGINT NOT NULL DEFAULT 0 CHECK (compute_ms >= 0),
    -- When a result was last accepted for the job, to the minute (the
    -- submission that stores one sets it at most once a minute). The job
    -- list's `stalled` flag asks "none in a day"; answered from the claims, it
    -- joined every task of the job to the day's completions -- growing with
    -- the job's whole history, on every list view. Display only; a purge
    -- clears it, a partial restore recomputes it (RUNBOOK 2.3).
    last_completed_at TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    activated_at    TIMESTAMPTZ
);

-- Named, reusable player configurations.
-- Each row stores the MAGPIE argument values for one player slot.
-- Rows are immutable once any job references them (enforced at the application layer).
--
-- recorder_type (-r1 / -r2): 'best' = play the top-ranked move (fast, right for autoplay);
--   'equity' = record all moves within mmargin equity of best; 'all' = record every move.
--   For autoplay in birdtest, always use 'best'.
--
-- sort_strategy (-s1 / -s2): 'equity' = sort by equity (score + leave value) — standard static
--   player; 'score' = sort by raw score only, for a static player. A simming player's candidates
--   are the top plays by equity (autoplay generates them so whatever the row says), so a simmer
--   is always 'equity': config creation refuses 'score' for one. Both static and simming
--   players are valid in games/game_pairs jobs.
--
-- Simulation columns are all NULL for a static (no-sim) player.

CREATE TABLE player_configs (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name             TEXT NOT NULL UNIQUE CHECK (char_length(name) <= 100),  -- e.g. "simmer-NWL23-4ply"
    recorder_type    TEXT NOT NULL,         -- 'best' | 'equity' | 'all'  (-r1 / -r2)
    sort_strategy    TEXT NOT NULL,         -- 'equity' | 'score'  (-s1 / -s2)
    -- The files this player loads, pinned by content rather than named.
    --
    -- kwg_id and klv_id are NOT NULL: there is no job lexicon left to fall back
    -- to, and "NULL = the lexicon default" was exactly the implicit name-based
    -- resolution this design removes.
    --
    -- winpct_id stays nullable, but NULL now means "this player never loads a
    -- win% model" -- true of every static player, since MAGPIE only reads one
    -- through config_load_win_pcts. Validated against the sim columns at job
    -- creation: a simming player must have one, a static player must not.
    kwg_id           UUID NOT NULL REFERENCES input_data(id),   -- (-l1 / -l2)
    klv_id           UUID NOT NULL REFERENCES input_data(id),   -- (-k1 / -k2)
    winpct_id        UUID REFERENCES input_data(id),            -- (-winpct)
    -- The config this one was cloned from, for a data update. Ratings do NOT
    -- carry over -- a clone is a new player config, so it enters a rating pool
    -- with no games and no rating until it plays -- so the UI must show where a
    -- config with no history came from.
    cloned_from_id   UUID REFERENCES player_configs(id),
    -- Every setting a task request states is stated here, never NULL for
    -- "MAGPIE's default": creation writes MAGPIE's value into the row
    -- (backend/src/magpie_defaults.rs), so a config plays the same on every
    -- MAGPIE release, and MAGPIE refuses a request that leaves one out. The
    -- exception is a static player's simulation settings, which are NULL
    -- because nothing reads them; the CHECK at the end of the table holds the
    -- two sets apart.
    --
    -- Two pairs of "how much to compute" / "how much to report". MAGPIE
    -- generates plays and plies, then displays a subset of each; birdtest
    -- stores exactly what is displayed.
    num_plies          INT NOT NULL CHECK (num_plies >= 0),           -- plies to simulate; 0 is static (-pl1 / -pl2)
    num_plies_recorded INT NOT NULL CHECK (num_plies_recorded >= 1),  -- plies to report (shplies)
    -- plays to generate and simulate (-np1 / -np2). Stated for a static player
    -- too: an opening-rack analysis sizes its move list from it.
    num_plays          INT NOT NULL CHECK (num_plays >= 1),
    -- plays to report (maxnumdplays). Required: "keep everything" is unbounded
    -- per position, and the worker and the server must agree on the number.
    num_plays_recorded INT NOT NULL CHECK (num_plays_recorded >= 1),
    -- Simulation parameters (all NULL for a static player, all set for a simmer)
    max_iterations   INT,                   -- -i1 / -i2
    stopping_pct     DOUBLE PRECISION,      -- -sc1 / -sc2 (0–100)
    use_inference    BOOLEAN,               -- -si1 / -si2
    time_limit_secs  INT,                   -- -tl1 / -tl2
    -- The remaining MAGPIE options that can affect how a player plays.
    -- Exhaustive on purpose: MAGPIE takes nothing a request leaves out from its
    -- own defaults, so a setting missing here is a task no worker will run.
    use_wordmap          BOOLEAN NOT NULL,   -- -w1 / -w2
    use_rit               BOOLEAN NOT NULL,  -- rack info table            (-rit1 / -rit2)
    use_wit               BOOLEAN NOT NULL,  -- word info table (-wit1 / -wit2)
    -- More simulation parameters: NULL for a static player, set for a simmer.
    min_play_iterations   INT,               -- -mi1 / -mi2
    threshold             TEXT,              -- 'none' | 'gk16'            (-th1 / -th2)
    sampling_rule         TEXT,              -- 'round_robin' | 'top_two_ids' (-sa1 / -sa2)
    inference_margin       DOUBLE PRECISION, -- -im1 / -im2
    utility_w_winpct       DOUBLE PRECISION, -- blended-utility weight on win%     (-uwin1 / -uwin2)
    utility_w_spread       DOUBLE PRECISION, -- blended-utility weight on spread   (-uspread1 / -uspread2)
    utility_spread_scale   DOUBLE PRECISION, -- blended-utility spread scale       (-uspreadscale1 / -uspreadscale2)
    -- Options that are one shared MAGPIE setting for the whole run rather
    -- than per-player. Stored here anyway (duplicated on both players'
    -- rows in a job, validated equal at job-creation time) so this table
    -- stays the single, exhaustive source of what a job asked MAGPIE for.
    -- The same holds of num_plays_recorded and num_plies_recorded above in a
    -- games or game-pairs job that captures positions: MAGPIE reads player
    -- 1's for both seats, so job creation requires the two to agree.
    movegen_margin         DOUBLE PRECISION NOT NULL, -- move-gen equity margin for 'equity' recording (-mmargin)
    -- Endgame and pre-endgame (PEG) solving, read only by games and game-pairs
    -- jobs: an opening rack never reaches a small bag, and a leave job's games
    -- end before one (job creation refuses a leave player that solves).
    --
    -- endgame_plies is the switch for both: 0 solves nothing, because PEG
    -- scores its emptier scenarios with endgame solves. Above 0 the player
    -- solves the endgame to that depth once the bag is empty, and runs PEG
    -- while the bag holds 1..peg_max_bag tiles (0 = no PEG). Neither has a time
    -- limit: the depth and the schedule bound the work, so a player is as
    -- strong on a slow machine as on a fast one. Like a simulation, a solve is
    -- multithreaded and so not reproducible run to run.
    endgame_plies          INT NOT NULL DEFAULT 0,    -- -eplies1 / -eplies2
    peg_max_bag            INT NOT NULL DEFAULT 0,    -- -pegbag1 / -pegbag2
    -- The PEG schedule: NULL when peg_max_bag is 0, all set when it is not.
    peg_stage_top_k        INT[],                     -- survivors per halving stage (-pegtopk1 / -pegtopk2)
    peg_scenario_stride    INT,                       -- 1 = full enumeration (-pegstride1 / -pegstride2)
    peg_opp_model          TEXT,                      -- 'rational' | 'pessimistic' (-pegpess1 / -pegpess2)
    peg_nested             BOOLEAN,                   -- nested lookahead (-pegnested1 / -pegnested2)
    -- The nested lookahead's knobs: set exactly when peg_nested is.
    peg_nested_cand_caps   INT[],                     -- per-level candidate caps (-pegncaps)
    peg_nested_max_depth   INT,                       -- nested pegs before a rollout (-pegndepth)
    peg_nested_strides     INT[],                     -- stride per inner bag size 1..4 (-pegnstrides)
    -- SET NULL, like jobs.created_by: a config outlives the admin who made it.
    created_by       UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- A player simulates exactly when it has plies (MAGPIE decides on
    -- num_plies > 0). A simmer states every simulation setting, including the
    -- win% model it loads; a static player states none, since nothing reads
    -- them and a request carrying them would suggest otherwise.
    CONSTRAINT player_configs_simulation_settings CHECK (
        (num_plies > 0
            AND winpct_id IS NOT NULL AND max_iterations IS NOT NULL
            AND stopping_pct IS NOT NULL AND use_inference IS NOT NULL
            AND time_limit_secs IS NOT NULL AND min_play_iterations IS NOT NULL
            AND threshold IS NOT NULL AND sampling_rule IS NOT NULL
            AND inference_margin IS NOT NULL AND utility_w_winpct IS NOT NULL
            AND utility_w_spread IS NOT NULL AND utility_spread_scale IS NOT NULL)
        OR
        (num_plies = 0
            AND winpct_id IS NULL AND max_iterations IS NULL
            AND stopping_pct IS NULL AND use_inference IS NULL
            AND time_limit_secs IS NULL AND min_play_iterations IS NULL
            AND threshold IS NULL AND sampling_rule IS NULL
            AND inference_margin IS NULL AND utility_w_winpct IS NULL
            AND utility_w_spread IS NULL AND utility_spread_scale IS NULL)
    ),
    -- The solver settings nest: PEG needs the endgame, the PEG schedule is
    -- stated exactly when PEG runs, and the nested knobs exactly when nested
    -- lookahead is on. MAGPIE refuses a request that breaks any of these.
    CONSTRAINT player_configs_solver_settings CHECK (
        endgame_plies BETWEEN 0 AND 25
        AND peg_max_bag BETWEEN 0 AND 4
        AND (endgame_plies > 0 OR peg_max_bag = 0)
        AND (
            (peg_max_bag = 0
                AND peg_stage_top_k IS NULL AND peg_scenario_stride IS NULL
                AND peg_opp_model IS NULL AND peg_nested IS NULL)
            OR
            (peg_max_bag > 0
                AND peg_stage_top_k IS NOT NULL AND peg_scenario_stride IS NOT NULL
                AND peg_opp_model IN ('rational', 'pessimistic')
                AND peg_nested IS NOT NULL)
        )
        AND (peg_nested IS TRUE) = (peg_nested_cand_caps IS NOT NULL)
        AND (peg_nested IS TRUE) = (peg_nested_max_depth IS NOT NULL)
        AND (peg_nested IS TRUE) = (peg_nested_strides IS NOT NULL)
    )
);

-- Per-job-type config tables (one row per job; replaces the config JSONB column)

CREATE TABLE job_opening_rack_config (
    job_id            UUID PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
    -- lexicon, variant and letter distribution live on the job now: the first
    -- on the player config, the other two on `jobs`.
    -- The player config used to analyze each rack (may be a simmer or static player).
    player_config_id  UUID NOT NULL REFERENCES player_configs(id),
    -- Racks handed out per task. One rack per task means one claim/submit round
    -- trip per rack, and the worker rate limit alone would then cap a worker at
    -- well under a rack per second against a space of millions.
    racks_per_batch   INT NOT NULL DEFAULT 500 CHECK (racks_per_batch >= 1),
    rack_size         INT NOT NULL DEFAULT 7 CHECK (rack_size BETWEEN 1 AND 7),
    -- Size of the rack space, computed at job creation. Tasks address ranges of
    -- it, so this is what tells the scheduler when the job is exhausted.
    total_racks       BIGINT NOT NULL CHECK (total_racks >= 0),
    -- Consensus: a rack is analysed until enough of its analyses agree on its
    -- best move, each analysis a task of its own. Its consensus is the share
    -- of its analyses whose rank-1 move is the most common rank-1 move. It is
    -- settled once it has at least `min_results_per_rack` analyses and its
    -- consensus is at least `consensus_pct`, or once it has
    -- `max_results_per_rack` analyses (settled without consensus), and not
    -- analysed again unless an admin changes these three (the only settings a
    -- job's config may change after creation), which restates every rack.
    -- One and one is one analysis per rack, which is what a static player
    -- gets: its analyses are deterministic and always agree.
    consensus_pct          DOUBLE PRECISION NOT NULL DEFAULT 100
                           CHECK (consensus_pct > 50 AND consensus_pct <= 100),
    min_results_per_rack   INT NOT NULL DEFAULT 1 CHECK (min_results_per_rack >= 1),
    max_results_per_rack   INT NOT NULL DEFAULT 1
                           CHECK (max_results_per_rack >= min_results_per_rack
                                  AND max_results_per_rack <= 100)
);

CREATE TABLE job_game_config (
    job_id              UUID PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
    -- lexicon, variant and letter distribution live on the job now: the first
    -- on the player configs, the other two on `jobs`.
    player1_config_id   UUID NOT NULL REFERENCES player_configs(id),
    player2_config_id   UUID NOT NULL REFERENCES player_configs(id),
    -- Even, as job creation requires: MAGPIE alternates the first mover
    -- within a task, from player 1, so an odd batch gives player 1 the first
    -- move in more of the job's games (KL-87). The default is the least such
    -- batch, so a row written without one is not an odd one.
    games_per_batch     INT NOT NULL DEFAULT 2,
    -- Whether the job runs the match test (stats/match_test.rs): a confidence
    -- interval for player 1's score that stays valid however often it is
    -- checked, the job stopping once it excludes an even score. Off, it plays
    -- max_games and stops, and min_games and confidence_pct are stored at
    -- their defaults and read by nothing. Off by default: a job that only
    -- wants the games played should not be stopped early by a test it did not
    -- ask for.
    test_enabled        BOOLEAN NOT NULL DEFAULT FALSE,
    -- Two finish conditions: a decision (looked for from min_games on) OR reaching max_games.
    min_games           INT NOT NULL,   -- the test is not acted on before this many games are complete
    max_games           INT NOT NULL,   -- job auto-completes at this count regardless of the test
    -- How confident a decision is: the interval's coverage, in percent.
    confidence_pct      DOUBLE PRECISION NOT NULL DEFAULT 95
                        CHECK (confidence_pct > 50 AND confidence_pct < 100),
    -- Keep the position analyses the worker produces while playing. A worker
    -- analyses a position every turn regardless; this decides whether those are
    -- recorded. Off by default: at ~22.5 turns a game it roughly doubles the
    -- rows a job produces.
    capture_positions   BOOLEAN NOT NULL DEFAULT FALSE,
    -- How MAGPIE spends its threads on a task (MULTI_THREADING_MODE): 'igp'
    -- gives them all to one game at a time, inside its simulation, which makes
    -- an iteration-bounded simulation reproducible; 'pgp' plays games in
    -- parallel, a thread each. Matters only when a player simulates; a job of
    -- static players runs alike in either. Stated on every request.
    threading_mode      TEXT NOT NULL DEFAULT 'igp' CHECK (threading_mode IN ('igp', 'pgp')),
    -- What `validate_job_body` requires, held here too for a row written any
    -- other way (a script, a fixture, a restore). The stopping rule reads the
    -- counts as unsigned: a negative max_games was a cap no job reached, so it
    -- ran for ever, and a batch of 0 had every claim play the same seed.
    CONSTRAINT job_game_config_counts CHECK (
        games_per_batch >= 1 AND max_games >= 1 AND min_games >= 0
        AND (NOT test_enabled OR min_games BETWEEN 1 AND max_games)
    )
);

CREATE TABLE job_game_pair_config (
    job_id              UUID PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
    -- lexicon, variant and letter distribution live on the job now: the first
    -- on the player configs, the other two on `jobs`.
    player1_config_id   UUID NOT NULL REFERENCES player_configs(id),
    player2_config_id   UUID NOT NULL REFERENCES player_configs(id),
    pairs_per_batch     INT NOT NULL DEFAULT 1,
    -- As on job_game_config: off, the job plays max_pairs and stops.
    test_enabled        BOOLEAN NOT NULL DEFAULT FALSE,
    min_pairs           INT NOT NULL,
    max_pairs           INT NOT NULL,
    confidence_pct      DOUBLE PRECISION NOT NULL DEFAULT 95
                        CHECK (confidence_pct > 50 AND confidence_pct < 100),
    -- Keep the position analyses the worker produces while playing. A worker
    -- analyses a position every turn regardless; this decides whether those are
    -- recorded. Off by default: at ~22.5 turns a game it roughly doubles the
    -- rows a job produces.
    capture_positions   BOOLEAN NOT NULL DEFAULT FALSE,
    -- With capture on, keep only each pair's first divergence: both games'
    -- positions at the first turn the two games play different moves, and
    -- nothing from a pair played identically. Before that turn the two games
    -- are the same game; after it they are two different ones, and the turn
    -- itself is where the players disagree.
    capture_first_divergence BOOLEAN NOT NULL DEFAULT FALSE,
    -- As on job_game_config.
    threading_mode      TEXT NOT NULL DEFAULT 'igp' CHECK (threading_mode IN ('igp', 'pgp')),
    CONSTRAINT job_game_pair_config_divergence_needs_capture
        CHECK (capture_positions OR NOT capture_first_divergence),
    -- As job_game_config_counts, in pairs.
    CONSTRAINT job_game_pair_config_counts CHECK (
        pairs_per_batch >= 1 AND max_pairs >= 1 AND min_pairs >= 0
        AND (NOT test_enabled OR min_pairs BETWEEN 1 AND max_pairs)
    )
);

CREATE TABLE job_leave_config (
    job_id         UUID PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
    -- The player the leave-generating bot plays as, in both seats: its lexicon
    -- and wordmap setting are the job's. Its leaves are never loaded -- every
    -- generation's come from the server-built KLV artifact, and generation 1's
    -- is a zeroed one -- and nor is its win% model, since the bot plays
    -- statically. So the job's complete data requirement is this player's kwg
    -- plus the job's letterdist_id and layout_id. Job creation refuses a player
    -- that simulates, sorts on anything but equity, or asks for a rack info
    -- table: leave values are measured from static equity play.
    player_config_id UUID NOT NULL REFERENCES player_configs(id),
    -- Games each leave-gen task plays over its forced-rack subset.
    num_iterations INT NOT NULL,
    -- The occurrence target every rack must reach before a generation closes,
    -- one per generation: element g (1-based, as generations are numbered) is
    -- generation g's, and the array's length is how many generations the job
    -- runs before it is complete. MAGPIE's `leavegen 100,200,500,…` shape.
    -- `<= ALL` over an array holding a NULL is NULL, which a CHECK passes, so
    -- the NULL test is separate; and it is indexed by generation, so flat and
    -- starting at 1.
    target_rack_counts INT[] NOT NULL CHECK (
        array_ndims(target_rack_counts) = 1
        AND array_lower(target_rack_counts, 1) = 1
        AND cardinality(target_rack_counts) >= 1
        AND 1 <= ALL (target_rack_counts)
        AND array_position(target_rack_counts, NULL) IS NULL
    ),
    -- Size of the forced-rack subset handed to a single task.
    racks_per_task    INT NOT NULL CHECK (racks_per_task >= 1)
);

-- Exports
--
-- A job's results, as one gzipped NDJSON object in the artifact store. Any job
-- can be exported; is_final says whether this is the completed job's corpus,
-- which is immutable and so built once and reused (the results stream
-- redirects to it), or a snapshot of a job still taking results, which is
-- offered as a download labelled with its time and never served in the
-- finished job's place.
--
-- Shaped like input_data_imports, and for the same reason: a long operation an
-- admin starts, polls, and then acts on. birdtest runs as a single instance, so
-- the task needs no lease and startup may fail any row still 'running'.
CREATE TABLE job_exports (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id        UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    state         TEXT NOT NULL DEFAULT 'running'
                  CHECK (state IN ('running', 'ready', 'failed')),
    -- NULL until the upload completes: the row exists from the moment the
    -- background task is spawned.
    artifact_key  TEXT,
    bytes         BIGINT,
    sha256        TEXT,
    -- Rows written. Recorded so a later mismatch against the job is visible
    -- rather than silent -- the same reason the KLV artifacts carry a digest.
    row_count     BIGINT,
    -- A games or game-pairs job that captured positions gets a second object
    -- beside its results: the positions, each with its ranked moves, in the
    -- shape an opening-rack export's lines have. All four are NULL for every
    -- other export. A second artifact rather than tagged lines in the first,
    -- so a consumer of a games job's results never meets a line of another
    -- kind.
    positions_artifact_key TEXT,
    positions_bytes        BIGINT,
    positions_sha256       TEXT,
    positions_row_count    BIGINT,
    -- TRUE when the snapshot the export was read in saw the job completed,
    -- with no claim still open and nothing staged: its final corpus. Decided
    -- inside that snapshot (exports::read_snapshot), not when the export was
    -- requested -- a job exported mid-run can complete before its rows are
    -- read, with its last results still landing. FALSE until built, so an
    -- unfinished or unmarked row is never taken for the final one.
    is_final      BOOLEAN NOT NULL DEFAULT FALSE,
    -- When that snapshot was taken, for the page's "Snapshot as of …".
    snapshot_at   TIMESTAMPTZ,
    error         TEXT,
    requested_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    requested_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at  TIMESTAMPTZ
);

-- The newest ready export for a job, which is what a download resolves to
-- (the newest final one, for the results stream).
CREATE INDEX job_exports_job_idx ON job_exports (job_id, requested_at DESC);

-- One export of a job at a time. Only the page's disabled button stopped a
-- second, and each holds a pool connection for the whole corpus read.
CREATE UNIQUE INDEX job_exports_one_running_idx ON job_exports (job_id) WHERE state = 'running';

-- Tasks

CREATE TYPE task_state AS ENUM ('available', 'claimed', 'completed');

CREATE TABLE tasks (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id               UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    -- The seed the task's games are played from. Every job type plays games,
    -- so every task has one, stated on its request: games and game pairs seed
    -- their batch from it and step by one per game; an opening-rack task's is
    -- the index of its first rack in the job's rack space, and rack i of the
    -- batch is analysed from seed + i; a leave-generation task's is drawn
    -- when the task is created. Stored as signed int64; interpreted as uint64
    -- at the application layer. For games, pairs and opening racks it is also
    -- the cursor that tiles the job's space, which is what the unique index
    -- below serves.
    seed                 BIGINT NOT NULL,
    state                task_state NOT NULL DEFAULT 'available',
    -- Denormalized counters used by SKIP LOCKED selection; avoids per-candidate
    -- join/aggregate. A task has one slot: it is claimed by one worker at a
    -- time and completed by its one accepted result, so each is 0 or 1 -- and
    -- held to it, so a counter that drifted fails the statement that drifted
    -- it rather than, days later, leaving a job that quietly stops dispatching.
    accepted_count       INT NOT NULL DEFAULT 0 CHECK (accepted_count IN (0, 1)),
    active_claim_count   INT NOT NULL DEFAULT 0 CHECK (active_claim_count IN (0, 1)),
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at         TIMESTAMPTZ
);

-- Prevent two tasks of one job from playing the same seed: for games, pairs
-- and opening racks that is two workers racing for the same slice of the
-- space, for leave generation a collision between two randomly drawn seeds
-- (which the claim path retries).
CREATE UNIQUE INDEX tasks_seed_unique_idx ON tasks (job_id, seed);

-- Partial indexes to support efficient SKIP LOCKED task selection and timeout
-- reclamation.
--
-- The queue index carries `created_at` rather than `state`, which the partial
-- predicate already fixes: claim-time selection takes the *oldest* available
-- task of a job (`registry::next_available`), so with `state` in the key the
-- planner had to read every available task of the job and sort it.
CREATE INDEX tasks_queue_idx   ON tasks (job_id, created_at) WHERE state = 'available';

-- Individual claims (one row per worker claim: a task's lapsed and declined
-- claims, then the one that completed it)
--
-- claimed_by_user_id carries no ON DELETE clause because a user row is never
-- deleted: account deletion anonymizes it in place (users.deleted_at, and a
-- tombstone username and email) and leaves these rows exactly where they are.
-- Removing them instead would take with them the position analyses keyed to
-- those claims -- an opening-rack batch's, and the in-game positions a games
-- job captured -- and leave-generation occurrences that were folded into
-- per-rack totals and cannot be subtracted back out. See
-- routes::admin::delete_user.

-- 'declined' is distinct from 'abandoned': one is a worker saying "I cannot do
-- this", the other is a claim that lapsed. Only the first is diagnostic.
CREATE TYPE claim_state AS ENUM ('claimed', 'completed', 'abandoned', 'declined');

-- An abandoned claim that ran past its deadline with its worker alive
-- (`task_claims.overrun`): `pending` until it is counted against its job,
-- then `counted`.
CREATE TYPE claim_overrun AS ENUM ('pending', 'counted');

CREATE TABLE task_claims (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    task_id              UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    -- The task's job, copied at claim time and never changed (a task never
    -- moves between jobs). Without it "this contributor's claims in this job"
    -- -- the public feed's `?worker=` -- meant every claim the contributor ever
    -- made, or every task of the job: seconds for a heavy contributor. With it,
    -- one range of the identity indexes below. No foreign key of its own:
    -- claims go with their task, and their task with its job.
    job_id               UUID NOT NULL,
    claim_token          UUID NOT NULL,
    state                claim_state NOT NULL DEFAULT 'claimed',
    claimed_by_user_id   UUID REFERENCES users(id),
    claimed_by_anon_uuid UUID REFERENCES anonymous_workers(uuid),
    claimed_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- When the task must be done by: `claimed_at` plus `settings.max_task_seconds`
    -- as it stood when the claim was made, which the assignment told the
    -- worker. Past it and a minute's grace the claim lapses whether or not its
    -- worker still heartbeats, and a result for it is refused: a task whose
    -- one unit outlasts the limit (a deep-sim game pair) would otherwise hold
    -- its slot for as long as its worker lived. The claim path always sets
    -- it; the default, the limit's own default, is for a row written any
    -- other way (a script, a test's fixture).
    deadline_at          TIMESTAMPTZ NOT NULL DEFAULT now() + interval '1 hour',
    -- Set on a claim taken back past its deadline while its worker was still
    -- alive: reclamation found it heartbeating within the heartbeat timeout
    -- of the moment it lapsed (deadline plus grace), or the worker submitted
    -- a result for it too late. NULL for every other claim -- a lapse whose
    -- worker had gone silent is what a dead worker leaves, and says nothing
    -- about the job. Such a claim is a task that hit the time limit as surely
    -- as a `time_limit` decline: a solve or a build that overruns MAGPIE's
    -- stop declines after the claim has lapsed, and that decline is a `404`.
    -- Counted against the job (`jobs.time_limit_declines`, `time_limit_streak`)
    -- by whoever next holds the job's row for it -- the job's next claim,
    -- a decline or a late result -- since reclamation runs over many jobs in
    -- one statement on the claim path and takes no job's row: `pending` until
    -- then, `counted` after.
    overrun              claim_overrun,
    last_heartbeat_at    TIMESTAMPTZ,
    completed_at         TIMESTAMPTZ,
    -- The move generations this claim's accepted result reported (the
    -- submission's `movegens`), written when it completes (zero until then,
    -- and for a claim that never does). Its contributor's running total adds
    -- this, so a purge can give back exactly what it added by summing the
    -- claims it is about to delete, without reading the results. Every
    -- claim's own, not the task's first result's.
    movegens             BIGINT NOT NULL DEFAULT 0 CHECK (movegens >= 0),
    -- As reported at claim time. What the fleet is actually running, which is
    -- the evidence for raising a job's floor.
    magpie_version       TEXT,
    CONSTRAINT claim_has_single_owner CHECK (
        (claimed_by_user_id IS NOT NULL)::int + (claimed_by_anon_uuid IS NOT NULL)::int = 1
    )
)
-- Room on each page for a claim's heartbeats. A heartbeat changes only
-- `last_heartbeat_at`, which no index covers, so it can be a HOT update -- an
-- in-page rewrite that touches none of this table's nine indexes -- but only
-- if the row's page has space, and claims are appended, so at the default
-- fillfactor of 100 a claim's first heartbeat found its page full and wrote a
-- new entry into every index: two a minute for every claim in flight.
WITH (fillfactor = 85);

-- A task has one slot: one claim holds it, or one claim completed it, never
-- two of either and never both -- a completed task is not handed out again.
-- The claim path keeps it so (a task is selected only while `available`,
-- under its job's dispatch lock and its own row lock), and the counters on
-- `tasks` and the job's `tasks_completed` count on it; this makes a breach,
-- by any two identities, fail at the statement rather than surface as
-- a counter gone wrong. A task has any number of lapsed and declined claims,
-- so those are outside it: a worker that declined a task for missing data and
-- then fixed its data has to be able to claim that task again.
--
-- It replaces two per-identity indexes from when a task had several slots and
-- the thing to stop was one identity taking two of them; one slot covers that
-- case too.
CREATE UNIQUE INDEX task_claims_one_slot_idx
    ON task_claims (task_id) WHERE state IN ('claimed', 'completed');

-- What a worker said it was missing when it declined. The server records gaps
-- for humans; it does not route on them (the client sends its own unsupported
-- set with each claim, which makes that state self-correcting).
CREATE TABLE worker_data_gaps (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id       UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    claim_id     UUID NOT NULL REFERENCES task_claims(id) ON DELETE CASCADE,
    role         TEXT NOT NULL,
    name         TEXT NOT NULL,
    expected     TEXT NOT NULL,
    actual       TEXT,                    -- NULL = file absent
    reported_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- (job_id, reported_at): the job list asks whether a job has a gap reported in
-- the last 24 hours, and the admin view groups a job's gaps, so both start at
-- the job. Ordered by time within it, the first question stops at the newest
-- row rather than walking every gap the job ever had.
CREATE INDEX worker_data_gaps_job_idx ON worker_data_gaps (job_id, reported_at DESC);
-- The cascade from a claim. A purge deletes every claim of a job, and without
-- this the lookup was a sequential scan of this table per deleted claim.
CREATE INDEX worker_data_gaps_claim_idx ON worker_data_gaps (claim_id);

-- Task requests (one-to-one with tasks; inserted in the same transaction as the task row)

-- One row per task, covering a contiguous range of the rack space. The racks
-- themselves are not stored: they are unranked from `rack_start` on demand,
-- which is what lets a job over millions of racks be created in constant time.
--
-- Named for opening racks rather than positions: the request is specifically a
-- set of opening racks, and there is no general position-analysis job. What
-- comes back from analyzing one *is* a position analysis, which is why the
-- record tables below keep that name.
CREATE TABLE opening_rack_requests (
    task_id           UUID PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
    -- No lexicon column: the player config carries it.
    variant           TEXT NOT NULL,
    letter_distribution TEXT NOT NULL,
    -- The job's pinned layout, by name. Stated on the request for the same
    -- reason the distribution is: a worker must play on the board the job
    -- pins, not on whatever board its own settings last loaded.
    board_layout      TEXT NOT NULL,
    -- Index of the first rack in this batch, and how many it covers. The final
    -- batch of a job may be short. A task reissuing racks a consensus still
    -- wants (`racks` below) has its seed here instead: the cursor past the end
    -- of the rack space, so rack i of it is analysed from a seed no earlier
    -- analysis used.
    rack_start        BIGINT NOT NULL CHECK (rack_start >= 0),
    rack_count        INT NOT NULL CHECK (rack_count >= 1),
    -- The racks themselves, for a task reissuing racks a consensus still
    -- wants: they are scattered over the space rather than a range of it.
    -- NULL for a task covering a range, which is every task of a job wanting
    -- one analysis per rack.
    racks             TEXT[] CHECK (racks IS NULL OR cardinality(racks) = rack_count),
    player_config_id  UUID NOT NULL REFERENCES player_configs(id)
);

CREATE TABLE game_requests (
    task_id           UUID PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
    -- No lexicon column: each player config carries its own.
    variant           TEXT NOT NULL,
    letter_distribution TEXT NOT NULL,
    board_layout      TEXT NOT NULL,
    -- Denormalized from the job config, like everything else here, so the
    -- request a re-dispatched task replays is exactly the one it was given.
    capture_positions BOOLEAN NOT NULL DEFAULT FALSE,
    -- Game pairs only: of the captured positions, keep only each pair's first
    -- divergence (job_game_pair_config.capture_first_divergence).
    capture_first_divergence BOOLEAN NOT NULL DEFAULT FALSE,
    -- seed is also stored on the tasks row; duplicated here for convenience when reading the full request.
    seed              BIGINT NOT NULL,
    num_games         INT NOT NULL DEFAULT 1,
    player1_config_id UUID NOT NULL REFERENCES player_configs(id),
    player2_config_id UUID NOT NULL REFERENCES player_configs(id)
);

CREATE TABLE leave_requests (
    task_id             UUID PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
    -- No lexicon or wordmap column: the player config carries both.
    variant             TEXT NOT NULL,
    letter_distribution TEXT NOT NULL,
    board_layout        TEXT NOT NULL,
    generation          INT NOT NULL,
    -- The seed the task's games are played from, chosen when the task is
    -- created, so a reissued task replays it. Stored as signed int64 and
    -- interpreted as uint64, like tasks.seed. Without it a worker seeded from
    -- its own process state, which differs by machine.
    seed                BIGINT NOT NULL,
    forced_racks        TEXT[] NOT NULL,   -- the rack subset this task must force (passed to MAGPIE's rack_list_create)
    num_games           INT NOT NULL,      -- denormalized from job_leave_config.num_iterations
    -- Combined KLV from the previous generation. Never NULL: generation 1 reads
    -- the server-built zeroed KLV at generation-0, so every generation fetches
    -- its leaves the same way and the client has no first-generation branch.
    previous_artifact_key TEXT NOT NULL,
    player_config_id    UUID NOT NULL REFERENCES player_configs(id)
);

-- An opening-rack job's racks: how many analyses each has, its most common
-- rank-1 move and how many analyses ranked it first, and whether it is
-- settled (see job_opening_rack_config). Every opening-rack job keeps them,
-- even one wanting one analysis per rack, which settles each rack at its
-- first: an admin may change its consensus settings later, and its reissues
-- then start from these rows. A rack has a row from its first analysis; the
-- racks still to be reissued are its unsettled rows, fewest analyses first.
CREATE TABLE opening_rack_progress (
    job_id     UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    rack       TEXT NOT NULL,
    results    INT NOT NULL CHECK (results >= 1),
    top_move   TEXT NOT NULL,
    top_count  INT NOT NULL CHECK (top_count BETWEEN 1 AND results),
    settled    BOOLEAN NOT NULL,
    -- Settled by reaching the job's most analyses, its analyses still split.
    without_consensus BOOLEAN NOT NULL CHECK (settled OR NOT without_consensus),
    PRIMARY KEY (job_id, rack)
);

-- What a reissue picks from: a job's unsettled racks, fewest analyses first.
CREATE INDEX opening_rack_progress_unsettled_idx
    ON opening_rack_progress (job_id, results, rack) WHERE NOT settled;

-- Per-rack occurrence progress for each generation of a leave-gen job, one row per
-- full 7-tile rack the distribution can draw (3,199,724 for English), seeded at zero when
-- the generation opens. Updated by `leave_gen::merge_staged`, which folds the accepted
-- results staged in `leave_rack_staging` below -- not by each submission; see there for why.
-- Drives claim-time rack selection and generation-transition detection (all racks >= target,
-- nothing in flight, nothing staged). Leave values are derived from these full-rack means as
-- MAGPIE's rack_list_write_to_klv does.
CREATE TABLE leave_rack_progress (
    job_id           UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    generation       INT NOT NULL,
    rack             TEXT NOT NULL,
    occurrence_count BIGINT NOT NULL DEFAULT 0,
    equity_sum       DOUBLE PRECISION NOT NULL DEFAULT 0,  -- occurrence_count-weighted; equity_sum / occurrence_count = mean
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (job_id, generation, rack)
);

-- Accepted leave results waiting to be folded into `leave_rack_progress`: one
-- row per accepted task, its racks, counts and equity sums as three parallel
-- arrays (compressed and stored out of line, so a 200,000-rack submission is a
-- few megabytes and one insert).
--
-- A submission used to fold itself: an UPDATE over every rack its games drew,
-- scattered uniformly across the 3.2 million rows above. Measured, that was
-- 2.5-5.5 s inside the transaction the worker waits on, 147-409 MB of WAL per
-- fold (the first touch of a page after a checkpoint writes the whole page),
-- and no HOT update ever, because `occurrence_count` is indexed. Nothing needs
-- the per-rack totals that promptly -- selection needs them roughly, closing a
-- generation and building its KLV need them exactly but only then -- so a
-- submission appends here and `leave_gen::merge_staged` folds everything
-- staged in one pass: every half hour, when a claim finds the generation
-- nearly done, and always before a generation closes.
--
-- `task_id` deliberately has no foreign key. A purge deletes every task of a
-- job, Postgres runs a cascade once per deleted row, and there is no index on
-- this column to serve one -- the shape that made two earlier purges scan a
-- table per task. A purge deletes a job's staged rows itself, by `job_id`;
-- deleting the job cascades through the index below.
CREATE TABLE leave_rack_staging (
    id          BIGSERIAL PRIMARY KEY,
    job_id      UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    generation  INT NOT NULL,
    -- Whose result this is. Claim-time selection holds the racks this task
    -- forced out of play until the merge says what they reached.
    task_id     UUID NOT NULL,
    racks       TEXT[] NOT NULL,
    counts      BIGINT[] NOT NULL,
    equity_sums DOUBLE PRECISION[] NOT NULL,  -- count * mean, per rack
    staged_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT leave_rack_staging_parallel_arrays CHECK (
        cardinality(racks) = cardinality(counts)
        AND cardinality(racks) = cardinality(equity_sums)
    )
);
-- What a merge takes, what selection excludes, and the cascade from `jobs`.
CREATE INDEX leave_rack_staging_job_idx ON leave_rack_staging (job_id, generation);

-- One row per generation a leave job has opened: what the dashboard shows.
--
-- `tasks_completed` and `games_played` are live, bumped in the submit
-- transaction. The rest is a summary of `leave_rack_progress` as of
-- `merged_at`, recomputed by each merge while the rows are warm; counted on
-- read it was a pass over the whole generation on every detail view and every
-- live push. Seeded with `racks_total` when the generation's universe is
-- written, so there is a denominator before the first merge.
CREATE TABLE leave_generation_progress (
    job_id          UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    generation      INT NOT NULL,
    tasks_completed BIGINT NOT NULL DEFAULT 0 CHECK (tasks_completed >= 0),
    games_played    BIGINT NOT NULL DEFAULT 0 CHECK (games_played >= 0),
    racks_total     BIGINT NOT NULL DEFAULT 0,
    racks_at_target BIGINT NOT NULL DEFAULT 0,
    -- The rack furthest from target, and its count. NULL until a merge.
    min_rack        TEXT,
    min_rack_count  BIGINT,
    merged_at       TIMESTAMPTZ,
    PRIMARY KEY (job_id, generation)
);

-- Where a leave generation's selection sweep has got to: the last rack handed
-- out in the lap under way. A row with a rack exists exactly while a lap has
-- racks left to hand out: the task that takes the last of them deletes it.
--
-- While many racks are below target, racks are handed out in primary-key order
-- from this cursor rather than lowest count first. Everything behind the cursor
-- has been handed out this lap and nothing ahead of it has -- a lap only starts
-- with no claim of the generation in flight and nothing staged -- so a claim
-- needs no list of what is out, and selection costs the same however much is
-- staged. Lowest-count-first could not say that: between merges the racks of
-- every staged result are exactly the lowest, and each claim hashed and
-- skipped all of them inside the job's dispatch lock.
--
-- Once few racks are below target the generation turns, at a lap's boundary,
-- to lowest count first, and stays there: a row with a NULL cursor_rack
-- (`leave_gen::at_boundary`).
--
-- Read and written only under that lock. A row that goes missing (a purge, a
-- partial restore) is a lap not started, which waits for what is in flight and
-- staged before it selects anything; nothing is handed out twice.
CREATE TABLE leave_selection_cursors (
    job_id      UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    generation  INT NOT NULL,
    cursor_rack TEXT,
    PRIMARY KEY (job_id, generation)
);

-- Task records (one per accepted claim, keyed by task_claim_id)
-- task_id is denormalized here for efficient job-results queries without joining through task_claims.

-- One analysed position per row, whatever produced it.
--
-- Opening rack jobs write one per rack. Games and game-pairs jobs write one per
-- turn when `capture_positions` is on: a worker analyses a position on every
-- turn anyway, and keeping those makes a job a corpus of analysed positions as
-- well as a measurement of strength.
--
-- The request that produced these is job-type-specific -- opening_rack_requests
-- or game_requests -- but what comes back is a position analysis either way,
-- which is why both share this table.
CREATE TABLE position_analysis_records (
    -- Surrogate, because the natural key differs by source: an opening rack is
    -- unique per (claim, rack), while an in-game position recurs at the same
    -- rack across turns and games.
    id              BIGSERIAL PRIMARY KEY,
    task_claim_id   UUID NOT NULL REFERENCES task_claims(id) ON DELETE CASCADE,
    -- The task, for the in-game dedup key below. Deliberately not a foreign
    -- key: a record goes with its claim (above) or its job (below), both of
    -- which cascade through their own indexes, so a cascade from the task
    -- only added an index entry and a foreign-key probe per record -- the same
    -- pattern `position_analysis_moves.task_id` and the staging table's were
    -- removed for.
    task_id         UUID NOT NULL,
    -- Denormalized from the task. Every read of a job's records -- the public
    -- results feed, the rack lookup, the admin stream, the export -- filtered
    -- on the job and could only reach it through `tasks`, which put the filter
    -- on the far side of a join from the sort and made the whole job the unit
    -- of work. With the column here they are index scans.
    job_id          UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    rack            TEXT NOT NULL,
    -- CGP of the position analysed. NULL for an opening rack, where the board
    -- is empty by definition and the rack is the whole position.
    position        TEXT,
    -- In-game positions only: which game of the batch, and which turn of it.
    game_index      SMALLINT,
    turn_number     SMALLINT,
    -- The move played on the previous turn of this game, and its score.
    -- NULL for turn 0 of a game (nothing preceded it) and for opening racks.
    previous_move       TEXT,
    previous_move_score INT,
    -- The move played from this position, and its score: the one chosen this
    -- turn, which need not be the top-ranked move below (a simmer's pick, or
    -- a solver's), and which no later row holds when only a pair's first
    -- divergence is kept. Every in-game position has it; an opening rack,
    -- from which nothing is played, never does.
    played_move         TEXT,
    played_move_score   INT,
    -- How the move played from this position was chosen: by static equity, a
    -- simulation, a pre-endgame solve or an endgame solve. Decides which of
    -- its moves' statistics are set: win_percentage and per-ply rows for a
    -- simulation, mean_spread and fidelity_plies for a solve. An opening rack
    -- is 'static' or 'sim'.
    analysis            TEXT NOT NULL CHECK (analysis IN ('static', 'sim', 'peg', 'endgame')),
    -- How many moves the worker ranked, which is generally far more than the
    -- stored moves. The one thing about the analysis those cannot tell you,
    -- since they are truncated.
    --
    -- The best move, its score and its equity are deliberately *not* stored
    -- here: they are the rank 1 row in position_analysis_moves, and duplicating
    -- them is a second copy to keep consistent for no gain.
    num_moves       INT NOT NULL,
    submitted_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT position_analysis_in_game_together CHECK (
        (game_index IS NULL AND turn_number IS NULL)
        OR (game_index IS NOT NULL AND turn_number IS NOT NULL)
    ),
    CONSTRAINT position_analysis_played_in_game CHECK (
        (played_move IS NULL) = (game_index IS NULL)
        AND (played_move_score IS NULL) = (played_move IS NULL)
    )
);

-- One position per turn of a task's game. It is also how a random saved
-- position is drawn
-- (`/api/jobs/:id/positions/random`): a random turn of one task's games, a
-- few hundred entries at most, where `ORDER BY random()` read the whole job.
CREATE UNIQUE INDEX position_analysis_records_in_game_idx
    ON position_analysis_records (task_id, game_index, turn_number)
    WHERE game_index IS NOT NULL;

-- Opening racks keep their natural key: one analysis per rack per claim. A
-- rack an opening-rack job analyses more than once, to reach a consensus,
-- is in several tasks, each with its own claim.
CREATE UNIQUE INDEX position_analysis_records_rack_idx
    ON position_analysis_records (task_claim_id, rack)
    WHERE game_index IS NULL;

-- The public results feed, which is newest-first within a job and paginated by
-- keyset. `id` is in the index because it is the cursor's tiebreaker:
-- `submitted_at` defaults to now(), which is transaction time, so every record
-- of one batch shares it exactly and it is not a key on its own.
CREATE INDEX position_analysis_records_feed_idx
    ON position_analysis_records (job_id, submitted_at DESC, id DESC);

-- The rack lookup (`?rack=`), which is the branch the site actually uses. One
-- probe rather than one per task of the job. Opening racks only: an
-- incidentally-captured in-game position is not an opening-rack analysis.
CREATE INDEX position_analysis_records_job_rack_idx
    ON position_analysis_records (job_id, rack) WHERE game_index IS NULL;

-- The positions search (`/api/jobs/:id/positions?rack=`): a game's positions
-- with one rack, newest first. In-game positions only, so the index costs a
-- job nothing unless it captures.
CREATE INDEX position_analysis_records_game_rack_idx
    ON position_analysis_records (job_id, rack, id) WHERE game_index IS NOT NULL;

-- The cascade from a claim (`task_claim_id ... ON DELETE CASCADE`). The
-- partial unique index on (task_claim_id, rack) above cannot serve it: a plain
-- equality on task_claim_id does not imply `game_index IS NULL`, so the
-- planner never uses a partial index for it. Without this a purge or a job
-- delete -- which removes every claim of the job, and Postgres runs the
-- cascade once per deleted row -- scanned this whole table once per claim: a
-- full English opening-rack job is ~6,400 claims over ~3.2 million records,
-- thousands of sequential scans inside one transaction holding the job's
-- dispatch lock and every open claim's row. One entry per record; the moves
-- below cascade from the record through their own index.
CREATE INDEX position_analysis_records_claim_idx
    ON position_analysis_records (task_claim_id);

-- The top `num_plays_recorded` moves per position, from the player config that
-- produced them. Storing every move the worker ranked would be untenable:
-- a job over the full English 7-tile space is roughly 3.2 million racks, and a
-- 40,000-pair job with capture on is 1.8 million positions.
CREATE TABLE position_analysis_moves (
    id              BIGSERIAL PRIMARY KEY,
    -- The record is the only parent. A `task_id` column used to sit here as
    -- well, with its own ON DELETE CASCADE, kept "for the cascade" after the
    -- job-wide aggregates that read it were removed. That cascade was the
    -- problem: the column had no index, so deleting a task -- which a purge or
    -- a job delete does once per task -- scanned this whole table to find the
    -- moves to cascade, and this is the largest table in the schema. A full
    -- English opening-rack job is some 6,400 tasks over 32 million move rows,
    -- which made its purge thousands of sequential scans of the table, hours
    -- inside one transaction holding the job's dispatch lock. The record's
    -- cascade already reaches every move through the index below.
    record_id       BIGINT NOT NULL REFERENCES position_analysis_records(id) ON DELETE CASCADE,
    rank            SMALLINT NOT NULL,
    move            TEXT NOT NULL,
    score           INT NOT NULL,
    equity          DOUBLE PRECISION NOT NULL,
    -- How many times a simulation played the move out (its share of the
    -- simulation's iterations, most going to the leaders). 0 for a move nothing
    -- simulated; NULL from a client that did not say.
    iterations      BIGINT CHECK (iterations >= 0),
    -- The win percentage: a simulation's, or a pre-endgame solve's over every
    -- way the bag can be drawn. NULL for a static or endgame analysis.
    win_percentage  DOUBLE PRECISION,
    -- Mean win%+spread blend in [0, 1] (see the player config's
    -- utility_w_winpct/utility_w_spread/utility_spread_scale), sometimes used
    -- to rank moves instead of equity or raw win percentage. NULL for a
    -- static player, same as win_percentage.
    blended_utility DOUBLE PRECISION,
    -- A pre-endgame or endgame solve's projected final spread for the mover,
    -- in points, and the endgame depth the move was ranked at (a PEG move's
    -- deepest tier; 0 is PEG's greedy seed). NULL for a static or simulated
    -- analysis.
    mean_spread     DOUBLE PRECISION,
    fidelity_plies  SMALLINT
);
-- Every read of a best move goes through its record: the results listing joins
-- `record_id` and filters `rank = 1`, and a rack lookup reads a record's whole
-- ranked list. There is deliberately no job-wide index on `(task_id) WHERE
-- rank = 1`: one existed for a dashboard aggregate over every best move of a
-- job, that aggregate is gone (the panel shows progress only), and the index
-- cost maintenance on every move insert into a table that runs to tens of
-- millions of rows.
CREATE INDEX position_analysis_moves_record_idx
    ON position_analysis_moves (record_id, rank);

-- Per-ply simulation stats for each candidate move. Only populated for simming
-- player configs; a static player has no per-ply statistics to record.
CREATE TABLE position_analysis_plies (
    move_id          BIGINT NOT NULL REFERENCES position_analysis_moves(id) ON DELETE CASCADE,
    ply              SMALLINT NOT NULL,
    bingo_percentage DOUBLE PRECISION NOT NULL,
    average_score    DOUBLE PRECISION NOT NULL,
    -- The natural key, and the index the cascade from moves uses: move_id is
    -- its leading column, so there is deliberately no second index on it.
    -- There was a BIGSERIAL `id` beside it that nothing referenced or read --
    -- every reader goes through move_id and the insert conflicts on this key
    -- -- at some 30 bytes a row between the column and its index: 2 to 5 GB
    -- for one simming opening-rack job's tens of millions of plies.
    PRIMARY KEY (move_id, ply)
);

-- What a simming player inferred of the opponent's leave before it simmed a
-- captured position: how many distinct leaves the inference found, how many it
-- drew in all, their mean equity, and the most drawn of them -- at most ten
-- `{leave, draws, equity}` objects, most drawn first. Only a simulated in-game
-- position past a game's first turn, whose opponent did not pass, has one; an
-- opening rack never does (there is no opponent move to infer from).
CREATE TABLE position_analysis_inference (
    record_id      BIGINT PRIMARY KEY
                   REFERENCES position_analysis_records(id) ON DELETE CASCADE,
    num_leaves     BIGINT NOT NULL CHECK (num_leaves >= 0),
    total_draws    BIGINT NOT NULL CHECK (total_draws >= 0),
    average_equity DOUBLE PRECISION NOT NULL,
    leaves         JSONB NOT NULL CHECK (jsonb_typeof(leaves) = 'array'
                                         AND jsonb_array_length(leaves) <= 10)
);

-- Shared by games and game pairs: one row per accepted claim, holding the
-- aggregate MAGPIE's autoplay reports. Autoplay does not emit individual games
-- -- it reports counts and score moments for a batch, and in `-gp` mode also
-- the pentanomial: how many completed pairs ended in each of the five possible
-- pair outcomes. The pentanomial is what the match test and the rating fits read; the
-- divergent summary alongside it is a diagnostic only.
CREATE TABLE game_results (
    task_claim_id     UUID PRIMARY KEY REFERENCES task_claims(id) ON DELETE CASCADE,
    task_id           UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    -- Denormalized from the task; see position_analysis_records.job_id.
    job_id            UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,

    -- Every game this task played. Two per pair for a game_pairs task.
    games             INT NOT NULL CHECK (games >= 0),
    wins              INT NOT NULL CHECK (wins >= 0),      -- player 1
    losses            INT NOT NULL CHECK (losses >= 0),
    ties              INT NOT NULL CHECK (ties >= 0),
    p1_score_mean     DOUBLE PRECISION NOT NULL,
    p1_score_sd       DOUBLE PRECISION NOT NULL,
    p2_score_mean     DOUBLE PRECISION NOT NULL,
    p2_score_sd       DOUBLE PRECISION NOT NULL,
    CONSTRAINT game_results_counts_sum CHECK (wins + losses + ties = games),

    -- The pentanomial: how many completed pairs ended in each of the five
    -- outcomes, indexed by player 1's half-point score across the pair, so
    -- pent_0 is "player 1 lost both games" and pent_4 is "won both". NULL for
    -- `games` jobs, which do not play pairs.
    --
    -- This -- not the divergent subset below -- is what the match test and the ratings
    -- read. The pair is the independent unit of a paired run, and *every* pair
    -- belongs in the sample: a pair whose two games played identically is a
    -- guaranteed 1-1 tie, lands in pent_2, and is exactly the observation that
    -- says "these two are hard to tell apart". Dropping those conditions the
    -- sample on its own outcome and inflates the apparent difference without
    -- bound.
    pent_0            INT CHECK (pent_0 >= 0),
    pent_1            INT CHECK (pent_1 >= 0),
    pent_2            INT CHECK (pent_2 >= 0),
    pent_3            INT CHECK (pent_3 >= 0),
    pent_4            INT CHECK (pent_4 >= 0),
    CONSTRAINT game_results_pentanomial_all_or_nothing CHECK (
        (pent_0 IS NULL AND pent_1 IS NULL AND pent_2 IS NULL
             AND pent_3 IS NULL AND pent_4 IS NULL)
        OR (pent_0 IS NOT NULL AND pent_1 IS NOT NULL AND pent_2 IS NOT NULL
             AND pent_3 IS NOT NULL AND pent_4 IS NOT NULL
             -- The pentanomial and the game counts are two views of the same
             -- games, so they must agree on both the count and the outcome:
             -- one pair per two games, and the same half-point total for
             -- player 1 either way. A worker that miscounts fails here rather
             -- than silently biasing a rating pool.
             AND (pent_0 + pent_1 + pent_2 + pent_3 + pent_4) * 2 = games
             AND pent_1 + 2 * pent_2 + 3 * pent_3 + 4 * pent_4 = 2 * wins + ties
             -- And on the draws: a pair scoring one or three half-points holds
             -- exactly one, a pair scoring two holds none or two.
             AND ties - pent_1 - pent_3 BETWEEN 0 AND 2 * pent_2
             AND (ties - pent_1 - pent_3) % 2 = 0)
    ),

    -- The divergent subset: pairs whose two games did not play identically.
    -- Kept as a *diagnostic* -- it says how often two configs actually differ,
    -- and how they fare where they do, which the job page shows as a match
    -- score of its own -- and deliberately not used as a statistical sample.
    -- Each player's mean score over the subset's games, as MAGPIE reports it
    -- (0 when no pair diverged). NULL for `games` jobs.
    divergent_games   INT CHECK (divergent_games >= 0),
    divergent_wins    INT CHECK (divergent_wins >= 0),
    divergent_losses  INT CHECK (divergent_losses >= 0),
    divergent_ties    INT CHECK (divergent_ties >= 0),
    divergent_p1_score_mean DOUBLE PRECISION,
    divergent_p2_score_mean DOUBLE PRECISION,
    CONSTRAINT game_results_divergent_all_or_nothing CHECK (
        (divergent_games IS NULL AND divergent_wins IS NULL
             AND divergent_losses IS NULL AND divergent_ties IS NULL
             AND divergent_p1_score_mean IS NULL AND divergent_p2_score_mean IS NULL)
        OR (divergent_games IS NOT NULL AND divergent_wins IS NOT NULL
             AND divergent_losses IS NOT NULL AND divergent_ties IS NOT NULL
             AND divergent_p1_score_mean IS NOT NULL AND divergent_p2_score_mean IS NOT NULL
             AND divergent_wins + divergent_losses + divergent_ties = divergent_games
             AND divergent_games <= games)
    ),

    submitted_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One row per accepted leave task (a single worker's forced-rack partition of a generation).
-- The full {rack, count, mean} submission is staged in leave_rack_staging, folded into
-- leave_rack_progress by the next merge, and not kept after that — nothing reads it back,
-- so there's no CSV artifact to reference here.
CREATE TABLE leave_records (
    task_claim_id   UUID PRIMARY KEY REFERENCES task_claims(id) ON DELETE CASCADE,
    task_id         UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    rack_count      INT NOT NULL,  -- number of distinct racks in this submission, for audit
    submitted_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One row per completed generation: the server-built combined KLV (see Aggregation in
-- "Leave Generation — On-demand, partitioned generations"), not tied to any single task_claim
-- since it's produced by the server from all of that generation's leave_rack_progress rows.
CREATE TABLE leave_generation_artifacts (
    job_id        UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    generation    INT NOT NULL,
    artifact_key  TEXT NOT NULL,
    -- SHA-256 of the KLV bytes as first written. The object store holds the
    -- only copy of these bytes, and an artifact is the one piece of state that
    -- can be silently overwritten -- by a restore that replays a generation
    -- transition against fewer results, or by a rebuild under a changed KLV
    -- builder (`builder`, below). Recording the hash is what turns that from
    -- invisible into a query; the ON CONFLICT DO NOTHING on insert means the
    -- row keeps the FIRST hash, so a later mismatch is evidence rather than an
    -- overwrite.
    sha256        TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    -- SHA-256 of the bytes the object store holds *now*, when they are not
    -- the bytes first written; NULL while they are. Set by every
    -- `rebuild_artifacts` check from the object it wrote, or found and could
    -- account for. Workers are sent this (or `sha256` when it is NULL) and
    -- refuse bytes that do not match, so it has to follow the object: a
    -- rebuild under a changed builder wrote new bytes, the row kept the old
    -- hash, and every task of the next generation failed its check on every
    -- worker. `sha256` stays the first hash, as the evidence it is.
    served_sha256 TEXT CHECK (served_sha256 ~ '^[0-9a-f]{64}$'),
    -- The MAGPIE KLV builder that wrote these bytes ('klv-1').
    --
    -- MAGPIE builds these artifacts, so an upgrade can legitimately change the
    -- bytes for the same leave values. Without knowing which builder wrote an
    -- artifact, `rebuild-artifacts` could only report "differs" -- and the
    -- first MAGPIE upgrade after a restore drill would read as data loss. With
    -- it, a rebuild under a different builder says so, and only two artifacts
    -- from the *same* builder disagreeing is evidence of anything.
    builder       TEXT NOT NULL,
    completed_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (job_id, generation)
);

-- One row per generation transition that has been *started*, claimed by the
-- worker request that found the generation complete.
--
-- A transition folds millions of leave_rack_progress rows into a KLV and
-- uploads it, which takes tens of seconds and cannot run inside the claim
-- transaction, since a proxy timeout would abandon it part-way. That leaves a
-- window in which a second claim would find the
-- same "every rack at target, nothing in flight" state and start the same
-- transition again, duplicating all of it. The primary key is what makes that
-- impossible: the deciding claim transaction commits this row under the job's
-- advisory lock, and any other claim that sees a live row is told there is no
-- work yet instead.
--
-- `started_at` exists for the crash case. If the process dies mid-transition
-- the row stays behind with no artifact to show for it, and the job would stall
-- forever on a transition nobody is running; a claim that finds a row older
-- than the takeover timeout with no artifact restarts it (see
-- leave_gen::next_step). `completed_at` is set when the artifact row is
-- written, so a stalled or repeated transition is a query rather than a guess.
CREATE TABLE leave_generation_transitions (
    job_id       UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    generation   INT NOT NULL,
    started_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    -- How many times this generation's transition has been started. Above 1
    -- means a takeover happened, which is worth seeing.
    attempts     INT NOT NULL DEFAULT 1 CHECK (attempts >= 1),
    PRIMARY KEY (job_id, generation)
);

-- Ratings
--
-- Ratings are siloed from job control flow entirely: nothing below is read
-- while dispatching, claiming, validating or completing a task, and nothing
-- above (jobs, the two game config tables, game_results) mentions a rating.
-- The coupling runs one way -- the fit reads finished game_results -- so a
-- rating can never affect whether a job stops. The match test stays on the job config
-- tables where it belongs: it is a per-job stopping rule, not a measurement.

-- A rating pool is a set of player configs whose ratings are comparable, plus
-- the game conditions that make them so.
--
-- Scoped by (variant, letterdist, layout) because a rating is only meaningful
-- against fixed conditions: pooling a wordsmog job with a classic one, or two
-- different letter distributions, produces a number describing no game anyone
-- played. Only game_pairs jobs matching a pool's scope are eligible evidence
-- for it. (Lexicon is deliberately *not* part of the scope: it lives on the
-- player config, and two configs on different lexicons playing each other is a
-- meaningful comparison.)
CREATE TABLE rating_pools (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name          TEXT NOT NULL UNIQUE CHECK (char_length(name) <= 100),
    variant       TEXT NOT NULL,
    letterdist_id UUID NOT NULL REFERENCES input_data(id),
    layout_id     UUID NOT NULL REFERENCES input_data(id),
    -- The fixed point every other rating is measured against. Ratings are only
    -- identifiable up to an additive constant, so exactly one player config
    -- must be pinned; the static bot at 2000 is the convention.
    anchor_player_config_id UUID NOT NULL REFERENCES player_configs(id),
    anchor_rating DOUBLE PRECISION NOT NULL DEFAULT 2000,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Which player configs are rated in a pool. Membership is the admin's lever:
-- not every player config belongs in a rating, and a config that is added or
-- removed causes the whole pool to be refit rather than patched, since a batch
-- fit has no per-player history to unwind.
--
-- Removal is soft (the row goes, the games stay in game_results), so
-- re-adding a config costs nothing but a recompute. Note that removing a
-- config also removes its games as *evidence*, which moves everyone else's
-- rating -- that is correct, not a bug, and the reason a removal triggers a
-- full refit.
CREATE TABLE rating_pool_members (
    pool_id          UUID NOT NULL REFERENCES rating_pools(id) ON DELETE CASCADE,
    player_config_id UUID NOT NULL REFERENCES player_configs(id),
    added_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    added_by         UUID REFERENCES users(id) ON DELETE SET NULL,
    PRIMARY KEY (pool_id, player_config_id)
);

-- One fit. Ratings are snapshotted per run rather than mutated in place, which
-- is what makes "why did this rating change?" answerable and gives the ratings
-- page a time axis at no extra cost.
--
-- Kept in full for a month, then thinned to the last run of each UTC day, the
-- pool's first run aside (ratings::thin_old_runs, hourly). A pool with an
-- active job takes a run every two minutes, and past a month nothing reads a
-- run but the history endpoint, which returns at most 500 over the pool's
-- life. The ratings and residuals below go with their run.
CREATE TABLE rating_runs (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    pool_id       UUID NOT NULL REFERENCES rating_pools(id) ON DELETE CASCADE,
    computed_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Why this run happened: 'membership' (an admin added or removed a config),
    -- 'evidence' (new results arrived), 'manual', or 'anchor' (an admin moved
    -- the anchor or its rating).
    trigger       TEXT NOT NULL,
    method        TEXT NOT NULL DEFAULT 'bradley_terry_newton',
    -- Fit provenance. A run that did not converge is still stored and still
    -- displayed, flagged: hiding it would leave the page silently stale.
    iterations    INT NOT NULL,
    converged     BOOLEAN NOT NULL,
    -- How much evidence went in, so a run can be compared to its predecessor
    -- without re-reading game_results.
    pairs_used    BIGINT NOT NULL,
    jobs_used     INT NOT NULL,
    -- The pool's eligible jobs' `games_completed`, summed, as of the fit: what
    -- the sweep compares before deciding to build the evidence matrix at all.
    -- Building it to find nothing had changed was the sweep's whole cost, for
    -- every pool every two minutes. NULL on a run that did not record it,
    -- which the next sweep refits.
    evidence_games BIGINT
);

CREATE INDEX rating_runs_pool_idx ON rating_runs (pool_id, computed_at DESC);

-- The ratings themselves: one row per player config per run. This is the only
-- table in the schema that holds a rating.
CREATE TABLE player_config_ratings (
    run_id           UUID NOT NULL REFERENCES rating_runs(id) ON DELETE CASCADE,
    player_config_id UUID NOT NULL REFERENCES player_configs(id),
    rating           DOUBLE PRECISION NOT NULL,
    -- Approximate standard error, in rating points. Wide bars are the honest
    -- signal that a config has barely played, or has only played opponents
    -- far from its own strength; the page shows them next to the rating for
    -- that reason.
    stderr           DOUBLE PRECISION NOT NULL,
    pairs_played     BIGINT NOT NULL,
    -- FALSE when no chain of games connects this config to the pool's anchor.
    -- Ratings are identifiable only relative to the anchor, so such a config's
    -- number comes from the fit's prior alone and means nothing; it is shown as
    -- unrated rather than as a confident 1500.
    connected_to_anchor BOOLEAN NOT NULL,
    is_anchor        BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (run_id, player_config_id)
);

-- The cross table of one fit, and its residuals: for every head-to-head with
-- games in it, once (the row is the config whose name sorts first), the score
-- that happened, its standard error and the average spread, beside the score
-- the fit's ratings predict. The page mirrors each row for the other side.
-- Stored with the run rather than recomputed on each view of the pool, which
-- rebuilt the pool's evidence matrix -- a grouped scan over every paired
-- result it counts -- on every public page view. Stored, they also describe
-- the evidence this fit used, not evidence that has moved on since.
CREATE TABLE rating_run_residuals (
    run_id               UUID NOT NULL REFERENCES rating_runs(id) ON DELETE CASCADE,
    row_player_config_id UUID NOT NULL REFERENCES player_configs(id),
    col_player_config_id UUID NOT NULL REFERENCES player_configs(id),
    pairs                DOUBLE PRECISION NOT NULL,
    actual               DOUBLE PRECISION NOT NULL,  -- the row config's score rate
    predicted            DOUBLE PRECISION NOT NULL,
    -- The standard error of `actual`, from the pairs' score variance in the
    -- summed pentanomial (ratings::HeadToHeadEvidence::score_and_stderr).
    stderr               DOUBLE PRECISION NOT NULL,
    -- The row config's average spread per game: Σ games·(its mean score − the
    -- other's) / Σ games over the same game_results rows.
    spread               DOUBLE PRECISION NOT NULL,
    PRIMARY KEY (run_id, row_player_config_id, col_player_config_id)
);

-- Backups
--
-- Written by scripts/backup.sh (and the restore drill), never by the server:
-- the backend reads this table for the admin dashboard and has no ability to
-- perform or delete a backup. See PLAN.md, "Making backups visible".
--
-- Insert-only, failures included: a run that broke leaves an ok = false row,
-- so the admin page shows a failure rather than a gap that reads as "nothing
-- happened". Nothing in the request path reads this table.
--
-- Restoring the database restores its own backup history, which is
-- momentarily confusing and harmless: the rows describe backups that do still
-- exist in the bucket.
CREATE TABLE backups (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    kind           TEXT NOT NULL CHECK (kind IN ('pg_dump', 'rds_snapshot')),
    -- Key prefix within the backup bucket ('pg/2026-09-07T03-00-00Z'), NULL
    -- for a snapshot; snapshot_id is the mirror of it. Exactly one is set.
    s3_key         TEXT,
    snapshot_id    TEXT,
    started_at     TIMESTAMPTZ NOT NULL,
    finished_at    TIMESTAMPTZ NOT NULL,
    dump_bytes     BIGINT CHECK (dump_bytes >= 0),
    -- Per-table exact counts at dump time. What a restore is verified against
    -- (PLAN.md, "Verifying a restore"), and what makes a silently truncated dump
    -- detectable without restoring it.
    row_counts     JSONB NOT NULL,
    sha256         TEXT CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    ok             BOOLEAN NOT NULL,
    CONSTRAINT backups_has_single_location CHECK (
        (s3_key IS NOT NULL)::int + (snapshot_id IS NOT NULL)::int = 1
    )
);

-- The admin page asks for the most recent runs, and the staleness figure asks
-- for the most recent successful one.
CREATE INDEX backups_finished_idx ON backups (finished_at DESC);

-- Settings an admin changes at run time (`/admin/settings`), as one row. The
-- deployment's own settings are environment variables, which take a deploy
-- to change; these take effect at the next claim.
CREATE TABLE settings (
    -- Always TRUE: the primary key and its check are what make it one row.
    id                BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (id),
    -- The longest a task may run. Every claim is given it (the assignment's
    -- `max_task_seconds`) and keeps its own deadline, claim time plus this as
    -- it stood then, so a change applies to claims made after it. A claim
    -- past its deadline and a minute's grace is reclaimed even while its
    -- worker heartbeats, and its result refused (`task_claims.deadline_at`).
    -- Ten minutes at the least, a day at the most. The floor is not the
    -- shortest batch worth a claim but the first claim on a machine: it may
    -- build the job's rack info table first, a minute to three that cannot
    -- be stopped part-way (and is kept for every task after it), so a limit
    -- near that would stop the task that paid for it, every time, on every
    -- new machine. The API refuses what this refuses (`routes::admin`); a
    -- test that wants a claim past its deadline moves the claim's
    -- `deadline_at`, not this.
    max_task_seconds  INT NOT NULL DEFAULT 3600 CHECK (max_task_seconds BETWEEN 600 AND 86400),
    -- Who changed them last, and when; NULL until anyone has. SET NULL, like
    -- jobs.created_by: the settings outlive the admin.
    updated_by        UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO settings DEFAULT VALUES;

-- Audit log

-- No foreign keys, deliberately. The log is append-only and has to outlive
-- what it describes: the census rows written by delete_job and delete_user
-- exist precisely to be read after the job or user is gone. A foreign key
-- here either blocks those deletions outright (NO ACTION -- every job has a
-- job.created row, every user a user.registered row) or rewrites history
-- (SET NULL / CASCADE).
CREATE TABLE audit_log (
    id              BIGSERIAL PRIMARY KEY,
    action          TEXT NOT NULL,
    actor_user_id   UUID,
    actor_anon_uuid UUID,
    target_type     TEXT,
    target_id       TEXT,
    -- Typed extra-context columns (replace JSONB metadata)
    job_id          UUID,                           -- task/result events
    reason          TEXT,                           -- ban events, etc.
    old_status      TEXT,                           -- status-change events
    new_status      TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Supporting indexes for the claim / submit / dashboard paths.

CREATE UNIQUE INDEX task_claims_token_idx     ON task_claims (claim_token);
CREATE INDEX        task_claims_task_idx      ON task_claims (task_id);
CREATE INDEX        task_claims_open_idx      ON task_claims (task_id) WHERE state = 'claimed';
-- Overruns not yet counted against their job (`task_claims.overrun`). Every
-- claim request asks which of its candidate jobs have one, and this is what
-- keeps that cheap: an entry lives from the reclamation that records it to
-- the job's next claim, decline or late result, so the index is all but
-- always empty -- a probe, not a scan of the job's claims.
CREATE INDEX        task_claims_overrun_idx   ON task_claims (job_id) WHERE overrun = 'pending';
-- Completed claims by time. The ETA (`jobstats::estimate_eta`, on every
-- detail view and live push) asks "how many of this job's claims completed
-- in the last hour" (the job list's `stalled` flag reads
-- `jobs.last_completed_at` instead), and no index on task_claims leads with the job (but
-- the overruns', which holds next to nothing), so
-- the alternative plan walks every task of the job and every claim of each
-- -- the job's whole history, for a question about its last hour. Through
-- this index the scan is bounded by the fleet's recent completions instead,
-- whatever the job's age -- as long as the query's bound is one the planner
-- can read: a constant `now() - interval '1 hour'`, not a parameter.
CREATE INDEX        task_claims_completed_idx ON task_claims (completed_at DESC)
    WHERE state = 'completed';
-- Each covers only its own kind of identity: a claim has exactly one, and
-- indexing the other kind's claims under a NULL key nothing looks up was dead
-- weight -- nearly half of each index. A lookup by `= $n` implies the
-- `IS NOT NULL`, so every reader still uses them.
-- Keyed by identity, job and completion time: an identity's completed claims
-- in one job, newest first, are one backward range -- the results feed's
-- `?worker=` reads a page through them whatever the contributor's share of the
-- job. Only completed claims have a `completed_at`; the rest sit at the NULL
-- end, outside the range. A lookup by identity alone still uses the leading
-- column. The completion time adds nothing to a claim's updates: completing
-- one changes `state`, which the open-claims index's predicate reads, so that
-- update was never a HOT one, and a heartbeat touches neither.
-- Each carries the claim's `movegens` and `claimed_at`, so a contributor's
-- work by job type -- movegens, compute time, tasks -- is an index-only walk
-- of their range (`GET /api/workers/*/:id/movegens`).
CREATE INDEX        task_claims_user_idx      ON task_claims (claimed_by_user_id, job_id, completed_at)
    INCLUDE (movegens, claimed_at) WHERE claimed_by_user_id IS NOT NULL;
CREATE INDEX        task_claims_anon_idx      ON task_claims (claimed_by_anon_uuid, job_id, completed_at)
    INCLUDE (movegens, claimed_at) WHERE claimed_by_anon_uuid IS NOT NULL;
-- There is no (job_id, state) index. The job-scoped reads of `tasks` -- the
-- detail page's counts by state, the census -- are served by
-- `tasks_seed_unique_idx (job_id, seed)` and the heap. The opening-rack finish
-- check asks for the job's first task by seed there, and for a task not
-- completed through `tasks_queue_idx` (available) and `task_claims_open_idx`
-- (claimed), not by state. A state index cost an entry on every task insert
-- and every state change, on the claim and submit paths, for no reader that
-- needed it.
-- For the cascade from `tasks` alone: no reader looks a result up by its task.
-- It was (task_id, submitted_at) for the per-task "first accepted result" read
-- every aggregate made while a task could have more than one; with one slot a
-- task has one result, and the aggregates sum the job's (`jobstats`).
CREATE INDEX        game_results_task_idx     ON game_results (task_id);
-- The public results feed for a games or game-pairs job, keyset-paginated like
-- the opening-rack one. `task_claim_id` is the primary key and so the
-- tiebreaker, since `game_results` has no serial.
CREATE INDEX        game_results_feed_idx
    ON game_results (job_id, submitted_at DESC, task_claim_id DESC);
CREATE INDEX        leave_records_task_idx    ON leave_records (task_id);
CREATE INDEX        audit_log_created_idx     ON audit_log (created_at DESC);
CREATE INDEX        audit_log_job_idx         ON audit_log (job_id);

-- Drives claim-time rack selection once few racks remain below target: "the
-- racks furthest from target in this generation" (`leave_gen::furthest_below_target`).
--
-- On `occurrence_count` alone, and selection orders on it alone. Counts tie in
-- their millions -- every rack of a generation starts at zero, and most of the
-- 3.2 million full racks are rare enough to stay there until they are forced --
-- so an ORDER BY that also broke ties by rack could not be served by this index
-- and every leave claim sorted the generation to find its few hundred racks
-- (4.9 s a claim at full size). Carrying `rack` in the key fixed that at a
-- price: measured on a full English generation, 180 MB where this index is
-- 22 MB, because keys that are nearly all equal deduplicate and unique keys
-- cannot. Nothing needs the tie broken, so the order gave it up instead; while
-- many racks are below target, selection does not read this index at all (it
-- sweeps the primary key, see `leave_selection_cursors`).
CREATE INDEX leave_rack_progress_pick_idx
    ON leave_rack_progress (job_id, generation, occurrence_count);
```

---

## Known Limits and Open Questions

Ten audits of this repository each left a findings record (`AUDIT_FINDINGS*.md`,
now deleted; they are in the git history up to the commit that removed them).
The eleventh's is `AUDIT_FINDINGS_7.md`, the twelfth's `AUDIT_FINDINGS_8.md`,
the thirteenth's `AUDIT_FINDINGS_9.md`, the fourteenth's `AUDIT_FINDINGS_10.md`,
the fifteenth's `AUDIT_FINDINGS_11.md`, the sixteenth's `AUDIT_FINDINGS_12.md`,
the seventeenth's `AUDIT_FINDINGS_13.md`, the eighteenth's `AUDIT_FINDINGS_14.md`,
the nineteenth's `AUDIT_FINDINGS_15.md`, the twentieth's `AUDIT_FINDINGS_16.md`,
the twenty-first's `AUDIT_FINDINGS_17.md`, the twenty-second's
`AUDIT_FINDINGS_18.md`, the twenty-third's `AUDIT_FINDINGS_19.md`, the
twenty-fourth's `AUDIT_FINDINGS_20.md`, the twenty-fifth's
`AUDIT_FINDINGS_21.md`, the twenty-sixth's `AUDIT_FINDINGS_22.md`, the
twenty-seventh's `AUDIT_FINDINGS_23.md`, the twenty-eighth's
`AUDIT_FINDINGS_24.md`, the twenty-ninth's `AUDIT_FINDINGS_25.md`, the
thirtieth's `AUDIT_FINDINGS_26.md`, the thirty-first's `AUDIT_FINDINGS_27.md`,
the thirty-second's `AUDIT_FINDINGS_28.md` and the thirty-third's
`AUDIT_FINDINGS_29.md` to `AUDIT_FINDINGS_32.md` (one per pass). The feature
batches' and the endgame/pre-endgame plans (`FEATURE_BATCH*_PLAN.md`,
`ENDGAME_PEG_PLAN.md`), all implemented, were deleted with the thirty-third's
records; they too are in the git history.
Everything they *changed* is described where it lives, above. This section is
what they *left*: limits that were accepted on purpose, options that were
considered and not built, and small things noted rather than fixed.

Each finding is numbered (`KL-n`) and set out the same way:

- **Context:** where it lives and what it depends on.
- **Problem:** what goes wrong, or what it costs.
- **Options considered:** the alternatives weighed. Where a finding's record
  names none, it says so, rather than inventing one after the fact.
- **Option implemented:** what the code does now. For a finding still open, that
  is "none yet", with the audit that opened it.
- **Justification:** why that option, and what would make it worth revisiting.

Numbers are permanent. A new finding takes the next unused number, whichever
subsection it goes in. A finding that is later resolved keeps its number and
says so in its implemented option, rather than being removed.

### Scheduling and claims

**KL-1. A lapsed claim of a job nobody claims from stays on its books, and its late result is accepted.**
- **Context:** Reclamation is lazy, and runs only for candidate jobs (see
  [Task States](#task-states)). The Workflow's step 7 says a timed-out claim's
  result is refused.
- **Problem:** A claim whose worker vanished on an inactive, completed or parked
  job stays `claimed` until the job is activated again or exported. If the
  worker comes back hours later, its submission *is* accepted, and the
  `tasks_claimed` the claim holds is inflated meanwhile.
- **Options considered:**
  - refuse any submission past the timeout, whether or not it was reclaimed;
  - a periodic reclaim sweep;
  - leave it.
- **Option implemented:** Left alone deliberately.
- **Justification:** A late result for a finished job is real work. It is
  harmless to the match test, which has already decided, and to ratings, which refit on
  it, and `tasks_claimed` is a display figure. Refusing late submissions throws
  away real results. A periodic sweep is the background process the lazy
  design exists to avoid. The one late result refused whether or not its claim
  was reclaimed is one past the claim's own deadline and grace
  ([Task time limit](#task-time-limit)): that is a task run past the limit
  its worker was given, not a worker that went quiet.

**KL-2. A poison task blocks a games or pairs job at its cap.**
- **Context:** No task is generated past `max_games`/`max_pairs`, so a job at its
  cap completes only when every task handed out has a result. Opening racks
  have always had this property.
- **Problem:** A task that every worker fails keeps the job `active`. It shows as
  repeated `task_failed` declines on one task. Worse, a job *every* task of
  which every worker fails is dispatched on and on: MAGPIE stops `contribute`
  after five failures in a row, so such a job alone on offer takes the whole
  fleet down within seconds, each contributor until its owner restarts it. The
  thirty-second audit found three ways job creation let one through (a 21x21
  layout, a margin past MAGPIE's ceiling, play counts it cannot allocate) and
  refuses them now; the server still does nothing with `task_failed` declines
  beyond this: outside leave generation, a worker is not offered again a
  task it declined within the hour (a small fleet may then wait that hour on
  a job's last task). Before that, the declined task was the oldest available and went
  straight back to whoever claimed next — the worker that had just failed it
  included — ahead of new work, so one task that fails everywhere stopped
  every contributor claiming from its job (thirty-second audit, pass 8); now
  each worker fails it once and moves on. Leave generation keeps the old rule —
  a declined task is reissued as it stands — because its rack selection does
  not count an available task's racks as out: skipped, the declined racks
  went out again on a second claim. A claim has no maximum age: an
  executor that hangs while its process goes on heartbeating holds its task
  for good (reasoned, not reproduced).
  Generation sizes (`num_iterations`, `max_iterations`, …) have no ceiling
  either, except `racks_per_task` (10,000 since the thirty-second audit: every
  claim and every `leave_requests` row carries a task's forced racks); a typo
  makes tasks that outlast their lease rather than fail. Batch sizes have one:
  `racks_per_batch` 10,000 (since the first audit), and since the thirty-second audit's
  pass 21 `games_per_batch`
  10,000 games (1,000 when capturing; `pairs_per_batch` half that) — past
  32,768 captured games a result could not name its games at all. The
  capturing cap does not count what each position records: a result is about
  350 bytes a position and 70 a recorded play, some 22 positions a game, so a
  job recording 50 plays passes 64 MiB at under 900 games, and its every
  result is refused (`413`) and replayed (the audit's pass 22). Sizing a
  capturing batch is the admin's, knowing its players' `num_plays_recorded`.
  And the checks run at creation only: a job or player config written before a
  ceiling tightens (none exists before launch) is not re-checked when it is
  reactivated, or when a new job names the config.
- **Options considered:** let a job past its cap complete when its only
  unfinished tasks have failed more than N times; deactivate, or flag, a job
  after N `task_failed` declines with no accepted result.
- **Option implemented:** None; not built. Creation refuses the configurations
  known to fail everywhere.
- **Justification:** No job has been seen stuck this way, and a guard on
  declines needs a policy (how many, from how many workers, and whether a
  broken release rather than the job is to blame). Revisit if one ever is.

**KL-3. Re-dispatch at redundancy above 1 walks every task the claimant has already filled.** *Closed: redundancy was removed (October 2026); a task has one slot.*
- **Context:** `registry::next_available` takes the oldest `available` task the
  identity holds no slot on. Generation never waits for redundancy to catch up.
- **Problem:** With workers of unequal speed, the fast one's completed-once tasks
  pile up waiting for the slow one. Every claim by the fast worker probes all
  of them first: linear in the backlog, inside the dispatch lock, and the
  backlog is unbounded.
- **Options considered:** a bound on how far generation may run ahead of
  acceptance.
- **Option implemented:** None.
- **Justification:** No job runs above redundancy 1 today. Whoever builds the
  replicated-task cross-check
  ([Why impossibility](#why-impossibility-and-not-per-worker-anomaly-detection))
  needs the bound.

**KL-4. The reclaim statement runs on every claim request.**
- **Context:** It is bounded by the claims in flight across the fleet, not by
  history.
- **Problem:** A millisecond or two per claim at a thousand workers.
- **Options considered:**
  - throttle it per process, to once every few seconds;
  - leave it on every claim.
- **Option implemented:** On every claim.
- **Justification:** The cost is small at the fleet sizes planned, and
  reclamation is already approximate by design. The throttle is there to take
  when a fleet is large enough to care.

**KL-5. Milliseconds deliberately left on the claim and submit paths.**
- **Context:** Four places each cost a round trip or a row lock:
  - taking the dispatch lock is three statements (`SET LOCAL lock_timeout`, the
    lock, the reset), of which only the reset runs under the lock;
  - the finish check reads the job's config row once per check, although the
    template holds it;
  - the identity's `tasks_completed` is bumped inside the submit transaction;
  - the job's own progress totals are one `UPDATE jobs`.
- **Problem:** A few round trips and row locks per claim or submission.
- **Options considered:**
  - batch the lock statements into one round trip;
  - read the config from the template;
  - move the counters out of the transaction.
- **Option implemented:** Kept as described.
- **Justification:**
  - The finish check uses `jobstats::game_stats`, which is shared with the
    display path, and that has no `AppState`.
  - The counter is a single-row update, kept in the transaction so the
    leaderboards' counters cannot drift.
  - The jobs update is the transaction's last statement, because it takes the
    row lock that every claim for the job also takes.
  - What is left is milliseconds.

**KL-6. Large results waiting their turn were still resident. Closed in the thirty-first audit.**
- **Context:** The three-slot bound on storing results of 8 MiB or more (1 MiB
  since this audit) is taken
  after the body has been read.
- **Problem:** Each waiter kept its body in memory for up to thirty seconds, and
  nothing bounded how many bodies were being read: some twenty-five
  maximum-size submissions at once would fill the web task. The thirty-first
  audit found that a caller needed no credentials to do it (§ "What bounds a
  submission?").
- **Options considered:**
  - take the turn from `Content-Length` before the body is read, as a layer on
    the result route;
  - above three at a time, answer `503` and let the submitter re-upload;
  - charge every body to one byte budget as it arrives, and keep a result
    charged until its handler returns.
- **Option implemented:** The first, generalised: a result's declared length
  is reserved whole from a 192 MiB budget (`extract::LARGE_BODIES`) before
  its body is read, or refused at once with `503`; the result, heartbeat and
  decline routes refuse an unknown caller before its body; small bodies share
  no budget but have 30 seconds to arrive. A waiter now holds its reservation,
  so the bodies waiting for a turn count against the same 192 MiB. The third
  option was built first and broken by the audit's own adversarial check: on
  one budget, small bodies could be made to wait behind large ones, and
  heartbeats with them.
- **Justification:** Reserving the declared length lets a caller with an
  identity hold a reservation cheaply, which is KL-54; charging as bytes
  arrive would make holding cost bytes, but lets honest uploads hold part of
  the budget while waiting for more of it. Reservations are the simpler of the
  two to reason about, and only large results are exposed.

**KL-7. `task_claims` is stored at fillfactor 85.**
- **Context:** A heartbeat changes only `last_heartbeat_at`, which no index
  covers, so it can be a HOT update if the row's page has room.
- **Problem:**
  - Some 15% of every claim page is kept free for good: about 6.5 MB a day at
    288,000 claims a day.
  - While the nightly dump holds its snapshot, HOT pruning cannot reclaim dead
    versions, so heartbeats are non-HOT again for the backup window. An
    export's snapshot does the same for as long as it reads (KL-94).
- **Options considered:** the default fillfactor of 100, or 85.
- **Option implemented:** 85.
- **Justification:** At 100 a claim's first heartbeat found its page full and
  wrote a new entry into every one of the table's indexes: two a minute for
  every claim in flight. The free space costs less than that.

**KL-8. A leave task fetches its KLV every time.**
- **Context:** The KLV is 3.6 MB from the store, plus a buffered copy on the
  server.
- **Problem:** The transfer repeats even when the worker already has those bytes
  under their content name.
- **Options considered:**
  - skip the fetch when the file matches, which needs another way to guarantee
    the process reads the file again;
  - fetch afresh.
- **Option implemented:** Fetched afresh every task.
- **Justification:** Writing the file afresh gives it a new file identity, which
  guarantees the process reads it again. That matters if an aborted leave run
  left its in-memory copy altered.

**KL-9. A leave generation's progress row is taken mid-submission.**
- **Context:** `stage_fold`'s upsert of `leave_generation_progress` (display
  counters) locks the generation's row from there to the commit.
- **Problem:** It serializes that generation's submissions for the last few
  statements.
- **Options considered:** move the upsert to the transaction's end (twelfth
  audit).
- **Option implemented:** Not moved.
- **Justification:** Those submissions already serialize on the job's row at the
  end, so moving it would save milliseconds.

**KL-10. The finish check runs before the worker is answered.**
- **Context:** The check is not display. It decides whether the job goes on
  dispatching.
- **Problem:** One submission in eight pays an aggregate over the job's results;
  the rest pay an `EXISTS`. The worker waits for it, holding a main-pool
  connection. It is a sum over every result row of the job, through
  `game_results_feed_idx`. While a task could have several results it was a
  sort of every row by task to keep the first: some 0.4 s (340–620 ms for
  games, 430–520 for pairs) at 400,000 rows, a job of 400,000 games or pairs at
  the form's default batch of one,
  where this entry said tens of milliseconds (thirty-second audit). The sum
  that replaced it with redundancy has not been re-measured; it reads the same
  rows without the sort, so it should cost well under that, and it is still
  linear in the job's history. Half of every live stats build is the same
  read.
- **Options considered:** move it off the worker's wait (eleventh audit);
  per-job win/loss/tie and pentanomial running totals kept in the submit
  transaction beside `jobs.games_completed`, for the match test and the page, with the
  full read as a periodic cross-check; a finish-check stride that grows with
  the job; a leaner form of the full results read (about 200–280 ms, against
  the sorting form of the time).
- **Option implemented:** Kept inline, and unchanged.
- **Justification:** It gates dispatch, a correctness input, and a counter the
  stopping rule trusts is what the read was kept to avoid; running totals are a
  schema change and a write per submission. Worth taking when a job of
  hundreds of thousands of one-game batches is run; larger batches make the
  read proportionally smaller.

**KL-11. A claim whose response is lost is a second claim when it is retried.**
- **Context:** The claim commits before the response is written, and MAGPIE
  retries a claim without limit.
- **Problem:** If the connection drops, or the load balancer answers `502`, the
  retry is handed another task. The first claim holds its slot, unheard from,
  until the heartbeat timeout. At a leave job's lap end, or a games job at its
  cap, that is the same idle as a dead worker, with none.
- **Options considered:**
  - a per-claim request id, sent by MAGPIE and reused across retries, stored on
    `task_claims` under a unique partial index, with a retry answered with the
    existing claim's assignment;
  - leave it.
- **Option implemented:** None yet. *Open (eleventh audit).*
- **Justification:** It is a wire change on both sides for a rare trigger, and
  is left for a decision.

**KL-12. A purge or delete of a large job is one request, in one transaction.**
- **Context:** Claims skip the job while the purge runs, and nothing waits on the
  claims or contributor rows it holds (see
  [Job Lifecycle Controls](#job-lifecycle-controls)). It runs on a task of its
  own, so a request the load balancer drops at its 300-second idle timeout no
  longer takes the transaction with it. The hold on the job lasts at least as
  long as its locks: to the end of the steps after the commit, the
  generation-0 rebuild among them.
- **Problem:** Its own length. The cascades are tens of millions of rows for a
  full simming opening-rack job:
  - minutes on the production instance class, with the job's claims held
    throughout;
  - the admin told nothing past the load balancer's timeout (the audit log and
    the job's page say when it is done, and clicking again is answered `409`);
  - that many dead tuples left to vacuum.
- **Options considered:**
  - a purge that commits the job's state change at once and deletes bottom-up in
    batches on a spawned task with a status row, like an export;
  - the result tables partitioned by job, so a purge is a `TRUNCATE` and a
    delete a `DROP`.
- **Option implemented:** None yet. *Open (eleventh audit).*
- **Justification:** Both are real redesigns, and neither is needed until a job
  that size exists. A purge's cost should first be measured on the synthetic
  million-rack dataset.

**KL-13. A reclaim can land between two statements of one leave claim.**
- **Context:** Each statement of a leave claim has its own snapshot.
- **Problem:** A claim can see a task as "not available" and then "not in
  flight", and start a lap (or a tail selection) with one lapsed task
  unissued. The next claim reissues it, so that task's racks are forced twice,
  on different seeds.
- **Options considered:** None recorded.
- **Option implemented:** Accepted as it is.
- **Justification:** The cost is duplicate coverage, never a closed generation
  missing a result: closing reads what is in flight and staged afresh, and
  then — on the tail's path too, since the thirty-second audit — whether any
  rack is below target at all, holding nothing out. Before that, a decline, a
  lapsed claim or a merge landing between the tail's selection and those
  reads closed the generation with that task's racks short of target (a
  declined task's at zero), which an instrumented test reproduced.

**KL-14. A worker's thread count is its own.**
- **Context:** A simulation's sampling, an endgame or pre-endgame solve, and a
  multi-threaded leave-generation run all depend on the worker's thread count;
  only static players that solve nothing are deterministic.
- **Problem:** The same task can give different results on different workers.
  That is why simming and solving jobs are excluded from any equality cross-check, and why
  an opening-rack job's consensus is only worth seeking for a simming player.
- **Options considered:**
  - pin threads per task from the server;
  - leave them to the contributor's `contribute.txt`.
- **Option implemented:** The contributor's.
- **Justification:** Pinning would be fair across workers and a waste of
  contributors' cores, so it was rejected.

**KL-54. The large-result budget can be held by callers with identities.**
- **Context:** A result declaring more than 1 MiB reserves its length from a
  192 MiB budget before its body is read; one identity may hold 64 MiB of it; each
  has a deadline and may not stall for 30 seconds (thirty-first audit; §
  "What bounds a submission?"). The split of a credential's bucket in two
  (claims, and work in hand) also doubles the credential lookups one
  credential may cause, to two a second.
- **Problem:** Three identities, each declaring 64 MiB and sending a byte every
  half minute, hold the whole budget until their deadlines (some seventeen
  minutes), and then again. Every other large result is refused with `503`
  meanwhile; MAGPIE retries for a quarter of an hour and then gives up the
  result. Small bodies — every heartbeat, claim, decline and result of 1 MiB or
  less — are untouched, and memory stays bounded. Results over 1 MiB are
  capture jobs' and some opening-rack jobs' (500 racks a task, with a simming
  player recording ten or more moves, is some 2 MB). And a small body has a
  fixed 30 seconds: a claim listing jobs its worker cannot run near the 1 MiB
  bound, on a link under about 35 KB/s, never arrives in time. The same fixed
  30 seconds applies to a result of up to 1 MiB, while one just over it gets
  30 s plus its size at 64 KiB/s: a 1.0 MiB result on a 32 KB/s uplink is
  refused on every retry where a 1.01 MiB one would pass (thirty-second audit).
- **Options considered:**
  - charge a large body as its bytes arrive, so holding it costs the bytes (at
    the price of honest uploads holding part of the budget while they wait);
  - a minimum average rate enforced as the body arrives, not only by its
    deadline;
  - rate rules at the load balancer (AWS WAF);
  - the large tier's rate-based deadline for a registered result of any size.
- **Option implemented:** Reservation, per-identity, deadline and stall limit.
- **Justification:** The holders need identities an admin can ban, and nothing
  but large results waits on it. Revisit if capture or simming opening-rack
  jobs run at scale, or the slow-link claim is ever seen.

**KL-55. Redundancy above 1 has two untested edges.** *Closed: redundancy was removed (October 2026); a task has one slot.*
- **Context:** No job runs above redundancy 1 today (see KL-3).
- **Problem:**
  - Two concurrent reclaims holding lapsed claims on the same two tasks update
    the `tasks` rows in opposite orders, and Postgres aborts one as a deadlock
    (logged, not fatal).
  - An `Unregistered` identity carries a fresh UUID, so a client that loses or
    never keeps its UUID can take a second slot on the same task.
- **Options considered:** order the reclaim's task updates and lock them with
  `SELECT … FOR UPDATE` first; key the slot check on the minted identity only
  once it persists.
- **Option implemented:** None yet. *Open (thirty-first audit).*
- **Justification:** Both matter only above redundancy 1. Whoever builds the
  replicated-task cross-check fixes them with KL-3.

**KL-56. An identity-less claim mints an identity and holds a task.**
- **Context:** A client with no identity may claim five times a second per
  address, burst thirty; each claim that hands out a task mints an identity.
- **Problem:** One address can hold some 1,500 tasks open for the heartbeat
  timeout, which can pin a leave job's lap end or tail, and those claims count
  toward the job's share.
- **Options considered:** a lower identity-less rate; a cap on open claims per
  address.
- **Option implemented:** None.
- **Justification:** It is outside the stated threat model (a broken client,
  not a hostile one). A ban does **not** stop it: such a client sends no
  identity, so each claim mints a new one, and banning any of them changes
  nothing (the thirty-second audit, pass 9, reproduced it: five of five
  claims after the ban got tasks). Nor does banning an account stop its owner
  claiming with no key. There is no ban by address; a cap on open claims per
  address, above, is the lever that would. Revisit if it is seen.

**KL-57. A job waiting on a derived build asks about it on every claim.**
- **Context:** `derived::status_for_job` runs for a waiting job on every claim
  that considers it, until the build lands (minutes; at worst three attempts
  of up to 70 minutes each, 5 and 15 minutes apart, each attempt a builder
  killed mid-build holds for its 75-minute lease, and the next scheduled run
  up to five minutes away) — or, for a build that has failed for good, until
  an admin retries it.
- **Problem:** A few milliseconds on every such claim, and a waiting job heads
  the candidate list because its claim count does not move — for as long as a
  failed build is left failed, which is not bounded by anything.
- **Options considered:** throttle it like `JobTemplates::recently_failed`, once
  a few seconds per job.
- **Option implemented:** None.
- **Justification:** Small; bounded by the build, or by an admin's reading
  `/admin/derived-data`. Add the throttle if many jobs ever wait at once.

### Leave generation

**KL-15. A lap's end can idle the job for a dead worker's timeout plus a replay.**
- **Context:** A sweep's lap starts only with nothing in flight and nothing
  staged. That rule is what lets a claim carry no exclusion list.
- **Problem:** When a lap's racks run out, the job hands out nothing until the
  lap's last results are in and merged.
  - With every worker alive, that is one task's duration.
  - When the worker holding one of the lap's last tasks has gone, it is the
    heartbeat timeout for the claim to lapse (twice that just after a server
    restart, with the reclamation grace), **plus a whole task** for whoever is
    reissued it, for every worker on the job.

  A lap is about 6,400 tasks for English, so this is a per-lap tax, and a fleet
  with other jobs to run loses only the leave job's share.
- **Options considered:**
  - *start the next lap while stragglers are out*, carrying an exclusion list of
    their racks only;
  - *reissue a suspect task redundantly near a lap's end*, once its claim has
    been silent for a couple of heartbeat intervals;
  - leave it.
- **Option implemented:** Left, a decision.
- **Justification:** It is a throughput tax on one job type, not a correctness
  problem, and each repair gives back some of what the sweep bought.
  - The first is bounded by the stragglers rather than by what is staged, but it
    reintroduces the list, and the next lap selects on counts missing the
    stragglers' own results.
  - The second needs a notion of a suspect claim that the design does not have,
    and burns a duplicate task when the worker was only slow.

  The first is the one to build if leave generation is ever the only job a
  large fleet is running, where an idle lap end is the whole fleet idle.

**KL-16. A sweep visits racks in key order, not rarest first.**
- **Context:** Selection sweeps the primary key from a cursor.
- **Problem:** Within a lap, it gives up forcing the rarest racks first.
- **Options considered:** lowest-count-first, the order it replaced.
- **Option implemented:** Key order.
- **Justification:** Within a lap the order is immaterial, since every rack
  below target is visited, and across laps the racks still short are what the
  next lap finds. Lowest-count-first only ever achieved rarest-first to the
  resolution of the last merge.

**KL-17. Claims on one leave job serialize on its dispatch lock.**
- **Context:** Selection is a fraction of a millisecond for most of a lap. Late
  in a lap, with about 2% of racks still below target and the heap no longer in
  rack order (as repeated merges leave it), a sweep claim walks some 26,500 key
  entries with a heap fetch each: 48–100 ms with the OS cache warm, measured in
  the thirty-second audit; on an instance whose cache cannot hold the
  generation it is thousands of random reads, inside the lock (not measured).
- **Problem:** The claim rate has a ceiling per job.
- **Options considered:** larger tasks.
- **Option implemented:** `num_iterations` is the tuning knob: larger tasks mean
  fewer claims and fewer, larger submissions.
- **Justification:** The ceiling is high while the generation is in cache.
  Worth measuring on the production instance class late in a lap with a cold
  cache, where the reads are random and the lock's other claimants give up
  after two seconds. Every claim also asks whether its generation's universe
  exists, inside the lock; until the thirty-third audit's third pass that was
  a sequential scan of every older generation's rows (KL-18), 133–279 ms a
  claim at 2–3 million rows and seconds once they outgrow the cache. It is
  one index probe now, whatever the history (`I-LEAVE-25`).

**KL-18. `leave_rack_progress` keeps every generation's rows for the life of the job.**
- **Context:** About 430 MB a generation for English. Once a generation's
  artifact is verified, its rows are read again only by `rebuild-artifacts`, the
  job's export (`leave_rack_progress` for every generation, `exports.rs`) and
  its public results feed (every generation, newest first, `routes/public.rs`).
- **Problem:** Storage that grows with every generation.
- **Options considered:** drop or archive a closed generation's rows, keeping the
  hash. Dropped, they also go from the export and the public feed, which would
  then hold only the current generation; archived, both would have to read the
  archive.
- **Option implemented:** Kept.
- **Justification:** The rows are what `rebuild-artifacts` derives a KLV from,
  and the design treats the object store as derivable from the database.
  Dropping them trades that guarantee for the space. They cost storage only,
  as long as every per-claim read of the table reaches the current
  generation through an index: an `EXISTS` probe that Postgres planned as a
  sequential scan read every closed generation first, on every leave claim
  (KL-17; fixed in the thirty-third audit's third pass).

**KL-19. `rebuild-artifacts` runs every generation inline in the admin request.**
- **Context:** About 13 seconds a generation in a release build.
- **Problem:** Past roughly twenty generations, the request outlasts the load
  balancer's 300-second idle timeout and stops part-way.
- **Options considered:** a spawned task with a status row, like exports.
- **Option implemented:** None.
- **Justification:** Each upload is idempotent, so running it again repairs a
  partial run. Build the task if a job ever has that many generations. (A job
  may list up to 100 generations' targets, so one that long can exist.)

**KL-20. A submission's validation ran while its claim and task rows were locked.** *(Closed.)*
- **Context:** It ran inside the same locks before, on an async worker, and then on the blocking pool.
- **Problem:** The locks were held while the result was decoded.
- **Options considered:** decode *before* taking the locks.
- **Option implemented:** Decoded before: the claim's job is read without a lock (a task never changes job), the result decoded and validated with nothing locked, and the claim then locked and checked again in the transaction that completes it (Result Submission, steps 1 and 2).
- **Justification:** A stale claim still ends as `accepted: false`, having cost only the decode.

**KL-21. `seed_generation`'s `COPY` is not aborted explicitly if a chunk fails to build.**
- **Context:** Seeding a generation streams its rows through `COPY`.
- **Problem:** A failed chunk leaves the `COPY` without an explicit abort.
- **Options considered:** an explicit abort.
- **Option implemented:** None.
- **Justification:** The transaction rolls back either way.

**KL-22. The database orders racks by its collation, not by byte.**
- **Context:** Under `en_US.utf8`, the test database's collation and RDS's
  default, `?` is ignored at the first level.
- **Problem:** `?AAABBC` sorts among the `AAABBC…` racks rather than before them,
  so a database-ordered rack list differs from an application-ordered one.
- **Options considered:** None recorded.
- **Option implemented:** Ordering stays in Postgres.
- **Justification:** Everything that orders on `rack` does so *inside* Postgres
  (the sweep, the feed's seek, the KLV's stream), so it is self-consistent.
  Anyone comparing the two kinds of list should know.

### Storage

**KL-23. Captured positions store their CGP as `TEXT`.**
- **Context:** 130–270 bytes each for English, below the TOAST compression
  threshold.
- **Problem:** About 1.8 GB for the nine million positions of a 400,000-game
  capture job.
- **Options considered:** a packed machine-letter encoding, several times
  smaller, which is a schema and wire change.
- **Option implemented:** `TEXT`, a decision.
- **Justification:** Leave it until a capture job of that size exists.

**KL-24. Every rating run keeps its residuals for the month runs are kept in full.**
- **Context:** Only the newest run's residuals are ever read, on the pool page.
- **Problem:** Some 4.5 million rows for a pool of twenty at steady state, and
  3–6 GB for a dense pool of forty, all of it dumped nightly.
- **Options considered:**
  - keep residuals for the newest run only, deleting the superseded run's in
    the fit that supersedes it;
  - keep them only for the runs the thinning keeps.
- **Option implemented:** None yet. *Open (twelfth audit)*, a retention decision.
- **Justification:** The run-by-run diff that this month of history exists for
  is the ratings', not the residuals'. Deleting history is a human's call.

**KL-25. Per-ply statistics are a row each, and a move's id exists only for them.**
- **Context:**
  - `position_analysis_plies` (≈88 bytes a ply with its key) is read by the
    export, which folds a move's plies straight back into an array, and by the
    public rack lookup and saved positions, which show each move's first two
    (`FIRST_PLIES`, `routes/public.rs`); the `float8[]` option below rewrites
    both reads.
  - `position_analysis_moves.id` is referenced only by the plies.
- **Problem:** Storage and inserts on the submit path.
- **Options considered:**
  - two `float8[]` columns on the move: ≈28 bytes a ply, with the ply inserts
    gone from the submit path, saving about 2.4 GB per simming opening-rack
    job;
  - `(record_id, rank)` as the move's key (already indexed), saving about
    0.9 GB per full opening-rack job.
- **Option implemented:** None yet. *Open (twelfth audit).*
- **Justification:** Both are schema and insert-path rewrites, and the moves one
  touches the restore order and every reader. They are worth doing together,
  before release, if simming opening-rack jobs are planned.

**KL-26. `position_analysis_records.task_id` is stored for opening racks.**
- **Context:** The opening-rack dedup key does not use it.
- **Problem:** 16 bytes a record, about 50 MB per full opening-rack job.
- **Options considered:** make it nullable for opening racks.
- **Option implemented:** None.
- **Justification:** Not worth a change of its own.

**KL-27. `audit_log` has no retention.**
- **Context:** Claims and submissions no longer write to it, which removed most
  of its growth. What remains is admin and account actions, and one row per
  decline.
- **Problem:** It still grows without bound.
- **Options considered:** partition it by month, and drop old partitions of
  worker events.
- **Option implemented:** None.
- **Justification:** The remaining growth is small. Partition it if it ever
  matters.

**KL-28. `worker_data_gaps` and `task.declined` audit rows grow with declines.**
- **Context:** Up to 32 gap rows and one audit row per decline.
- **Problem:** Growth with declines.
- **Options considered:** None recorded.
- **Option implemented:** None.
- **Justification:** It is bounded in practice by workers × jobs, since a
  client remembers a job it declined, though only until it restarts.

**KL-29. A completed job's scheduling history is kept for good.**
- **Context:** `tasks`, `task_claims` (abandoned and declined ones included) and
  the per-type request rows stay as long as the job's results do. Nothing
  deletes them short of a purge.
- **Problem:** Storage. A leave generation's `forced_racks` arrays alone are 20
  to 40 MB.
- **Options considered:** delete a completed job's scheduling history, as
  distinct from its results; keep it.
- **Option implemented:** None yet. *Open (eleventh audit)*, a retention
  decision.
- **Justification:** Deleting it is irreversible, and results reference claims.

**KL-30. `input_data_import_rows` of confirmed and failed imports are never deleted.**
- **Context:** Only staged imports expire.
- **Problem:** Small rows that accumulate.
- **Options considered:** include them in KL-29's retention decision.
- **Option implemented:** None.
- **Justification:** The rows are small, and part of the same decision.

**KL-31. The nightly backup reads the database twice.**
- **Context:** `pg_dump` runs, then an exact `count(*)` of every table in the
  same snapshot, which the manifest and the drill compare against.
- **Problem:** A second full read of the instance.
- **Options considered:** count lines of the dump's own data files instead.
- **Option implemented:** None.
- **Justification:** It changes what the manifest certifies. It is left for when
  the corpus makes the backup window matter.

**KL-32. `audit_log`'s filters and `task_claims.claimed_at` have no index.**
- **Context:** A filtered audit query (by action, target type or actor) is a
  sequential scan of the log and a sort, and a page view without a `job_id`
  filter counts the whole log (`COUNT(*)`, a sequential scan); the unfiltered
  first page and a `job_id` filter, count included, use their indexes. `GET
  /api/admin/fleet` filtered on `claimed_at` and so scanned every claim ever
  made, not a week of them: `task_claims` is never pruned (KL-29), so the scan
  grew without bound, on the main pool with no statement timeout.
- **Problem:** At a million audit rows, 35 to 90 ms for a filtered page and 25
  to 70 ms for the count (8 ms at 100,000; the audit's passes 22 and 23, PG16
  defaults);
  the fleet page's scan was 210 ms at 1.5 million claims (half a year at a
  claim every ten seconds) and linear in the table from there.
- **Options considered:** an index on `claimed_at`; the fleet page counted from
  the week's completed claims and the open ones claimed that week, through the
  two partial indexes that already exist.
- **Option implemented:** The second, for the fleet page (thirty-third audit,
  pass 1): 88 ms on the same table, reading a week of the fleet's completions
  rather than its history, and no longer counting a claim that lapsed or was
  declined. An open claim counts only if it was made in the week too: one
  lazy reclamation has left `claimed` long after its worker went (KL-1) is not
  a version anything is running now. The audit log's scans are unchanged. Both reads moved to the
  display pool, so they take its 15 s statement timeout and stay off the
  connections claims use.
- **Justification:** An index on `claimed_at` would cost an entry per claim on
  the hottest write table, for one view; the audit log's filters are admin-only
  and bounded by the log's size.

**KL-58. `jobstats::worker_contributions` scales with the job.**
- **Context:** The per-contributor table on a job's page, rebuilt at most every
  ten seconds per watched job (`JOB_STATS_CACHE_SECONDS`).
- **Problem:** 0.8 s for a job with 600,000 completed claims, about 8% of a
  database core per busy dashboard. It still joins `tasks`, though
  `task_claims.job_id` exists.
  A watched job that is also visited can cost two builds an interval, and a
  push overtaken by a page build more (KL-78).
- **Options considered:** read `task_claims` by `job_id` alone; keep running
  totals per contributor.
- **Option implemented:** None.
- **Justification:** Within the "decide on evidence" stance
  (`SLOW_STATS_THRESHOLD`, one second, logs the whole stats computation this is
  part of). Revisit when it is logged.

### Abuse and input

**KL-33. Recovering an account does not revoke its API keys.**
- **Context:** A password reset and "sign out everywhere" end sessions. Keys are
  listed on the account page, where the owner can revoke them.
- **Problem:** Keys an attacker made while in control go on working until the
  owner revokes them.
- **Options considered:** revoke keys on reset.
- **Option implemented:** None yet. *Open (sixteenth audit).*
- **Justification:** Revoking would stop every one of the contributor's machines
  along with the attacker's.

**KL-34. Confirming an address happens when the link is opened.**
- **Context:** The confirmation page confirms on load.
- **Problem:** A mail scanner that runs the page's script confirms the address
  — and with it an account someone else registered on that address, under a
  name they chose; the owner can take it over by a reset, but inherits the
  name, and the registrant's API keys go on working (KL-33). The confirmation
  mail does not name the account: it was named for a pass of the
  thirty-second audit, which carried a registrant's own 32 characters —
  newlines included — to any address under birdtest's sender. Once a scanner
  has confirmed such an account, the reset mail and the taken-address notice
  do name it, so an owner who resets sees whose it is, and so a stranger's
  name still reaches the owner — but only as a name: a username holds no line
  break, control or invisible character (refused at registration; an older
  account's are mailed as `?`).
- **Options considered:** a button to press.
- **Option implemented:** Confirm on open. *(Sixteenth audit.)*
- **Justification:** A button would cost every registrant a click.

**KL-35. Only ASCII addresses register (`is_bare_address`), and only ones SES parses.**
- **Context:** SES does not send to SMTPUTF8 addresses, and refuses one that is
  not a dot-atom at a host name (pass 24 holds registration to that).
- **Problem:** Internationalized addresses cannot register, nor quoted local
  parts or address literals (`"a b"@x.com`, `a@[192.0.2.1]`).
- **Options considered:** have the form send an internationalized domain as
  punycode.
- **Option implemented:** None. *(Sixteenth audit.)*
- **Justification:** Existing accounts are unaffected.

**KL-36. Every pool connection is pinged when it is taken.**
- **Context:** sqlx's `test_before_acquire`.
- **Problem:** A round trip per acquire: three or four per claim or submission.
- **Options considered:** turn it off, saving the round trips; a connection
  broken by a failover would then fail one request instead of being replaced
  silently.
- **Option implemented:** None yet. *Open (sixteenth audit).*
- **Justification:** A trade between those round trips and one failed request
  after a failover, left for a decision.

**KL-37. Registration answers the same, and what it leaves behind does not.**
- **Context:** A taken address and a fresh one get byte-identical registration
  responses, but only the fresh one creates the account.
- **Problem:** Signing in with the username and password just used answers
  `403` "confirm your email" against `401`, which tells a prober whether the
  address was free. That is deterministic, where the login timing gap is only
  statistical, and the probe holds a free address for a day. Until fixed, "someone
  probing the address learns nothing" (Account Creation Flow) is true of the
  response and not of the account.
- **Options considered:** keep registrations pending confirmation in a table of
  their own (username unique, address not), create the `users` row only at
  confirmation, and record a taken address as a pending row too, so both
  branches leave the same state.
- **Option implemented:** None yet. *Open (twelfth audit).* The thirty-first
  audit found two more channels for the same fact, left with it: two
  registrations of one fresh address racing past the taken check give the
  loser a `409` (the unique index) rather than the identical `201`, and
  `/api/users` lists unconfirmed accounts. The thirty-second audit measured the
  statistical gap too: a fresh address's insert and commit take about 1.6 ms
  that a taken one's notice does not (medians 31.8 against 30.2 ms, the
  Argon2 run being the same).
- **Justification:** It is a schema and flow change, left for a decision. The
  pending-registrations table would close all four.

**KL-38. Unconfirmed accounts, spent tokens and anonymous identities are never reaped.**
- **Context:**
  - An unconfirmed account goes only when its name or address is registered
    again.
  - Expired and used confirmation and reset tokens stay.
  - Every worker install that ever received a task keeps its
    `anonymous_workers` row.
- **Problem:** Rows that accumulate.
- **Options considered:** a reaper in the hourly sweep, plus a policy on what an
  anonymous identity with contributions is worth keeping.
- **Option implemented:** None.
- **Justification:** The rows are small and bounded by rate limits. Add the
  reaper if they ever matter.

**KL-39. `?worker=<pseudonym>` hashes every contributing anonymous identity.**
- **Context:** The pseudonym is a SHA-256 that cannot be indexed as written,
  because `convert_to` is not immutable.
- **Problem:** At 100,000 anonymous contributors, 100–200 ms on the display pool.
  That is under its fifteen-second statement timeout.
- **Options considered:** a stored pseudonym column with an index.
- **Option implemented:** None.
- **Justification:** Build it when that many anonymous contributors exist.

**KL-40. Usernames are unique whatever their case, and otherwise free text.**
- **Context:** Any string of 3–32 characters is accepted, less line breaks,
  control, format (zero-width, bidi) and other invisible characters, which the
  thirty-second audit refuses — joiners and variation selectors allowed only
  where scripts and emoji place them, and a name that differs from a taken one
  only in those is taken (and an expired, unconfirmed one gives it up) — even
  where a non-joiner is visible, as in Persian, which merges `می‌خواهم` with
  `میخواهم`, and Malayalam's older chillu (`ന്‍`, allowed as a trailing
  joiner after a virama) with a bare virama, while it does not merge that
  chillu with its atomic form (`ൻ`), which looks the same. That check scans
  every name (about a quarter of a second at 200,000 accounts, under the
  registration limit) and is check-then-insert: two twins registering at the
  same instant both succeed. An expression index on the stripped name would
  make it a lookup, and a unique one close the race once existing twins are
  cleared. Look-alikes are left alone: another script's `а` for
  `a`, composed and decomposed accents.
- **Problem:** Variants of a name can sit side by side on the public lists. One
  such variant, a username of 16 hex characters, merges under `?worker=` with
  the anonymous worker whose pseudonym it matches.
- **Options considered:**
  - refuse format and control characters (done, thirty-second audit);
  - normalize (NFKC);
  - allow one script only;
  - refuse pseudonym-shaped names.
- **Option implemented:** Invisible characters refused; the rest is a product
  decision left open.
- **Justification:** A character-set rule has to allow for international names,
  which is a product decision.

**KL-41. A rating pool could not be edited or deleted once created.** Closed in the September 2026 feature batch.
- **Context:** A pool's anchor and scope were fixed. Creation validates them: a
  variant a job can have, input-data rows of the right roles, and an anchor
  rating whose scale does not overflow.
- **Problem:** A pool made by mistake stayed, and kept its anchor config from
  being deleted; an anchor could not be moved at all.
- **Options considered:** an update route and a delete route.
- **Option implemented:** Both. `PATCH /api/admin/rating-pools/:id` moves the
  anchor and its rating and refits; `DELETE` removes the pool and what cascades
  from it, freeing its configs and input data. The scope (variant,
  distribution, layout) stays fixed: a pool over other conditions is another
  pool.
- **Justification:** Mistakes are cheap to undo, and a pool's scale can follow
  the configs an admin wants it anchored on.

**KL-42. A worker's identity was resolved before its rate limit was checked. Closed in the twenty-first and twenty-second audits.**
- **Context:** The worker extractor looked a presented key or UUID up in the
  database before any limit applied.
- **Problem:** Unmetered lookups for credentials that match nothing.
- **Options considered:**
  - charge an address's bucket only for misses. This locked valid workers
    behind a shared address out along with a misbehaving neighbour.
  - charge each credential its own bucket, and the address's only for a
    credential not recently resolved.
- **Option implemented:** The second, `ratelimit::CredentialGate`.
  - Every presented key or UUID is charged its own bucket before the lookup.
  - One that has not resolved in the last ten minutes also pays its address's
    bucket (5 a second, burst 100), match or not.
  - A credential that resolved recently skips the address's bucket.
- **Justification:** A misbehaving machine behind a shared address does not
  lock out the workers beside it. What remains: after a restart nothing is
  known, so more than a hundred machines behind one address are admitted five
  a second.

**KL-43. `?rack=` canonicalises by Unicode code point.**
- **Context:** The rack lookup sorts the typed rack before searching.
- **Problem:** Code-point order equals machine-letter order for English, but not
  for a distribution whose letters are outside ASCII or longer than one
  character. There, a lookup would miss.
- **Options considered:** None recorded.
- **Option implemented:** Code-point order.
- **Justification:** No such opening-rack job exists.
- **Revisited (October 2026 audit):** not a limit. An opening-rack job's racks
  are spelt by `RackIndex`, whose tiles `LetterDistribution::parse` sorts by
  character, so a stored rack is in code-point order too -- German's `Ä` and
  Polish `Ą` after `Z`, the blank first -- and the lookup finds it however it
  is typed. A distribution with a multi-character letter has no rack space at
  all (`RackIndex::new` refuses it). Machine-letter order is the order of a
  *captured* position's rack (`canonical_rack`), which the positions search
  uses. The lookup now spells the typed rack with the index's own
  `RackIndex::spelling`, so the two cannot drift (`U-RACK-12`).

**KL-44. Concurrent imports each hold a ~190 MB tarball in memory, and more.**
- **Context:** An input-data import buffers its tarball, and the walk keeps every
  lexicon and leaves file's bytes until they are uploaded: most of the current
  release's 250 MB uncompressed (the thirty-first audit's count; this entry said
  a 94 MB tarball alone).
- **Problem:** Some 450 MB an import at the current release's size: two admins
  importing at once on a 2 GB task is close, three is not. The same task also
  runs MAGPIE's `convert rackequity2klv` for every leave transition and every
  "Check artifacts" rebuild, 375 MB at its peak for English, with nothing
  bounding how many run at once (thirty-second audit); an import, the 192 MiB
  large-result budget and two or three conversions together approach 2 GB.
- **Options considered:** None recorded for imports; a semaphore of one or two
  around KLV conversions.
- **Option implemented:** None.
- **Justification:** Imports are admin-only.

**KL-70. Small things in input-data import.**
- **Context:** `inputdata.rs`, the admin import page (thirty-first audit).
- **Problem:**
  - The page's "files hashed" stays at 0 while an import runs (entries are
    written only once staging succeeds), and its byte count stops during the
    walk and the upload.
  - Each network chunk of the download awaits its own progress `UPDATE`: some
    thousands of serial round trips per tarball.
  - The audit log's `input_data.import_staged` is written when the request
    starts, failed imports included, and names no date, ref or commit;
    `input_data.deleted` names only the id of a row that no longer exists.
  - A `known` row an admin deletes between staging and confirming is not
    inserted again, though the diff showed it as present (thirty-second
    audit).
  - "From data-X or later" is wrong for a file shared with an older release
    imported after a newer one; and a job declined for a player's `.kwg` or
    `.klv2` from a newer release than its distribution tells the worker the
    distribution's date (MAGPIE says "or later", so weakly).
  - Deleting an input row leaves its `inputs/{sha}` object in the bucket, as a
    cancelled import does: content-addressed, reused by a re-import.
  - Two pinned paths differing only in case (`NWL23.kwg`, `nwl23.kwg`) are two
    rows; a worker on a case-insensitive filesystem keeps one and declines the
    other (reasoned, not reproduced).
  - Two imports staged at the same time each label a path `new` that becomes a
    collision once the other confirms, so the second admin misses the second
    look; a staged import's expiry counts from the request, not from staging.
  - An import whose time limit fires while its staging transaction commits can
    commit anyway with the row already `failed`; its staged rows are then never
    expired (expiry takes `staged` rows only). The window is milliseconds.
  - An import past its time limit is failed, but the archive walk, on the
    blocking pool, runs to its end with the tarball and its files in memory: an
    admin starting it again at once holds two.
- **Options considered:** write the entry count as the walk goes; throttle the
  progress update to one a second; audit at staging and failure with the
  commit; re-derive collision labels at confirmation; a cancellation flag the
  walk checks between entries.
- **Option implemented:** None.
- **Justification:** Admin-only and rare; where one changes what is stored, it is
  a row an admin sees and can import again.

**KL-71. A claim's "update MAGPIE" message covers more than old clients.**
- **Context:** `claim_task` rewrites a `400` from reading its body into "a task
  claim must carry a JSON body … update MAGPIE", the message an old MAGPIE's
  contributor sees.
- **Problem:** A body the server could not read (the client disconnected) or a
  missing content type gets the same advice, which is wrong for them. And a
  first claim sent without a length and cut off at 16 KiB gets the generic
  "larger than this endpoint accepts" rather than the claim's own "send the
  X-Worker-UUID" (MAGPIE always sends a length).
- **Options considered:** rewrite only a body that parses and lacks the fields.
- **Option implemented:** None; `413` and `503` are passed on as they are since
  the thirty-first audit.
- **Justification:** Cosmetic: the client that gets it has already gone, or
  sends no content type, which only an old MAGPIE does.

**KL-72. What exports leave behind, and what they do not say.**
- **Context:** `exports.rs`, the admin job page, the artifacts bucket (thirty-
  first audit, third pass).
- **Problem:**
  - Deleting a job leaves its export objects in the bucket until the 30-day
    rule (a purge removes them); the bucket is versioned and no rule removes
    expired delete markers; `failed` and expired `job_exports` rows are never
    pruned; a build ended by its time limit or a restart leaves its multipart
    upload to the seven-day rule.
  - An opening-rack export of a consensus job has one record per analysis of
    a rack; the job's progress table (`opening_rack_progress`) says which
    move each rack settled on.
  - After a point-in-time restore (§1), a `ready` row whose objects a later
    purge deleted redirects to a 404.
  - The admin page shows only the newest export row, so a failed or running
    export hides an older ready one that still downloads.
  - Presigned links are signed with the task role's temporary credentials and
    may expire before the page's "valid for an hour".
- **Options considered:** collect a deleted job's keys as a purge does; a
  lifecycle rule for delete markers and a prune of old rows; mark the first
  result in the export; list every unexpired export on the page.
- **Option implemented:** None.
- **Justification:** Storage and wording, with no wrong data served.

**KL-73. The contributor-instruction test catches one phrasing.**
- **Context:** `F-DOCS-1` (`contributeDocs.test.ts`) reads the pages' source.
- **Problem:** Its guard against putting `contribute.txt` beside the binary
  matches one form of words; another phrasing would pass.
- **Options considered:** render the pages and test the text.
- **Option implemented:** None.
- **Justification:** The pages render no component tests today (tier 1F is
  plain TypeScript); the journeys (tier 5) read the account page.

**KL-74. The rating fit biased large or thinly linked groups, and understated their errors.** Closed in the thirty-second audit.
- **Context:** `stats/bradley_terry.rs`: the fit was MM iteration capped at
  10,000, with a prior of two virtual draws for every member against the
  anchor's strength, and standard errors from the diagonal of the information
  (opened in the thirty-first audit, pass 4).
- **Problem:** The prior's pull added up across any group of configs joined to
  the anchor only through a few links, and MM converged very slowly on a group
  that moves together. Measured with noiseless evidence: a 12-member cluster
  linked by one 300-pair job was stored 42 Elo low with a shown error of ±1.9
  (the full-covariance error is about ±28), unconverged; a 20-config chain's
  top 36.5 Elo low; a 30-member cluster about 200 low. A well-played island
  with no path to the anchor held the fit at the cap and marked the whole pool
  unconverged.
- **Options considered:**
  - a Newton / IRLS solve on log-strengths with the full Hessian;
  - a much weaker prior, or one applied only to configs with a perfect or zero
    score, so it no longer ties every member to the anchor;
  - a prior spread over each config's actual opponents;
  - standard errors from the inverse of the full information matrix.
- **Option implemented:** All three parts, in the thirty-second audit (pass 1),
  the prior and its errors after nine versions that each failed an adversarial
  check (AUDIT_FINDINGS_28, 1.K). The fit
  is Newton's method with a backtracking line search on a strictly concave
  objective, each step bounded at 8 natural-log units (about 1,400 Elo), a step
  under 0.5 taken whole, convergence judged on the full step, and damping if
  the curvature fails to factor. The prior is virtual drawn games against the
  pool's centre, the plain mean of every rating, on a logistic twice as wide as
  real games', two for a config with no games and `2 / (1 + g/200)`, at least
  0.2, for one with `g`. Errors are the diagonal of the inverse of the full
  information over the anchor's component, widened by one and a half times the
  prior's one-step pull on each config. `U-STATS-5` pins the
  shapes above and the ones the adversarial checks found; runs record
  `method = 'bradley_terry_newton'`.
- **Justification:** Each version tried in the audit pulled some shape of pool:
  - two draws per config toward the anchor (the old prior), converged: a field
    400 Elo from the anchor some 200 low, a 12-rung ladder's top 390 low;
  - two per config spread over its opponents, split evenly and then by the
    smaller degree: a newcomer in a star halved, then a config over a gauntlet
    of lightly played opponents 2 to 4 errors low;
  - virtual draws only where the maximum likelihood diverges: a 200-Elo jump
    when a newcomer conceded a quarter point;
  - Firth's penalty: not concave, so a config between far-apart opponents had
    two answers and a quarter point moved it 600 to 900 Elo;
  - two draws per config against a fitted centre, not faded: a strong tier
    joined to the rest by one job, 345 Elo and nearly four errors low;
  - faded, against a centre fitted as a strength of its own: in a mature pool
    the centre followed the newcomer, whose virtual games alone had not faded,
    and a 3-pair sweep rose 460 Elo as the rest of the pool played on; and,
    faded to nothing, a million-pair sweep left no curvature and a fit at the
    answer was stored as not converged;
  - against the mean rating, floored at a twentieth: a block that swept, or was
    swept by, everything outside it floated thousands of Elo on unrelated
    configs' pulls, every error infinite;
  - floored at a fifth, with errors from the games alone: tiers of lightly
    played configs 400 to 800 Elo apart, joined by one small job, 1 to 3 errors
    low, their 95% intervals covering the truth 30 to 65% of the time.

  The pool-centre prior was compared with the old one in a Monte Carlo of every
  shape found (gauntlet, a hub with twenty leaves, twenty shared swept
  baselines, young round-robins and stars centred and 400 away, newcomers,
  a config between far-apart opponents, two ladders, two tiers): it biases
  every shape less than the old prior did (a 12-rung ladder's top some 40 low
  against 390; two thin tiers 600 apart some 260 against 510), but no weighting
  removes the pull that adds up across a thinly joined group, so the errors
  carry it instead, and the intervals cover the truth again. KL-79 has the
  figures.
  Concavity is what makes it one answer, continuous and monotone; a centre at
  the mean of the ratings keeps a far field in place and does not move with a
  pool's maturity; the wider scale and the fade keep the pulls small, at the
  price of more swing for a barely played config (KL-79).

**KL-75. Small things in rating pools.**
- **Context:** `ratings.rs`, `routes/ratings.rs` (thirty-first audit, pass 4).
- **Problem:**
  - History is thinned to 500 points by count, not by time: a year-old active
    pool gives the eleven months before the last one some eight points, where
    the thinning keeps a run a day (the endpoint's API callers are all that
    read it since the page's history chart went).
  - A pool's scope is its variant, distribution and layout, not the job-level
    `bingo_bonus` and `sim_cutoff`. Both have been job-creation body fields
    since October 2026, with inputs on the new-job form, so a pool can already
    hold pairs jobs played under different bingo bonuses or sim cutoffs —
    different rule sets — and fit them as one, with nothing on the pool page
    saying so (`build_matrix` and the sweep's `evidence_games` filter neither).
  - Adding a member already in the pool writes an audit row and refits
    (removing one that is not is a `404`, since the thirty-second audit).
  - A pool's first fit by the thirty-second audit's method moves every rating
    (a different prior), and the history draws the jump with no mark; the API
    does not expose a run's `method`.
- **Options considered:** thin by time buckets; add the two settings to the
  pool's scope (two `rating_pools` columns, matched in both evidence queries
  and shown on the pool pages); pin them at job creation to MAGPIE's defaults
  for a pairs job, keeping pools simple but admins unable to run a variant;
  answer a no-op membership change without a write; expose `method` on
  history points and mark a change.
- **Option implemented:** None.
- **Justification:** The other three change no rating. The scope bullet was
  accepted when the two settings could only come from `magpie_defaults`, which
  had not moved; that no longer holds (thirty-third audit), and whether pools
  scope by the settings or job creation pins them awaits an owner decision.

**KL-76. The phone layout's remaining edges.**
- **Context:** The site at phone width (E-10), after the thirty-first audit
  wrapped the header and put wide tables in scrolling boxes.
- **Problem:** Header links are 20 px tall, 24 px apart when they wrap at
  280 px, which is borderline for WCAG 2.5.8. (E-10 now answers the job page and
  both rankings with a 32-character name of wide letters, pass 12, but measures
  at 393 px only: at 320 px its twelve-digit count puts the job page's
  contributor table 2 px past its box, which a realistic count does not.) A
  pool's ratings table at 320 px scrolls in its box, the rating column partly
  off screen. And the rating charts keep fixed
  margins for names and labels (204 px around the dot plot, 184 px around the
  history), so at 320 px the dot plot's scale is 34 px wide and its tick labels
  run together, and at 280 px four configs 600 Elo apart sit on one another; the
  page does not scroll sideways, so E-10 passes (thirty-second audit).
  A clamped error bar (±400 Elo drawn) has no mark, so ±400, ±1,278 and ±∞
  look the same while the caption says the bars are one error; the prior's
  widened errors make clamped bars more common (thirty-second audit, pass 2).
  The charts' name labels are shortened by character count, not measured: a
  name in wide capitals (`NWL23-WMWM-4PLY-…`) still runs past its margin
  (pass 25).
- **Options considered:** larger tap targets; a 320 px run of E-10; narrower
  chart margins below the `sm`
  breakpoint, or names stacked above the dots; an open end on a clamped bar.
- **Option implemented:** None.
- **Justification:** Checked by hand in the audit's sweep (744 page loads, none
  wider than the screen, and pass 13's 156, one spill fixed).

**KL-77. Where the archive walk and GNU tar still part, failing safe.**
- **Context:** `inputdata::walk_archive`, compared with `tar -xzf` over many
  crafted archives in the thirty-first audit's last passes; every real format
  tried (GNU tar's gnu, oldgnu, ustar, posix and v7, with extended attributes;
  Python's gnu and pax) walks and names files as `tar -t` does.
- **Problem:**
  - A tarball with no end-of-archive blocks, followed by zero padding after its
    gzip member, is refused ("invalid gzip header") since the walk reads every
    member; GNU tar extracts it.
  - A symlink alias is resolved lexically: a target `CSW24.kwg/` or
    `CSW24.kwg/../CSW24.kwg` pins the alias, where on disk it fails.
- **Options considered:** stop at a member of zeros; resolve links against the
  archive's directory tree.
- **Option implemented:** None.
- **Justification:** No real writer produces either, and both fail safe: a
  worker hashes what it extracts and declines a mismatch.

**KL-78. Small things found in the thirty-second audit and left.**
- **Context:** The job pages, the purge path, the worker list (thirty-second
  audit, pass 1).
- **Problem:**
  - A games job force-completed by an admin has no stored verdict, so its
    test panel showed the live status — "running … not acted on until N pairs
    are complete" — on a completed job. (Since fixed: the Significance Test card
    says "not decided: the job was completed before the test was".)
  - Two purge races were reasoned about and not reproduced: a leave universe
    seeding spawned in the microseconds before a purge commits could seed a
    generation of the purged job; and a leave transition running when its job
    is purged still uploads its KLV to the generation's key, which a re-run's
    same generation shares, so an upload hung for hours could land over the
    re-run's.
  - Deleting a player config reads every row of `game_requests` (twice),
    `opening_rack_requests`, `player_config_ratings` and
    `rating_run_residuals` (twice) for its foreign-key checks, none of those
    columns being indexed: 1.9 s warm at 4 million requests and 818 MB of
    residuals, for a config nothing references; admin-only and rare.
  - `GET /api/workers` answers a page past the end without its query now, but
    a page just short of the end still reads `offset + limit` rows of each
    arm's index: about 0.3 s at 300,000 contributing anonymous identities.
  - The admin audit log has gaps the thirty-second audit left: `job.activated`
    does not record the allocation, so an allocation change logs no values;
    `job.deleted` and its census have `job_id` null, so a job's history
    filtered by job misses its deletion; completing a completed job (or
    deactivating an inactive one) succeeds and logs a no-op transition;
    rating-pool membership rows do not name the pool; activating a completed
    job queues its derived builds before it answers `409`. The job pages show
    an inactive or completed job's old allocation beside its status.
  - The SPRT, on a run with no variance — every pair the same outcome —
    computed an LLR of 0 whatever the mean, so a thousand straight wins ran to
    `max_units`. (Closed with the SPRT: the match test's interval keeps a
    width when no variance is observed, `sqrt(2·ln(1/α) / (n²·ρ²))`, so a
    thousand straight wins decide.)
  - One hostile leave result can hold a rack's count up to `num_games` × 1,000
    occurrences at a mean of ±5,000 — the plausibility ceilings — which fixes
    that rack's mean and puts it at target for good, shifting its sub-leaves.
    A task has one result (KL-14), so nothing cross-checks it;
    outside the broken-client threat model the checks are built for (not
    reproduced).
  - An admin's "merge now", or an export settling a leave job, waits for a
    merge turn as well as its own job's lock: behind the two merges running and
    any other job's merges already waiting (turns go first come, first
    served), each a couple of minutes on the default database.
  - An admin's change made just before a live push's build that then fails
    (not a `404`) waits out the interval: the build spent its wake-up. Built
    again at once, a failing build would most likely fail again.
  - A live push overtaken by a newer page build (not an admin action) builds
    again, up to three times, rather than sending the newer payload the cache
    holds, and a page view in the gap between the cache's expiry and the next
    push builds one of its own (so a watched and visited job can cost two
    builds an interval; not measured); and when a job's stream ends for good (a `404` after a delete) its
    page goes on showing the last stats with nothing to say the job is gone.
- **Options considered:** label a force-completed job's test panel as such;
  publish the cached payload when a push is overtaken; an `onGone` callback
  that tells the page;
  drop the per-task request tables' foreign keys to player configs (the job's
  config already pins them), or index them;
  re-check the purge counter inside the seeding and the transition's upload;
  a cursor for the worker list, as the results feed has.
- **Option implemented:** None.
- **Justification:** None corrupts a stored result; the races need a window of
  microseconds or an hours-long hung upload, and the list's cost needs hundreds
  of thousands of identities (KL-38, KL-56).

**KL-79. What the rating fit's prior still costs.**
- **Context:** `stats/bradley_terry.rs`: virtual draws against the pool's
  centre (the mean rating), on a logistic twice as wide as real games',
  `2 / (1 + g/200)` for a config with `g` games, at least 0.2; each error
  widened by one and a half times the prior's one-step shift (thirty-second
  audit, KL-74). The
  figures are the audit's adversarial check's, noiseless and Monte Carlo,
  before the widening unless said.
- **Problem:** Each config's pull levels off at a constant however far it is
  from the centre, and the pulls add up through whatever joins a group to the
  rest; no weighting tried removes that without moving it to another shape.
  - *Tiers of lightly played configs joined by one small job*, at realistic
    gaps: two tiers of 20, five pairs a head-to-head, one 20-pair link, 400
    Elo apart, the upper one error low; 600 apart, 1.9 errors (its 95%
    interval covered the truth 65% of the time); 800 apart, ten a tier, 2.2
    (30%). Played at 100 pairs a head-to-head, under one error. With the pull
    in the error, these intervals cover the truth 95 to 100% of the time, the
    biases about one shown error (1.0 at 600, 1.3 to 1.4 at 800).
  - Ladders, the top: 12 rungs of 100 Elo over 100 pairs, some 35 low (a third
    of its error); 12 of 200, some 70 (half); 20 of 100, some 130 (nine
    tenths); 20 of 50 over 300, 15.
  - Mature tiers joined by one 300-pair job, by the lower tier's size and
    layout: at +600, 5 to 10; at +800, 20 to 65 (0.2 to 0.6 error); at +1,000,
    70 to 100 (about half); 20 at +1,200, 200 to 290 (1.2 to 1.6). Young tiers
    at +1,000: 1.6 to 2.6 errors. (All from the games' errors alone.)
  - A block that swept, or was swept by, everything outside it has its level
    set by the prior, with an error of hundreds of Elo; other young configs'
    pulls move it by a fraction of that.
  - A config barely played swings more between refits than under the old
    prior: one pair against each of two configs 1,000 Elo apart, RMSE about 170
    to 190 Elo against 105; a newcomer on 3 pairs, 140 against 160.
  - The widening is one Newton step, one and a half times over, which the
    check fitted to these tiers: at 1 it covered 64 to 86% at 800 Elo. Most
    errors grow a few percent (nine in ten under 2.5%); a thinly held
    config's, or a newcomer's clean sweep's, by a fifth to a third, and in rare
    cases more (one thinly held config at +1,600 went from 542 to 1,278).
  - About once in 500 fuzzed cases, a config taking a quarter point more moves
    its own rating the wrong way by under an Elo, its opponent's moving
    further the right way.
- **Options considered:** a narrower scale; no fade (a mature tier nearly four
  errors low); a fitted centre (moved with a pool's maturity); a floor of a
  twentieth (a swept block floated) or a half (tiers worse); the other priors
  KL-74 lists; flagging a config the prior holds rather than widening its
  error; refitting at half the prior to measure the pull.
- **Option implemented:** Draws against the mean at twice the scale, faded at
  200 games, floored at 0.2, and one and a half times the pull added to each
  error.
- **Justification:** The pulls are the thin-link limit of any prior that pulls
  config by config, and every version tried had them, most worse (the old
  prior held the 600 case 510 Elo low). What can be made honest is the error
  bar, and the widening brings those intervals back to 95%. Worth revisiting — with
  a flag on configs the prior holds, or pools built with more than one job
  between tiers — if tiers like these appear.

**KL-80. Small things in the backup pipeline, and a detached refit.**
- **Context:** `scripts/backup.sh`, `backup-drill-check.sh`,
  `restore-roundtrip.sh`, `infra/backup.tf`, and the rating refit's blocking
  thread (thirty-second audit, pass 2).
- **Problem:**
  - `backups.finished_at`, the admin page's "Took" and the `DurationSeconds`
    metric are taken before the upload, so they leave its time out.
  - Run as the container's PID 1 (`bash -c`), the scripts ignore SIGTERM, so an
    ECS StopTask ends them by SIGKILL without their EXIT trap: a stopped backup
    writes no `ok = false` row (the failure rule still fires). A `TERM` trap
    alone does not help while `pg_dump` or `pg_restore` runs in the
    foreground — bash defers it until the child exits, and the kill comes
    first (measured in `postgres:16`, exit 137, trap not run); the long steps
    would have to run in the background under `wait`.
  - `restore-roundtrip.sh` reads the source's row counts after the dump,
    outside its snapshot, so a writer on a live stack fails it (it wants an
    idle one, and says so); `backup.sh` takes them inside the snapshot.
  - A rating refit whose admin request is dropped releases its transaction
    and lock while the fit it started runs on to the end on the blocking pool,
    so a retry can run a second beside it (seconds each, only for a long
    ladder of sweeps).
  - A failure in `backup-drill-check.sh`'s step 2b leaves its
    `birdtest-backups-empty-*` MinIO bucket and host log files behind.
  - Two reasoned about and not reproduced: if CloudWatch aligns the staleness
    alarm's 12-hour periods to fixed boundaries, "36 hours" can be up to about
    45; and a snapshot session lost without a `FATAL` line is read as a value,
    though a later step then fails the backup.
- **Options considered:** a second timestamp after the upload for the row and
  the metric (the manifest keeps the first); the long steps backgrounded under
  `wait` with a `TERM` trap; the empty bucket in the check's cleanup; a
  staleness alarm on 1-hour periods.
- **Option implemented:** None.
- **Justification:** None loses a backup or passes a bad one.

**KL-81. Small things in the account lifecycle.**
- **Context:** `routes/auth.rs`, `email.rs` (thirty-second audit, pass 5).
- **Problem:**
  - A password reset scores and hashes the new password between reading its
    link and spending it, outside any transaction, so an account deleted or a
    link spent in between costs a wasted score and hash (the spend checks
    again). Hashed inside the transaction, it held the account's row — which
    the reset locks first, in the order a delete does — for the whole wait for
    a turn, up to ten seconds under a flood, against the account's own
    submissions; hashed before the link was read, a wrong link cost a run.
  - The SES client has no operation timeout, so if SES hangs, background sends
    accumulate (bounded by the rate limits on what sends them).
  - Resetting for a known address writes a token row that an unknown address
    does not: a statistical timing difference of a millisecond or so, beside
    KL-37's deterministic one (not measured).
  - A reset link refused a weak password still works, so it can be tried
    again: five scorings an hour per link (whoever sends them), after which the
    owner waits for the bucket too. Scoring a crafted password is close to a
    second, on two turns of its own; a flood of them makes registrations and
    resets answer `503`, never sign-ins.
  - The CSRF cookie is not `__Host-`-prefixed, so a sibling subdomain able to
    set cookies could plant one (cookie tossing); none exists, and the token
    is compared in constant time since pass 13.
  - A username is compared by `lower`, not by a normal form: `émile` composed
    and decomposed are two accounts that look alike. Normalizing new names
    only would lock out an existing decomposed one at sign-in.
- **Options considered:** hash inside the transaction; a timeout on the SES
  client; a dummy write for an unknown address; spending a link on a weak
  password; NFC-normalizing usernames at registration and sign-in.
- **Option implemented:** The link read first, then its own bucket (five
  scorings an hour), then the score and hash outside the transaction (passes 5
  and 6); UTF-8 on every SES part. Not built: the SES timeout, the dummy
  write, spending a link on a weak password (a typo would cost the owner the
  link), and normalizing usernames.
- **Justification:** Resets are limited per address, per caller and per link;
  the rest is bounded or moot while KL-37 stands.

**KL-82. Small things in startup, configuration and the background loops.**
- **Context:** `main.rs`, `config.rs`, `clientip.rs`, `bin/build-derived.rs`,
  `routes/auth.rs`, `routes/public.rs`, `infra/ecs.tf` (thirty-second audit,
  pass 12).
- **Problem:**
  - The single-instance invariant rests on the service's
    `deployment_maximum_percent = 100`. AWS counts tasks `RUNNING` or
    `PENDING` against it; whether a task already draining or stopping counts
    is not established (not reproduced — ECS cannot be run here). If it does
    not, a new task can start during the old one's 30 s of draining and up to
    120 s of shutdown, and its startup fails the old one's running imports and
    exports and releases its open transitions. While the two run, neither
    sees the other's `jobs::DispatchHolds`: a purge, a delete or a consensus
    edit on one does not keep the other's claims off the job (they wait out
    the dispatch lock on pool connections, as before holds existed), the
    other's submissions for its claims wait out their lock timeout rather
    than being answered at once, and the finish check's second purge witness
    (`claims_holds_taken`, `I-STATS-9d`) is blind to a purge made on the
    other. The scheduler's and
    the finish check's other in-memory state (`RECENTLY_BUSY`, the
    zero-generation `BUILDING` set, the debounce) is per process too.
  - The five background loops have no panic guard: a panic in a tick ends that
    loop until the next restart, with no alarm (no panic path found).
  - Registration and reset mails go out on detached tasks, which a graceful
    shutdown does not wait for: a `SIGTERM` just after a registration can lose
    its confirmation mail; the user can ask for another.
  - The admin NDJSON results stream is not ended by the shutdown signal, as
    SSE is: an open download holds the old task for up to the stop timeout.
  - A `TRUSTED_PROXY_HOPS` above the real number of proxies keys every per-IP
    limit on the peer — in production the ALB — so the site shares one bucket,
    silently.
  - The derived-file builder does not check its MAGPIE against
    `MIN_MAGPIE_VERSION` as the web task does. Both images are built from the
    same Dockerfile and MAGPIE pin, by convention: the Terraform checks only
    that `derived_builder_image` is set and has `backend_image`'s tag (KL-62).
- **Options considered:** a session-scoped advisory lock taken on a dedicated
  connection before the reapers, so a second process waits for the first to
  exit; `catch_unwind` around each tick; tracking the mail tasks for shutdown;
  ending the NDJSON stream on the shutdown signal; a warning when
  `X-Forwarded-For` routinely has fewer entries than configured; the version
  check in the builder.
- **Option implemented:** None of these (pass 12 took the address before the
  reapers, so a second process on the same host exits before touching the
  first one's work, and range-checked the heartbeat timeout and session TTL).
- **Justification:** The lock would make a new task wait, unhealthy, behind a
  draining one that ECS may already be waiting on; it is worth adding once the
  overlap is shown to happen. The rest are rare, bounded by a restart or a
  retry, or moot while the two images are built from one pin.

**KL-83. A heartbeat exchange is bounded by MAGPIE's hour, not by the heartbeat timeout.**
- **Context:** `~/MAGPIE/src/impl/contribute.c` (heartbeat loop),
  `src/compat/chttp.{h,c}`; `config.rs` (`HEARTBEAT_TIMEOUT_SECONDS`)
  (thirty-second audit, pass 13).
- **Problem:** MAGPIE sends one heartbeat attempt, waits for it to end, and
  sleeps thirty seconds. Its 120 s timeouts bound a connect and a stall, not
  the exchange, which may run an hour while it makes progress. The server
  records a heartbeat when it arrives, so on a link slow enough that one
  exchange takes longer than the timeout less thirty seconds, a live worker's
  claim lapses, its task goes to someone else, and its result is answered
  `accepted: false`. Shown with MAGPIE's own libcurl options against a server
  that stalls 110 s twice mid-reply: every heartbeat succeeded, 250 s apart.
  A connect of 120 s plus a stall of 120 s exceeds even the default 300 s.
- **Options considered:** a whole-exchange bound on MAGPIE's heartbeat (a
  minute, say) so "thirty seconds plus the bound" is the true worst case, and
  the floor set from it; a floor of an hour, which would leave a dead
  worker's task unclaimed for as long.
- **Option implemented:** None. The floor (180 s) keeps a timeout above
  MAGPIE's cadence and nothing more: one heartbeat stalled until libcurl gives
  up (about 127 s, its speed averaged over a few seconds) leaves some 187 s
  between recorded heartbeats, past the floor; the default, 300 s, covers it.
- **Justification:** A link that needs minutes to move a heartbeat's fifty
  bytes is, for scheduling, a worker that is gone. The MAGPIE bound is the
  fix, with a release and a `MIN_MAGPIE_VERSION` bump; it is left for a
  MAGPIE change of its own.

**KL-84. A worker's first request over plain http discloses its credential.**
- **Context:** `~/MAGPIE/src/ent/client_state.c` (`server` accepts any
  scheme), `src/compat/chttp.c` (follows redirects); `infra/ecs.tf`
  (`aws_lb_listener_rule.http_api_refused`) (thirty-second audit, pass 13).
- **Problem:** A contributor who writes `server http://…` sends their API key
  or anonymous UUID in the clear. The load balancer now answers `/api/*` on
  port 80 with `426` and a message instead of redirecting, so MAGPIE stops
  after that first request (shown against a stand-in: "claiming a task failed
  with HTTP 426: birdtest's API is served over https only…") — but that
  request has gone. Before, the redirect was followed on every request: an
  anonymous worker worked, its UUID in the clear each time; a keyed one lost
  the `Authorization` header at the scheme change, was treated as identity-less
  and lost its results, still reporting "an authenticated worker".
  Separately (not reproduced): if the ALB ever passed a client's own second
  `X-Forwarded-For` header after the one it appends to, the rightmost entry
  would be the client's choice; tested only against nginx, which rewrites the
  header into one.
- **Options considered:** MAGPIE refusing a non-`https` server other than a
  loopback address, and `CURLOPT_REDIR_PROTOCOLS` limited to https so no
  redirect can downgrade; the refusal at the load balancer.
- **Option implemented:** The load balancer's refusal. The MAGPIE check is the
  real fix and needs a MAGPIE release and a `MIN_MAGPIE_VERSION` bump (tier 6
  talks to `http://localhost:8080`, which a loopback exception keeps working).
- **Justification:** One request's disclosure, loudly reported, where every
  request's was silent; the message says to revoke a key sent that way. (The
  `426` goes without the `Upgrade` header RFC 9110 asks of it — a fixed
  response cannot set headers; libcurl and browsers show the body regardless.)

**KL-85. The worker trusts its server's sizes.**
- **Context:** MAGPIE's `chttp.c` (`write_callback`), the games,
  opening-rack and leave-generation executors in `config.c`
  (thirty-second audit, pass 14; AUDIT_FINDINGS_19 §4 weighed it first).
- **Problem:** Nothing on the worker caps what the server asks for: a response
  body grows without limit, and `num_games`, `num_plays` and the recorded-play
  counts are checked only for being positive. A compromised or buggy server
  ends its workers' runs — shown under `ulimit -v`: a 2 GiB artifact, a
  `num_plays` of two billion, and a games task of 10^15 with positions
  captured each end in a failed allocation and an abort, with no decline, so
  the claim lapses and the next worker dies the same way. Distinct
  `previous_artifact_sha256` values and pinned rack tables also grow the data
  directory without limit.
- **Options considered:** a cap on response bodies in `chttp`
  (`CURLOPT_MAXFILESIZE_LARGE` and a limit in the write callback); upper bounds
  in `config_contribute_*` matching the server's own (plays, recorded plays,
  plies, a games and racks ceiling), refused as a server error so the task is
  declined.
- **Option implemented:** None: the server validates every size it hands out
  (pass 3.2 of this audit), and a server that does not is already in a
  position to waste its workers' time.
- **Justification:** The trust model is the one AUDIT_FINDINGS_19 chose;
  PLAN had said otherwise until pass 14. Worth doing with the next MAGPIE
  release that touches the executors.

**KL-86. Small things on the admin pages.**
- **Context:** `frontend/src/routes/admin/` (thirty-second audit, pass 16).
- **Problem:**
  - A static player's simulation-only fields stay editable on the new-config
    form and are sent as null.
  - "Fetch and diff" stays enabled while an import runs; a second start drops
    the first from the page, which then stages and expires unseen (there is
    no list of imports).
  - Confirming an import that expired while staged answers `409`, and the
    page keeps showing it staged until reloaded (reasoned, not reproduced).
  - A rating pool's member is removed without a confirmation.
  - An export poll that fails keeps polling every three seconds until one
    succeeds (pass 16 made it poll again; before, it stopped for good).
  - A request that never settles leaves the job page's actions disabled until
    a reload (`request()` has no timeout; reasoned, not reproduced).
  - While the job page's export read fails, its poll and its retry both ask
    again; a poll already out can briefly put back "Building…" after a newer
    answer said ready (the next poll corrects it).
- **Options considered:** disabling the fields, the button while running, a
  reload on a `409`, a confirmation.
- **Option implemented:** None of these. Pass 16 made the job page's reads
  settle apart and retry, and pass 17 redesigned how it keeps state: a REST
  read is applied only if it is the newest and no live payload came while it
  was out, the allocation box is filled once and then only the admin's (the
  job's current value shown beside it), and a deleted job — found by a read,
  an action or the stream, pass 18 — stops the retries and disables its
  actions. Passes 16 and 17 also made every action on it take
  one click, trimmed a ban's target, confirmed an unban,
  required the job form's players, and named an input row's digest and its
  derived files in the delete confirmation.
- **Justification:** Admin-only, each visible and recoverable by a reload or
  a second action.

**KL-87. What the SPRT got wrong at the edges, and the odd games batch.** *The SPRT's two halves are closed: the match test replaced it (October 2026). The odd-batch half remains.*
- **Context:** `stats/sprt.rs` (since replaced by `stats/match_test.rs`),
  `routes/admin.rs::validate_job_body` (thirty-second audit, pass 18).
- **Problem:**
  - A `games` job created before pass 18 with an odd `games_per_batch` gave
    player 1 the first move in more than half its games — every one at a
    batch of 1 — so its verdict favours player 1 (about +42 Elo at a batch
    of 1, +14 at 3, +8 at 5). Nothing marks such a job. *(The
    schema's column default was 1 as well, so a direct `INSERT` that left the
    column out — a script, a test fixture — made one; since the thirty-third
    audit it is 2, and a CHECK holds the counts to what the API requires,
    `I-JOB-1f`. Evenness itself stays the API's rule alone: test fixtures play
    odd batches on purpose.)*
  - *(Closed.)* The SPRT's normal approximation overstated |LLR| when almost
    every pair was a split: `[0,0,10000,1,0]` at ±10 read 1151 where the exact
    GSPRT gives about 1.1, so one decisive pair among a few dozen splits could
    decide a job with no minimum. The match test's interval keeps a width
    when no variance is observed, so a run of splits decides nothing on its
    own.
  - *(Closed.)* With `min_units = 0` and wide Elo bounds the SPRT's type I
    error roughly doubled (about 10–12% against α = 5% at [0, 200]). The match
    test has no Elo bounds, needs a floor of at least 1, and keeps its error
    rate however often it is checked: simulated, equal players got a winner in
    at most α + 2% of runs (TESTING.md, `U-STATS-3b`).
- **Options considered:** refusing to activate a games job with an odd batch;
  fishtest-style pseudo-count regularization, or the exact MLE GSPRT; a floor
  on `min_units`; replacing the test.
- **Option implemented:** Pass 18 made new games jobs' batch even. October
  2026 replaced the SPRT with the match test
  ([Why a match test](#why-a-match-test-and-not-an-sprt)), which closes the
  second and third problems.
- **Justification:** Odd-batch jobs are found with `SELECT job_id FROM
  job_game_config WHERE games_per_batch % 2 = 1`; their verdicts should be
  read as biased and the jobs re-run.

**KL-88. Allocation is a share of claims, not of worker time.**
- **Context:** `scheduler.rs` (the deficit counts claims), the admin job page
  (thirty-second audit, pass 19).
- **Problem:** Every claim counts one, whatever its task costs, and task sizes
  differ by orders of magnitude across job types at the form's defaults (a
  games batch of 2 games; a leave task of thousands). At equal claim rates
  the job with the longer tasks holds nearly every worker: two jobs at 50/50
  with tasks of 30 and 1 time units split claims 750/750 and worker time
  96.8% / 3.2% (the real `scheduler::claim`, twenty workers in simulated time).
  Also: a leave job
  whose generation-0 KLV could not be built starts a new build on every claim
  that reaches it, with no backoff (the derived builds and templates have one).
  The decline skip covers MAGPIE's hand-back on `stop` (sent as
  `task_failed`), so a contributor who stops and restarts is not offered that
  task for an hour; and any template-load error, a transient pool timeout
  included, parks the job for sixty seconds for every worker. MAGPIE keeps a
  job's first place in its `unsupported_jobs` list when it declines it again,
  so a live job pushed past 200 entries in one run is dropped by the server
  (which keeps the newest 200) every time, offered and declined on every
  claim (it needs 200 distinct declines in one run).
- **Options considered:** weighting the counter by a task's estimated cost
  from its template (games per batch, twice the pairs, iterations, racks) —
  start-time fair queuing with packet lengths; stating the rule and asking
  admins to size tasks comparably; a backoff on the generation-0 build; a
  distinct decline reason for MAGPIE's stop; parking a template only on errors
  from its rows.
- **Option implemented:** Stated: PLAN's schema comment and design table, and
  the admin page's allocation field, say it is a share of claims.
- **Justification:** A cost estimate per job type is a design of its own, and
  wrong estimates would bend shares as badly; comparable task sizes are in the
  admin's hands today.

**KL-89. A job settles for an hour after joining, and a pass that is one worker's lifts it.**
- **Context:** `scheduler::join_at_parity`, `issue_claim`, `unsettle`,
  `lift_passed_over` (thirty-second audit, pass 20).
- **Problem:** A job joins at the lowest ratio among the jobs being served and
  is settled, claim by claim, level with each class of workers that runs
  faster. Six gaps remain. A class that makes no claim of it within an hour
  of its joining -- a few workers on long tasks, a class that comes online
  later -- is not settled against, and when it does claim it finds the job
  below its pace and gives it every claim until it has caught up. A worker
  that cannot run the job but does not say so -- it fails the task
  (`task_failed`) or vanishes -- leaves the settling its claim gave. And a job
  is lifted when a worker passes it over for want of a task, which may be that
  worker's alone -- a task it declined within the hour: a job only a minority
  can run, lagging by design, is lifted
  each time such a worker passes it over, and loses the priority its lag gave
  it. Settling also costs, for that hour, about 40% of the claims of a
  newcomer only a minority can run beside an older job everyone runs, when
  the minority's claims come in pairs: the first of a pair goes to the older
  job on the tie (119 of 200 in the hour, then 200). And a job passed over is
  lifted to where the job claimed stood in this request's list; if that job
  was purged or re-activated lower in the meantime, the lift overshoots it
  (it needs an admin action to race the claim). And a job whose dispatch lock
  stays held — no ordinary holder does; seedings, purges and deletes are
  skipped without a wait — lets the jobs beside it run a ratio unit ahead and
  then answers their workers `Idle`, each waiting its two seconds on the lock
  first: with a synthetic holder of 40 s and 100 workers, 187 claims idled
  where none had (the audit's pass 22, not reproduced with a real holder).
- **Options considered:** a longer settling window (it would settle, too, a
  job whose lag became structural while it lasted); recording which workers
  can run which job (the unsupported sets are the client's, and not stored);
  telling apart, in the handlers, a job with no task for anyone from one with
  none for this worker.
- **Option implemented:** None; the rule and its gaps are stated.
- **Justification:** Every class that can run a newly joined job below its
  pace claims it at once, so an hour is long; a worker that cannot run a job
  declines it with a reason, which undoes its settling; and the per-worker
  lift needs a worker that repeatedly finds nothing it may take in a job
  others take from.

**KL-90. A few actions still leave no audit row, and sign-in attempts no record at all.**
- **Context:** `audit_log` (thirty-second audit, pass 22).
- **Problem:** The release of an expired, unconfirmed account when its name or
  address is registered again deletes the row with no audit row (and, through
  `worker_bans`' cascade, any ban on it). An admin's rating-pool recompute and
  leave-merge, and the bulk results stream, write none either. And a ban's
  row says `target_type = 'worker'` for an account and an anonymous UUID
  alike: which it was is read from `worker_bans`, or from the id. And nothing
  records a sign-in attempt: not the audit log (by design, "Audit actions"),
  not the service's logs at their deployed level (`RUST_LOG=birdtest=info,tower_http=info`,
  where the HTTP trace is at debug), not the load balancer (no access logs) —
  the limiter refuses a guesser and nobody can see that it did.
- **Options considered:** a row for each; a `target_type` of `user` or
  `anon_worker` on bans.
- **Option implemented:** None; they are stated here.
- **Justification:** None of them destroys or grants anything an admin would
  need to reconstruct: an unconfirmed account never ran a task, a recompute
  and a merge are repeatable, a stream only reads (an export is logged), and
  the ban's identity kind is in the table beside it. A line per refused
  sign-in, or per bucket that trips, is a small change when an operator wants
  to watch for guessing; the limiter bounds it either way.

**KL-91. A visitor can still drive the bounce rate, and flood one mailbox slowly.**
- **Context:** Registration and password reset mail (`routes/auth.rs`),
  `infra/ses.tf` (thirty-second audit, pass 24).
- **Problem:** Registration mails a confirmation to any address it is given, at
  ten an hour per client address: one address sends some 240 a day to made-up
  domains, every one a bounce, and many addresses more. SES reviews an account
  at a 5% bounce rate and may pause its sending at 10%, which stops every
  confirmation and reset. Suppression makes each address bounce once, not
  once per mail, and the bounce-rate alarm fires at 4%; nothing stops the
  stream. Separately, one address can be sent five "already has an account"
  notices and five reset mails an hour, from any client addresses: some 240 a
  day to one person, indefinitely. The mail itself carries no per-message
  feedback: which addresses bounced is in SES's suppression list, not in
  birdtest. And the outbox file name that the end-to-end suite reads mail by
  maps `a.b@x` and `a-b@x` to one suffix (tests only; their addresses are
  unique). Sends are paced to the account's rate, in one queue in the order
  they came, so a burst from many client addresses no longer fails, but it
  queues: past the rate, everyone's mail — a real registrant's, a reset —
  waits behind it. The queue holds an hour of mail at the rate (20,000 at
  most); past that a mail is refused, and a reset that would go out after its
  link expired is dropped, each with the mail-failed alarm. So a flood from
  enough client addresses delays mail, and past an hour's worth loses it,
  loudly.
- **Options considered:** a daily cap per address as well as the hourly one; a
  site-wide cap on confirmation mail; refusing reserved domains (`.invalid`,
  `.test`) at registration; a configuration set with bounce and complaint
  events to SNS, marking an account's address as bouncing; a CAPTCHA.
- **Option implemented:** Only the alarms (bounce and complaint rates, any
  failed send), account-level suppression, and SES's reason in the log.
- **Justification:** A pause is loud now, and caught before SES acts; birdtest's
  registration volume is small enough that an operator reading the alarm can
  close registration or ask SES for a review. A site-wide cap or a CAPTCHA
  would turn the same attacker into one who blocks real registrations instead.
  Reserved domains are the few an attacker would not use.

**KL-92. What RUNBOOK §1's re-apply step does not handle.**
- **Context:** The step that re-applies, after a full restore, the security
  actions since the restore point (thirty-second audit, passes 22–24, and its
  check `scripts/reapply-check.sh`).
- **Problem:** It re-applies what the damaged instance's audit log records, so
  an action whose row is gone — deleted by someone with database access, or an
  action that writes none (KL-90) — is not re-applied; admin flags, which are
  not audited, are compared instead. It takes the damaged rows from an hour
  before the restored instance's newest one, so an action in a transaction
  that ran for more than an hour across the restore point is missed. It
  matches the two logs by id, and refuses — rather than reading nothing new —
  when an id names different rows on the two: a damaged log a migration
  renumbered, or the restored instance writing once repointed (so the step is
  not pasted again after that; a missed action is applied by hand). The
  review lives in the ops task's `/tmp` and is lost with the task. A password
  reset since the restore point is applied by copying the damaged instance's
  current hash, which a later change with no audit row (a migration) would
  have altered too: the apply lists every hash it copies. Every session ends,
  not only those that should.
- **Options considered:** exporting the whole audit log by id; writing the
  review to the restored database; ending only the sessions of accounts with
  actions since the restore point.
- **Option implemented:** None; they are stated here and in RUNBOOK §1.
- **Justification:** Each action it re-applies is one request's short
  transaction; to miss one, that request would have had to wait on a lock for
  an hour. An attacker who can delete audit rows can do anything the
  step would undo, and the damaged instance is kept, with a final snapshot,
  for exactly that investigation. The apply refuses without the review, so a
  lost one is redone rather than skipped. Signing everyone in again once is
  the price of not having to tell which sessions are safe.

**KL-93. A small batch caps a fast job at a claim a second per worker.**
- **Context:** The new-job form defaults a batch to one pair (two games), as
  do the API and the schema (`pairs_per_batch` 1, `games_per_batch` 2), and a
  credential may claim once a second, burst 5 (the claim row of the rate-limit
  table). A MAGPIE worker runs one task at a time and waits out a `429` for its
  `Retry-After` (thirty-third audit).
- **Problem:** A pair between static players takes milliseconds, so at the
  default a static or static-vs-light job's worker plays about one pair a
  second whatever its core count, and spends most of its time in back-off.
  Every task also costs fixed overhead: a claim transaction (about twelve
  statements, the job-row lock among them), a submission transaction, and a
  row each in `tasks`, `task_claims`, `game_requests` and `game_results`. A
  400,000-pair job at batch 1 is 400,000 of each, and the finish check (KL-10)
  and the rating sweep's `build_matrix` read every `game_results` row — the
  sweep every two minutes per pool with an active job. Nothing tells the admin
  to size a batch to a task's duration.
- **Options considered:**
  - defaults sized for about a minute of work for the configs chosen (the
    form can tell whether either player simulates or solves: say 100 pairs
    for static against static, 1 for a simming pair);
  - a note beside the batch field that a task under a second is capped by the
    claim limit;
  - per-job pentanomial running totals for the rating sweep only (display, so
    KL-10's objection to a counter the stopping rule trusts does not apply),
    making `build_matrix` O(jobs) rather than O(result rows).
- **Option implemented:** None yet. *Open, awaiting an owner decision
  (thirty-third audit).*
- **Justification:** A larger default batch trades per-task overhead against
  overshoot past a test's stopping point and the work lost when a claim lapses;
  which way a default should lean is the owner's call, and the sweep's counters
  are a schema addition.

**KL-94. An export's snapshot holds back vacuum for as long as the export runs.**
- **Context:** An export reads its results and positions in one `REPEATABLE
  READ`, read-only transaction, which is what makes its "final" marker sound
  ("Final is decided inside the snapshot"); a build may run up to six hours
  (`EXPORT_BUILD_LIMIT`), and two may run at once (thirty-third audit).
- **Problem:** As with the nightly dump (KL-7), the snapshot pins the vacuum
  horizon for the whole read: `task_claims` heartbeats go non-HOT once their
  pages fill (an entry in every index, twice a minute per claim in flight); a
  leave merge's rewrite of a generation, some 3.2 million dead tuples and
  430 MB, cannot be vacuumed; and submissions' dead `tasks`, `jobs` and
  counter-row versions accumulate. An export that overlaps the nightly dump
  extends the window.
- **Options considered:** keyset-paginate the read in short transactions,
  with an explicit witness of finality (`jobs.claims_issued` and the job's
  status, read at both ends) in place of the single snapshot; leave it.
- **Option implemented:** None.
- **Justification:** A full opening-rack export reads in minutes, not hours,
  so the window is usually short; replacing the snapshot is a design change to
  what makes "final" trustworthy, and is worth making only if a full-corpus
  export ever takes hours.

**KL-59. A failed sign-out leaves the session live.**
- **Context:** `lib/auth.ts` set the store to `null` in a `finally`, and the
  layout's sign-out had no error path (`F-AUTH-2` pinned the store's side).
- **Problem:** On a `503` or a network failure the server never cleared the
  session cookie, yet the page showed the user signed out; the next page load
  signed them back in. On a shared machine that matters.
- **Options considered:** keep the session and say "sign-out failed"; retry.
- **Option implemented:** The first (thirty-third audit, pass 3). A failed
  logout asks `/api/me` again rather than clear the store, so a live session
  stays signed in (and one the server no longer knows shows signed out), and
  the layout says the sign-out failed and to try again, staying on the page.
- **Justification:** Only the logout's response can remove the HttpOnly
  cookie, so the page must not claim what the server has not done. A retry
  loop would outlast the user's attention; a message they can act on does not.

**KL-60. Admin UI gaps the API covers.**
- **Context:** The job form (`/admin/jobs/new`), the player-config pages and the
  password form.
- **Problem:**
  - The job form sends no `racks_per_batch` or `rack_size`, so opening-rack
    jobs of another rack size or batch can be made only through the API.
  - A blank password gets a generic `400` rather than a field error (the form's
    `required` normally stops it first).
  - The job form loads MAGPIE's version with the player configs and input
    data, so its failure is reported as "Could not load player configs and
    input data" (thirty-second audit).
  - A duplicate config name gets a generic `409` with no field error.
    Creating a config writes no audit row, where creating a job or a pool does.
  - Narrowed by the thirty-third audit, pass 3: the job form now sends
    `capture_positions` (and `capture_first_divergence`), `/player-configs/[id]`
    shows every setting of a config, and the job form's distribution and
    layout start empty and must be chosen, so none of those is a gap any more.
  - The job and player-config forms show a server refusal as one line at the
    foot of the form, by API field name (`layout_id`, `confidence_pct`), not at the
    input under its label; and job creation checks the layout only once the
    other settings pass, so an admin can fix one error and meet a second.
  - The job form's α and β took any value the server did (`step="any"`), so
    their spinners stepped by one and were of no use. (Gone with the SPRT: the
    form has no α or β, and no number box has spinner arrows.)
  - A leave job whose generation-0 KLV fails to build after the job row
    commits is answered `500`, but exists; the form stays, and a second click
    makes a second job.
- **Options considered:** add the fields; map zxcvbn's blank-password error;
  load the version separately; a config page; a `name` field error on the
  conflict; a `player_config.created` audit row; answer a failed KLV build
  with the created job and a warning.
- **Option implemented:** None.
- **Justification:** Admin-only, and the API is documented.

**KL-61. Marking every pool for a refit can still skip one.**
- **Context:** After `mark_every_pool_for_refit`, the sweep skips a refit when
  `pairs_used` and the members are unchanged.
- **Problem:** A deleted job's pairs replaced by the same number of new ones
  within one 120-second sweep leaves a pool fitted on evidence it no longer
  has, until the next new pair.
- **Options considered:** skip the "same pairs" shortcut when the last run's
  `evidence_games` is NULL.
- **Option implemented:** None.
- **Justification:** It needs a coincidence and heals itself.

### Deployment

**KL-45. Terraform's state is local.**
- **Context:** `infra/main.tf` declares no backend, so the state is a file on the
  machine that applied. RUNBOOK.md's recovery steps and both ops scripts read
  it.
- **Problem:** If the file is lost with that machine, the scripts can no longer
  reach the database by SQL, and the next apply tries to create every named
  resource again.
- **Options considered:** an S3 backend in a second region, versioned and
  locked, with a bootstrap bucket outside this configuration.
- **Option implemented:** None yet. *Open (fifteenth audit).* README.md says to
  keep the state off the machine and out of the region.
- **Justification:** The backend's region and account are the operator's
  choice.

**KL-46. The restore drill's disk stops at Fargate's 200 GiB.**
- **Context:** 200 GiB holds a database a little over 150 GiB with its dump
  beside it. The ops task's scratch restore (RUNBOOK.md §2.1) has the same
  ceiling.
- **Problem:** A larger database cannot be drilled, or scratch-restored, on
  Fargate.
- **Options considered:**
  - a hand-run drill with `DRILL_TARGET=server` against a scratch instance
    restored from a snapshot;
  - a PITR scratch instance for §2.1.
- **Option implemented:** Past the limit, the drill refuses to start and names
  the alternative.
- **Justification:** 200 GiB is Fargate's maximum ephemeral storage.

**KL-47. Fargate's provisioning time is the floor on a deployment's gap.**
- **Context:** A minute or so that Terraform cannot shorten.
- **Problem:** Every deploy has a gap at least that long.
- **Options considered:** two tasks alive at once, the
  [primary/secondary split](#scaling).
- **Option implemented:** None.
- **Justification:** Two live tasks break the single-instance assumptions the
  system was written against.

**KL-62. Terraform does not check that its variables agree.**
- **Context:** `infra/*.tf` variables.
- **Problem:**
  - `backend_image` and `derived_builder_image` are two free strings. A
    release that moves a builder version in one and not the other leaves rows
    keyed to the web task's builder that nothing builds, and jobs needing them
    wait, visible only on `/admin/derived-data`.
  - `acm_certificate_arn` is not checked to be an ACM ARN in the stack's
    region; a wrong one fails the apply half-way.
  - `mail_from_address` is not checked to be within `ses_domain`; outside it,
    every mail fails.
- **Options considered:** cross-variable `validation` blocks (Terraform 1.9).
- **Option implemented:** None at first (thirty-first audit). *Closed (thirty-third
  audit, pass 1):* the three validations, and `infra/tests/variables.tftest.hcl`
  (`S-TF-1`, `S-TF-2`), which CI's `terraform` job runs. `derived_builder_image`
  must have `backend_image`'s tag (a missing tag is `latest`; an image named by
  digest is let through, having none to compare); `acm_certificate_arn` must be
  an ACM ARN in `region`; `mail_from_address` must be at `ses_domain` or a
  subdomain of it. Pass 4 added `github_token_parameter_arn`, which none of
  the three covered: empty, or an SSM parameter ARN in `region`. A parameter's
  name went into the execution role's policy, which IAM refused half-way
  through the apply; another region's ARN applied, and then no task could read
  its secret. README "Deploying" now says how to make the parameter (default
  `aws/ssm` key: the execution role has no `kms:Decrypt`).
- **Justification:** The reason for leaving them was that `terraform validate`
  does not exercise a validation's condition: only a plan against real values
  shows it refuses the wrong ones and not the right, and one that refuses a
  correct apply is worse than none. `terraform test` (1.7 and later) plans
  against a mock AWS provider with no credentials, so each validation, these
  three and every single-variable one before them, now has a plan it must
  refuse and one it must accept. Still unchecked: that two images at one tag
  carry one MAGPIE (a convention of the release, README "Deploying").

**KL-63. Secrets that reach logs or subprocesses.**
- **Context:** Nginx's access log, MAGPIE subprocesses, the database URL.
- **Problem:**
  - Nginx logs `/confirm-email?code=…` and `/reset-password/confirm?token=…`
    to CloudWatch for 30 days; a reset token opened but unused stays valid for
    30 minutes.
  - MAGPIE subprocesses inherit the backend's whole environment,
    `DATABASE_URL`, `SESSION_SIGNING_KEY` and `GITHUB_TOKEN` included, while
    parsing lexica fetched from the network.
  - The backend does not verify the database's certificate: without
    `sslmode=verify-full` the connection is encrypted but not authenticated.
  - Only a `500` carrying a database error code is scrubbed before it reaches
    the client (`error.rs`); any other internal error's message — an object
    store or MAGPIE subprocess failure — is sent as it is. None seen carries a
    secret (thirty-second audit).
- **Options considered:** a `log_format` without `$args` for those paths;
  `env_clear()` plus the few variables MAGPIE reads; `verify-full` with the RDS
  CA bundle in the image; a generic message for every `500`, the detail logged.
- **Option implemented:** None yet. *Open (thirty-first audit).*
- **Justification:** Hardening with no demonstrated failure; each is a small
  change with its own deployment risk.

**KL-64. Infrastructure hardening not done.**
- **Context:** `infra/`, `.github/workflows/`, `docker-compose.yml`.
- **Problem:**
  - No bucket's policy (the artifacts and backups buckets and their two DR
    replicas) denies requests without `aws:SecureTransport`.
  - The ALB does not set `drop_invalid_header_fields`.
  - The alerts SNS topic is unencrypted.
  - `ses:SendEmail` is allowed on `*` rather than the domain identity.
  - The pages' CSP has no `script-src` or `default-src`.
  - Third-party CI actions are pinned by tag, not commit; base images
    (`rust:1-slim-bookworm`, `debian:bookworm-slim`, `node:22-alpine`,
    `nginx:1.30-alpine`, `python:3.11-slim`, `postgres:16`, Chainguard's
    MinIO at `latest` and its client at `latest-dev`) and
    `# syntax=docker/dockerfile:1.7` by floating tag,
    not digest; the fake worker's `pip install requests` by no version; and
    `rust:1` with the stable toolchain takes new clippy lints on its own.
  - README's release build (`--pull` since pass 15) builds from the working
    tree, so nothing ties a pushed image to a commit CI passed.
  - The dev compose file publishes Postgres, MinIO and the backend on every
    interface, with committed credentials.
  - The task roles' trust policies carry no `aws:SourceAccount` or
    `aws:SourceArn` condition, which AWS recommends for Scheduler and S3; the
    derived-file builder is given `SESSION_SIGNING_KEY` only because the
    shared configuration requires it (thirty-second audit, pass 17).
  - The nightly's backup drill starts once `minio-init` is running, not
    finished; and MAGPIE's `convert_lexica.sh`, which CI caches the output of,
    has no `set -e` and exits 0 after a partial conversion (thirty-second
    audit, pass 15).
- **Options considered:** each fix as named.
- **Option implemented:** None.
- **Justification:** No failure shown. Worth a pass of its own before release.

**KL-65. Some failures raise no alarm.**
- **Context:** The backup and drill failure rules match on a task's exit code.
- **Problem:** A task that never starts its container (an image pull or secret
  failure) raises nothing: a backup's is caught only by the 36-hour staleness
  alarm, and a drill's not at all. The derived builder has no failure alarm.
  And a drill leaves no record anyone reads: its `DrillSuccess` metric has no
  alarm and no page, and it writes no row, so when it last passed is visible
  only in its logs (thirty-second audit; PLAN said it wrote a result row).
  S3 replication to the DR region has no metrics, replication-time control or
  failure event, so a replication that stops raises nothing, and the RPO
  region loss depends on goes with it (pass 17).
- **Options considered:** match `stopCode`/`stoppedReason` too; an alarm on the
  builder's failures; an alarm on `DrillSuccess` missing for some 32 days;
  replication metrics with an alarm on failed operations.
- **Option implemented:** None.
- **Justification:** Staleness catches the first within a day and a half, and
  a failed build shows on `/admin/derived-data`.

**KL-66. Most multi-step procedures are still pasted Markdown.**
- **Context:** The audit's rule: a procedure longer than a few commands, or
  with conditions, guards or loops, belongs in a tested script.
- **Problem:** Untested and still pasted:
  - RUNBOOK §1's ceiling arithmetic, poll loop, swap, `state rm` and `import`;
  - §2.0's deletes, §2.1's fetch and scratch restore, §2.3 and §2.3b;
  - §5's guards and `tfvars` generation, §6's staging and teardown;
  - the deploy rollback's wait, the password rotation's URL rewrite;
  - README "Deploying"'s SSM setup and alert-path checks.

  Earlier audits found defects in these very blocks, pass after pass.
- **Options considered:** move each into `scripts/`, with `aws` and `terraform`
  stubbed in CI.
- **Option implemented:** §2.2 moved (`scripts/restore-job.sh`, thirty-first
  audit). The rest: none yet. *Open (thirty-first audit).*
- **Justification:** Each move is a rewrite with tests of its own. §2.2 went
  first because it was the one found to fail. Take §2.3/§2.3b and §1 next.

**KL-67. RUNBOOK §4's first check compares nothing.**
- **Context:** §4, "Verifying a restore".
- **Problem:** Check 1 prints the manifest's row counts for the operator to
  compare by eye; nothing compares them with the restored database.
- **Options considered:** reuse `restore-drill.sh`'s comparison before the
  service starts.
- **Option implemented:** None.
- **Justification:** The monthly drill does compare them. Fold it in with KL-66.

### The MAGPIE side

**KL-48. MAGPIE's decline body and shutdown printing are traced by hand.**
- **Context:** The claim, the decline and the four shutdowns are checked on
  birdtest's side (`routes::worker::contract_fixtures`). On MAGPIE's side,
  `test/contribute_test.c` now pins most of them against the same fixtures:
  - `claim_task_over_http` writes `magpie_version`, `board_dim`, `rack_size`
    and `unsupported_jobs` (`contribute_claim_body`, which
    `test_the_claim_body_matches_the_claim_fixture` checks against the fixture);
  - each `shutdown-*.json` reason is waited out or obeyed as intended
    (`test_only_a_data_shutdown_is_waited_out_for_a_set_aside_job`);
  - every assignment key the executors read, and every result key the
    serializers produce (see [The client stops being birdtest's
    code](#the-client-stops-being-birdtests-code)).

  Two pieces are still traced by hand at every audit:
  - `decline_over_http` writes `claim_token`, `reason` and `missing` with
    `role`/`name`/`expected`/`actual`;
  - `print_shutdown` reads `reason` (for `unsupported_build`'s rebuild advice),
    `message`, `required_magpie_version`, `download_url` and
    `required_tarball_dates`.
- **Problem:** A change on MAGPIE's side to the decline body, or to the
  shutdown fields it prints, is caught only by hand. (Narrowed by the
  thirty-third audit's pass 2: the title used to say only the assignment shapes
  were pinned, which stopped being true when the claim-body and shutdown tests
  landed.)
- **Options considered:** expose the message builders in `contribute.c`, so they
  can be tested.
- **Option implemented:** Hand traces.
- **Justification:** Nothing is wrong today.

**KL-49. `birdtest-contribute`'s include graph differs from upstream's.**
- **Context:** The branch has no include cycles (`find_circ_deps.py` on a clean
  archive; a working tree's untracked `cppcheck_dir/` shows some that are not
  the branch's). But `compat/ctime.h` no longer includes `io_util.h` here, while
  upstream main still does.
- **Problem:** An upstream file that gets `io_util`'s declarations through
  `ctime.h` fails to compile after a merge.
- **Options considered:** None recorded.
- **Option implemented:** None.
- **Justification:** MAGPIE's CI would report the failure.

**KL-50. The backup, ops and drill tasks pull `backup_image` from public ECR.**
- **Context:** `public.ecr.aws/…/postgres:16`. Public ECR is anchored in
  us-east-1.
- **Problem:** Whether it still serves pulls in another region while us-east-1
  is down is unverified. For a stack in us-east-1, that is exactly RUNBOOK §5's
  case.
- **Options considered:**
  - mirror it with the three app images, which is one more image to keep
    current;
  - accept the risk.
- **Option implemented:** None yet. *Open (twenty-first audit).*
- **Justification:** A trade between upkeep and an unverified risk, left for a
  decision.

**KL-51. A full disk while writing a wordmap or rack info table ends the worker.**
- **Context:** The writers are MAGPIE's CLI writers (`fwrite_or_die`), which
  exit.
- **Problem:** On the contribute path, the claim then waits out the heartbeat
  timeout, and the partial temporary is removed by the next write, an hour on.
- **Options considered:** decline the task.
- **Option implemented:** None.
- **Justification:** Declining means error returns through writers the CLI
  shares.

**KL-52. Local write failures are counted, not remembered.**
- **Context:** A worker that cannot write a table or a fetched KLV, for example
  because its data directory is read-only, counts a failure.
- **Problem:** A success on another job resets the count, so the worker keeps
  claiming the job it cannot run, between the ones it can.
- **Options considered:** set the job aside, as for other failures.
- **Option implemented:** Counted.
- **Justification:** Noted rather than fixed. The record gives no further reason.

**KL-53. A wordmap or table this build cannot reproduce ends in a `data_out_of_date` shutdown.**
- **Context:** When the mismatched job is the worker's last active job, it is
  remembered as unsupported, and the server's shutdown message advises
  downloading data.
- **Problem:** The cause is the build, not the data. The worker's own summary
  prints the "built as" lines, which say otherwise.
- **Options considered:** a shutdown reason for builder mismatches, on the
  server.
- **Option implemented:** None.
- **Justification:** The worker's summary already shows the real cause.

**KL-68. Small things on MAGPIE's contribute path.**
- **Context:** `birdtest-contribute`'s `contribute.c`, `client_state.c`.
- **Problem:**
  - The raw autoplay summary line is printed on every games and leave task.
  - A heartbeat's single attempt can block for about 120 seconds against an
    unreachable server, and the submission waits for it.
  - An `expected_data` whose `files` is not an array, or lists an entry
    without a role, name or digest, or with a role MAGPIE does not know, fails
    the claim (`contribute_check_expected_data`) and ends the run, without a
    decline, so the claim lapses on the heartbeat timeout. A new role must
    therefore ship with a `min_magpie_version` raise, which the server filters
    on before dispatch; otherwise every older worker's run ends on that job's
    first claim, rather than the job being declined and set aside the way an
    unknown `job_type` is. Until the thirty-third audit's pass 3 this bullet
    said an unknown role was skipped silently; MAGPIE refuses it since its
    pass 2.
  - `contribute_find_derived` takes the first matching entry; the server now
    refuses a job that would pin two files under one name, so only a server bug
    reaches it.
  - `MAGPIE_VERSION` went to 0.1.1 in the first of `birdtest-contribute`'s
    commits since 0.1.0, and later ones changed results without a bump (a
    simulating opening-rack player ranks every play, short opening racks,
    racks with designated letters refused); the version's comment lists less
    than 0.1.1 holds (thirty-second audit). 0.1.1 and every commit up to
    dbf8df3b are now on the remote, so a build of any of them reports 0.1.1,
    dbf8df3b's included, which still captures a forced turn as the previous
    simulation's analysis (B2-3, fixed in the unpushed 1a0ae932). None can
    claim from this server: their claim states no `board_dim` or `rack_size`
    (added in c3c6a875, also unpushed), which `ClaimBody` requires, so the
    floor still has nothing to exclude. Nothing is deployed, so no bump; the
    first result-changing commit after a deployment bumps the version and the
    floor together (thirty-third audit, pass 4).
  - `magpie contribute` installs no SIGINT or SIGTERM handler, so Ctrl-C or
    `docker stop` leaves the open claim to lapse on the heartbeat timeout,
    though the decline path exists and the REPL's `stop` uses it.
  - Replacing a stale wordmap, rack info table or fetched KLV renames over the
    old file, which `rename()` does not do on Windows; the Windows build is
    written but has never been compiled or run.
  - A redirect is followed to any host or port with the request's body and
    `X-Worker-UUID` (KL-84 has the scheme half).
  - A leave task with no `lexicon` names a file `(null)_birdtest_….klv2`
    before failing to load it (a server bug is needed to send one).
  - `json_get_int_or` casts a double to `int` unchecked, which is undefined
    past `int`'s range (used for `num_plays` and display caps).
  - A shutdown's message, `expected_data` paths and error bodies are printed
    raw, so a server can send terminal escapes.
  - `heartbeat_start` initialises its mutex on every task and never destroys
    it.
  - `magpie contribute` exits 0 after every error (five failures, a `401`, no
    server), so a supervisor restarting on failure never restarts it.
  - A captive portal answering a result with a `200` that is not JSON is
    counted as an accepted task (reasoned, not run).
  - A decline's answer is not read: a `401` passes silently and the run claims
    again at once.
  - Everything after a NUL byte in `contribute.txt` is ignored, so a crash's
    zero-filled tail hides the lines after it (and a `uuid` appended there).
  - `server` is not held to a URL's shape: a bare key appended to it with no
    newline and no space is printed in the status line and in request errors
    (one with a space is refused, pass 16).
  - A UUID save cut short by a full disk leaves the part it wrote; the next
    run refuses it naming the line (pass 15), where truncating back to the
    file's old length on a failed write would leave nothing to fix.
- **Options considered:** each fix as named, on `birdtest-contribute`; bump
  to 0.1.2 before the branch is first pushed if anything between is ever built.
- **Option implemented:** Pass 14 fixed a negative `maxtasks` (now refused),
  the `bonus_square_from_char` sign (a byte above 0x7f was a negative index),
  and — with reasons of their own — the server-assigned UUID's form and a
  settings file that cannot be written. The rest: none.
- **Justification:** None changes a result. They go with the next MAGPIE change
  that has a reason of its own.

**KL-69. What `scripts/restore-job.sh` does not handle.**
- **Context:** RUNBOOK §2.2's copy-back, a script since the thirty-first audit.
- **Problem:**
  - It copies by column position (`SELECT *`, `LIKE`, `INSERT … SELECT *`): a
    column added in production after the snapshot fails every table, with no
    documented way out.
  - It holds the job's merge lock only while it runs. A leave job stopped
    part-way whose staged rows a merge then folds in reads, on the next run, as
    rows with other contents: it needs §2.0 and a run from the start (the
    script's hint says so), not a resume.
  - `scripts/scrub.sql` replaces anonymous UUIDs where they are keys, not
    where someone typed one into a ban's or an audit row's free-text reason.
  - Restoring a deleted job whose `input_data` row was deleted and then
    imported again stops on it: the new row has a new id, and the job's config
    names the old one. The job has to be recreated on the new row.
  - A run refused because the scratch copy holds no row of the job leaves its
    dumped files in the work directory until the next run clears them.
  - A deleted player config whose name was taken since stops the run with the
    generic hint (the name is unique); a restored config whose clone parent is
    gone loses its lineage rather than getting the parent back.
  - The ops shell runs the script as of the last `terraform apply`: one from
    before this audit's second pass cannot restore a deleted job, and the
    RUNBOOK no longer says how to by hand. Apply first.
- **Options considered:** copy the column list the two schemas share; keep the
  merge lock across runs (a marker the sweep reads); rewrite UUID-shaped text.
- **Option implemented:** None.
- **Justification:** A schema change between a snapshot and its restore, a
  stop and a merge between two runs, and a UUID typed into a reason are each
  rare, and each fails loudly or harmlessly.

---

## Possible Future Improvements (from fishtest)

### Worker Client Features

- **Fleet mode** — a flag that makes the worker exit cleanly on error or empty queue, enabling orchestrators (systemd, Docker, CI) to manage its lifecycle.
- **Global artifact cache** — multiple workers on the same machine or network share downloaded dictionaries and bot binaries rather than each fetching independently.
- **Hardware-aware binary selection** — workers report CPU capabilities and download or compile the appropriate binary variant for their architecture.

### Configuration

- **Per-job-type (or per-job) heartbeat timeout** — the heartbeat timeout window is a single global constant for v1. A future improvement could make it configurable per job type or per individual job.
- **Automatic database password rotation** — the master password is set by hand and rotated by runbook, because every other secret is handled that way and the tasks read a fixed `DATABASE_URL` from SSM. Turning RDS-managed rotation back on means injecting `DB_PASSWORD` from the managed secret (the backend can already assemble its URL from `DB_*` parts), teaching `backup.sh` and `restore-drill.sh` the same parts, and an EventBridge rule that forces a new ECS deployment on each rotation — with a short window after each one where new connections fail until tasks restart. Having the process re-read the secret itself would close that window but adds a Secrets Manager code path the design deliberately avoids.

### Scaling

- **Primary/secondary server split** — one instance owns task scheduling and mutations; read-only instances serve the dashboard. Eliminates concurrent scheduling conflicts under high worker load. Note that several things assume a single instance today and would have to move first: the dispatch holds (`jobs::DispatchHolds`) that keep claims off a job being purged, deleted, seeded or having its consensus edited, and the purge count the finish check compares; staged imports, exports and leave-generation transitions and the startup reapers that fail or release them; the in-process rate limiters; and the per-process SSE broadcaster (`desired_count` is validated to at most 1 for that reason; KL-82).
- **Deleting an unused player config scans the per-task request tables.** The
  foreign keys from `game_requests` and `opening_rack_requests` have no index
  (one would be a write per task issued, for a rare admin action), so the
  delete's checks read every request row: tens of milliseconds at half a
  million, seconds on a mature database. Accepted.
- **`leave_rack_progress` is vacuumed at the default scale factor.** The table
  keeps every generation's rows (3.2 million per English generation), so 20%
  dead tuples is several merges' worth before autovacuum starts. A per-table
  `autovacuum_vacuum_scale_factor` near 0.02 is a tuning choice left for when
  a production table's bloat is measured. *(Fifteenth audit.)*
- **`users` carries two tasks-completed indexes** (`users_contribution_idx` for
  `/api/users`, tie-broken by `created_at`, and `users_worker_rank_idx` for
  `/api/workers`, by `id`), each written on every registered submission. One
  would do if `/api/users` broke ties by id — a visible ordering change, left
  for a decision. *(Fifteenth audit.)* (The worker list's other two orders,
  movegens and compute, have an index each too, on both kinds of identity:
  every submission writes them.)
- **The job list's `stalled` flag reads `jobs.last_completed_at`** — done
  (nineteenth audit). Answered from the claims, "no result in a day" joined
  every task of the job to the day's completions (hundreds of milliseconds at a
  million tasks, growing with history, on every list view); the submission that
  stores a result now keeps the time, at most once a minute.
- **Debounced live stats** — done: the finish condition is checked on every eighth submission (plus whenever nothing is left in flight), and the SSE push is coalesced per job and spaced at least `JOB_STATS_CACHE_SECONDS` (10) apart — by a cool-down after every build since the thirty-second audit, before which it held only under steady load — the page and new subscribers reading the last push's payload. What remains is the payload's cost itself: its contributor list groups every completed claim of the job (joined through `tasks` by `tasks.job_id`; no index on `task_claims` leads with the job — KL-58), and its game statistics read every result — hundreds of milliseconds at a few hundred thousand tasks, growing with history. *Open (fifteenth audit):* a per-job contributor running total (`job_contributors`, upserted in the submit transaction like `users.tasks_completed`, given back by purge and delete, recounted by RUNBOOK §2.3b) would make the list an index read; it is a schema change and one more write per submission, left for a decision.

---

## Backups and Restore

How birdtest's state is preserved and how it is put back. The rest of this
document describes what the system does; this section describes what happens when
the disk, the region, or an admin's finger goes wrong. [RUNBOOK.md](RUNBOOK.md) is
the operational half: this is why, that one is what to type.

Phases 1–4 below are implemented; Phase 5 is not. The **Option** tables are what
was built, so they read as rationale rather than as proposals — they record why
each fork went the way it did, which is the part that is expensive to reconstruct
later.

| | Where |
|---|---|
| 30-day PITR, `copy_tags_to_snapshot`, optional Multi-AZ | [infra/rds.tf](infra/rds.tf) |
| Artifact bucket lifecycle and cross-region replication | [infra/s3.tf](infra/s3.tf) |
| Backup bucket (KMS, Object Lock, lifecycle, CRR), the nightly task, its schedule, the alarms, and the monthly restore drill | [infra/backup.tf](infra/backup.tf) |
| The dump itself | [scripts/backup.sh](scripts/backup.sh) |
| The automated restore drill | [scripts/restore-drill.sh](scripts/restore-drill.sh) |
| `backups` table, artifact checksums, destruction censuses | [backend/migrations/0001_initial.sql](backend/migrations/0001_initial.sql) |
| `GET /api/admin/backups`, `POST /api/admin/jobs/:id/rebuild-artifacts` | [backend/src/routes/admin.rs](backend/src/routes/admin.rs), [backend/src/backups.rs](backend/src/backups.rs) |
| Admin surfaces | [frontend/src/routes/admin/backups/](frontend/src/routes/admin/backups/), the job page's **Check artifacts** |
| Local snapshot, restore, scrub, and a dump/restore round trip | [scripts/](scripts/) |

### What has to survive

State lives in four places, and they are not equally precious.

| Store | Contents | Class |
|---|---|---|
| RDS Postgres | Everything in the [Schema](#schema-1): users, API key hashes, jobs, player configs, tasks, claims, results, `input_data.content`, audit log | **System of record** |
| S3 artifact bucket | Per-generation KLVs under `leaves/{job_id}/generation-{n}.klv2` | **Derivable** |
| SSM Parameter Store | `/birdtest/DATABASE_URL` (which carries the hand-set RDS master password), `/birdtest/SESSION_SIGNING_KEY` | **Secret, unmanaged** |

Within Postgres the rows differ enormously in how replaceable they are, which is
what makes selective restore worth building rather than only whole-database
rollback:

- **Irreplaceable.** `game_results`, `position_analysis_records` / `_moves` /
  `_plies` / `_inference`, `leave_rack_progress` together with `leave_rack_staging` (the
  accepted leave results not yet merged into it), `leave_records`, `rating_pools`,
  `rating_pool_members`, `task_claims`, `audit_log`. This is donated compute. A
  contributor is not going to run the same 40,000 game pairs again because we
  lost them, and the match test's verdict derived from them cannot be recomputed from
  anything else. Rating *runs* are the exception in the other direction: they are
  a pure function of pool membership and `game_results`, so a lost snapshot is
  one recompute away — which is exactly the property batch fitting buys.
- **Irreplaceable and sensitive.** `users` (email, argon2 hash), `api_keys` (key
  hashes), `worker_bans`, `anonymous_workers`. Losing these logs the whole fleet
  out; leaking them is a disclosure incident. This is what makes backup encryption
  a requirement rather than a nicety.
- **Reconstructible with effort.** `input_data` rows are re-importable from the
  MAGPIE-DATA tarballs — but only while upstream still publishes the exact bytes
  each row's `sha256` pins. Because jobs pin `input_data.id`, an upstream retag
  that changes a file's bytes makes a restored job unrunnable. Treat `input_data`,
  `content` BYTEA included, as if it were irreplaceable.
- **Regenerable.** `tasks` and the per-type request rows. Every job type generates
  its tasks on demand from a deterministic space, and `purge_job` already deletes
  them and lets them regenerate. They are in the dump because excluding them is
  more work than including them, not because they are needed.

**Size decides the mechanism.** `position_analysis_moves` sets the budget: a full
English 7-tile opening-rack job is ~3.2M racks, and with `num_plays_recorded` at 10
that is 32M move rows per job, plus plies for simming configs. A handful of such
jobs puts the database in the tens of gigabytes with the results tables holding
better than 95% of it. Three consequences run through the rest of this section: a
logical dump must be parallel and compressed; "restore the whole database to fix
one job" is unacceptably slow, so selective restore is a first-class path; and a
naive nightly full dump of an ever-growing append-only corpus is mostly
re-uploading data that has not changed, which is the argument for keeping physical
PITR as the primary mechanism and logical dumps as the portable secondary.

### Objectives and failure scenarios

Recovery targets, chosen to be honest about what a single-instance hobby-scale
deployment can actually promise:

| | Target | Bounded by |
|---|---|---|
| RPO, infrastructure failure | 5 minutes | RDS PITR replay granularity |
| RPO, region loss | 24 hours | Nightly cross-region copy |
| RTO, single-AZ / instance failure | < 1 hour | RDS restore-to-new-instance + ECS pointing at it |
| RTO, logical error (bad purge, bad delete) | < 4 hours | Scratch restore + selective repair |
| RTO, region loss | < 24 hours | Terraform apply in the new region, restore, DNS |

The scenarios worth designing against, in descending order of likelihood:

1. **An admin destroys data through the API.** `purge_job` deletes every claim,
   result, rating and progress row for a job in one transaction, and `delete_job`
   is similarly total. (`delete_user` is not: it anonymizes the account and keeps
   its claims and results, destroying only its keys, outstanding codes, name,
   address, password, admin flag and sessions.) There is no confirmation dialogue in the API
   layer and no undo. This is the most likely way birdtest loses contributor work,
   and it is the scenario that most demands *selective* restore: the rest of the
   database has moved on and must not be rolled back.
2. **A bad migration.** Until release the schema is a single file edited in place,
   and the documented reset is `DROP SCHEMA public CASCADE`. That command against
   the wrong `DATABASE_URL` is a total loss with no application-level trace.
3. **Instance or AZ failure.** RDS is single-AZ by default; an instance failure is
   an outage and a restore.
4. **Region loss.** Unlikely, survivable only if backups already left the region.
   The rebuild (RUNBOOK §5) is a second copy of the stack under `name_suffix`,
   applied with its scheduled tasks off (`scheduled_tasks_enabled = false`)
   until its database is restored and its artifacts synced, its storage sized
   from the replicated manifest, and its zones taken from the region (`azs`
   unset, then pinned to the `azs` output for later applies).
5. **Credential compromise.** An attacker with the task role can `PutObject` over
   any artifact key; an attacker with broader AWS access can delete backups. This
   is what object versioning and Object Lock are for.
6. **Artifact corruption or accidental overwrite.** A KLV rebuilt with a changed
   the KLV builder and written to an existing key silently changes what workers fetch for
   that generation.

### Backing up Postgres

| Option | For | Against |
|---|---|---|
| **A — RDS-native only** (snapshots + PITR) | Zero code, zero operational surface; block-level and fast; ~5-minute PITR granularity | Snapshots are opaque and bound to RDS — they restore only as a whole new instance, never as a table or a row; unreadable on a laptop; a region-wide failure takes them unless copied; the fastest path to "undo one job purge" is a full instance restore |
| **B — Logical dumps to S3** (`pg_dump -Fd -j4 -Z6`) | Portable — restorable into any Postgres 16, including a laptop and a `docker compose` stack; supports `pg_restore -t`, `-n`, `--data-only`; readable and greppable; survives the account if copied out | Code and a schedule to own; a consistent dump of a live database holds a long transaction; dump and restore time grows with the corpus; the dump embeds the schema, which matters under the in-place migration policy |
| C — AWS Backup | One place for policy, retention and cross-region copy; Vault Lock gives genuine ransomware/insider resistance | Another service and IAM surface; still snapshot-shaped, so it does nothing for selective restore; Vault Lock in compliance mode is irreversible |
| D — Streaming replica / logical replication | Near-zero RPO and RTO | Doubles the database cost at `db.t4g.micro` scale, and replicates logical errors instantly — a purge is replicated in milliseconds. It is availability, not backup |

**Built: A + B, with C as a later hardening step.** RDS automated backups at 30-day
retention are the primary mechanism and the fast path for infrastructure failure.
Nightly `pg_dump` to a separate, versioned, cross-region-replicated backup bucket is
the secondary: it is what makes selective restore, local reproduction, and
out-of-AWS survival possible. AWS Backup with a locked vault is worth adding once
there is contributor data worth an insider-threat model.

Directory format beats custom format for the dump: `-j` parallelism is what makes a
tens-of-gigabytes dump finish, and per-table files make it possible to pull exactly
one table out of a backup without streaming the whole archive.

Take the nightly dump **from a snapshot-restored instance rather than from
production** if and when the dump starts to take long enough to matter: restore the
most recent automated snapshot to a temporary `db.t4g.medium`, dump from it, delete
it. That removes all load and all long-transaction concerns from the live instance
at the cost of a slower job and a few cents. It starts by dumping directly from
production — at current scale that is minutes — with the snapshot path to be adopted
when a dump exceeds ~30 minutes.

### Where the dump runs, and where it lands

| Option | For | Against |
|---|---|---|
| **EventBridge Scheduler → ECS RunTask** | Same VPC and security group as the service, so it reaches RDS with no new network path; no time limit; same logging as everything else; triggerable ad hoc with one CLI call | A task definition and a schedule to maintain |
| Lambda | Cheapest, simplest to schedule | No `pg_dump` in the runtime — needs a layer or container image pinned to the server's major version; a 15-minute ceiling a growing corpus will eventually hit; needs VPC attachment anyway |
| In-process, from the backend | No new infrastructure; naturally exposed in the admin UI; already has `DATABASE_URL` and an S3 client | Puts a heavyweight, long-running, memory-hungry job inside the request-serving process, and couples backup liveness to application liveness — the deployment most in need of a backup is the one whose backend is crashlooping |
| GitHub Actions on a cron | Free, visible, no AWS scheduling | RDS is not publicly accessible; would require exposing it or a bastion, a worse trade than any backup is worth |

**Built:** EventBridge Scheduler invoking `ecs:RunTask` against a dedicated
`birdtest-backup` task definition, using the `postgres:16` official image with an
inline command, so the dump tool's version tracks the server version by
construction and no new image needs building. The backend keeps a *read-only*
relationship to backups: it lists them and reports staleness, but it never performs
one. That keeps the crashlooping case safe.

| Where backups live | Notes |
|---|---|
| A prefix in the existing artifacts bucket | Fewest resources; but the task role already has `PutObject` there, so a compromised backend could overwrite backups. **Rejected.** |
| **A separate `birdtest-backups-<account>` bucket** | Distinct IAM: the backup task writes, the backend has no access at all, restore uses a human's credentials. Versioning on, public access blocked, SSE-KMS with a dedicated key. **Built.** |
| Separate bucket in a second region, written directly | Cross-region PUT costs and latency on every dump; replication is the better shape |
| **Separate bucket + Cross-Region Replication** | Nightly dump writes locally; CRR copies to `birdtest-backups-dr-<region>`. **Built.** |
| Separate AWS account | The only configuration that survives a full account compromise. Overkill for now; the endpoint of this progression |

Lifecycle on the backup bucket: keep 30 daily, transition to Glacier Instant
Retrieval at 30 days, expire at 365; expire noncurrent versions at 90 days; abort
incomplete multipart uploads at 7 days.

#### Layout and manifest

```
s3://birdtest-backups-<account>/
  pg/2026-09-07T03-00-00Z/
    manifest.json
    dump/                     # pg_dump -Fd output, one file per table
  pg/2026-09-07T03-00-00Z.manifest.json   # duplicated at top level for cheap listing
```

`manifest.json` is what makes a backup self-describing, and the in-place-migration
policy is what makes it non-optional. It carries `started_at` / `finished_at`,
`postgres_version`, `pg_dump_version`, `backend_image`, `migration_checksums`,
`database_bytes`, `dump_bytes`, `table_row_counts`, `artifact_keys_referenced`, and
a `sha256`.

The `sha256` is a digest of the dump's *contents* — every file hashed under its
relative path, then a hash of that listing — and deliberately **not** a hash of a
tar of the directory. A tar carries mtimes and ownership, which S3 does not
preserve, so a tar digest would report a mismatch for every dump that had merely
made the round trip through the bucket. The restore drill found this the first time
it ran, which is the argument for the drill in miniature.

The drill of a bucket with nothing in it passes, saying so: a new stack (or
RUNBOOK §5's copy) turns its schedules on before its first backup, and the drill
runs on the 1st. A bucket with objects but no manifest under the prefix fails,
since passing it would pass every drill of a misplaced prefix; a stack that
should have backups and has none is the backup-stale alarm's to report.

`migration_checksums` and `backend_image` together answer "what code can read this
dump", which under a single mutable `0001_initial.sql` is otherwise unanswerable.
`table_row_counts` is what a restore is verified against, and what makes a
silently-truncated dump detectable without restoring it.

#### Encryption and secrets

The dump contains argon2 password hashes, API key hashes, email addresses and
unexpired reset-token hashes. It is encrypted with SSE-KMS using a customer-managed
key whose policy grants `Decrypt` to the restore role and, to the backup task,
only through S3 and only for the backups bucket — a multipart upload under SSE-KMS
needs it (S3 decrypts the data key to complete the upload, and `aws s3 cp` goes
multipart past 8 MB; granting only `GenerateDataKey` and `Encrypt`, as first
written, refused every upload of a real database's larger tables). The task holds
no `s3:GetObject`, so a compromised backup task can create backups but not read
old ones.

The two SSM parameters are not covered by any of the above and are the thing most
likely to be forgotten in a region-loss drill, because they are deliberately not
managed by Terraform:

- `SESSION_SIGNING_KEY` — losing it invalidates every session cookie (users log in
  again; recoverable, annoying). Restoring a database *with* a rotated key has the
  same effect.
- `DATABASE_URL` — carries the RDS master password, which is set by hand rather
  than managed by RDS (managed passwords rotate every 7 days, which would break
  a fixed URL). A restore keeps the password, so the URL is regenerated from the
  new endpoint, and in fact **must** be after any restore-to-new-instance, since
  the endpoint changes. Rotation is a runbook step.

They are documented in the runbook as manual steps rather than copied into a
KMS-encrypted `secrets.json` beside the dump: copying long-lived secrets into a
second store to guard against a scenario that ends with "generate a new 32-byte key"
is a poor trade, and `DATABASE_URL` is derived during restore anyway.

### Artifacts: back up, or rebuild?

The KLVs in S3 are the only application data outside Postgres, and they have an
unusual property: **they are pure functions of data that is already in the
database.** `run_transition` streams `leave_rack_progress` into a
`rack,count,equity_sum` CSV and runs `magpie convert rackequity2klv` against the
pinned letter distribution — whose bytes are themselves in
`input_data.content`. `leave_rack_progress` rows are never deleted per
generation, so every generation's inputs remain present for the life of the job.

One thing changed when MAGPIE took this over: **a rebuild that produces
different bytes is no longer on its own evidence of corruption.** With a single
Rust implementation it was; with MAGPIE building them, an upgrade can
legitimately change the bytes for the same leave values. So
`leave_generation_artifacts.builder` records which builder wrote each artifact,
and a rebuild under a different one reports that rather than "differs" — without
it, the first MAGPIE upgrade after a restore drill reads as data loss.

| Option | For | Against |
|---|---|---|
| **Rely on versioning + rebuild** | No extra copies of multi-megabyte binaries; the DB stays the single system of record | Rebuild must be byte-reproducible; a future MAGPIE KLV builder produces different bytes for an old generation (which is why the builder is recorded per artifact) |
| **Replicate the artifacts bucket cross-region** | Trivial (CRR); covers "S3 object gone" without any rebuild logic | Pays storage for derivable data |
| Include artifacts in the nightly bundle | One restore unit; fully self-contained | Largest and most redundant; re-uploads unchanged binaries nightly unless made incremental |
| **Store the artifact's sha256 in the DB** | Makes corruption and drift *detectable*, and makes a rebuild verifiable | A schema change and a small code change |

**Built: versioning + CRR on the artifacts bucket, plus
`leave_generation_artifacts.sha256` and an admin-triggered rebuild path.** The
replication covers the KLVs (`leaves/`) and, since the twelfth audit, the imported
input data (`inputs/`): a database restored in another region names both, and
without the input objects no derived file builds and no job needing one
dispatches — re-importing would bring them back only for as long as GitHub still
serves the tarballs. Exports are not replicated. The three
work together: replication handles object loss, the checksum turns "was this
artifact corrupted or overwritten" into a query, and the rebuild path is what a
restore uses when a DB restored to time *T* references keys that no longer exist.

Two details a rebuild has to respect:

- **A rebuild records what the object holds**: `served_sha256`, which workers
  are sent and check the object against, while `sha256` keeps the hash first
  written as the evidence it is. Left on the first hash, a rebuild under a
  changed builder (or `force`) had every task of the next generation refused
  by every worker. It is set on *every* rebuild, not only one that rewrites:
  from the bytes written, or the bytes read back when the object is left
  alone — so a rebuild whose write landed but whose update did not, or an
  object version an operator copied back (RUNBOOK §3), are put right by
  running **Check artifacts** again. But only bytes the check can account
  for — the first build, the rebuild, or what was already served: taking any
  object's hash approved, after a mistaken purge, a re-run and a copy-back of
  the old rows, the re-run's KLV as the old run's leaves. An object nothing
  accounts for is replaced with the first build when the rows reproduce it
  exactly, and otherwise left, reported (`object_accounted_for`, shown on the
  admin page) and refused by workers until an admin restores the right
  version or forces a rebuild. A worker declines a KLV that fails the check
  and waits, leaving the job claimable, since this is the server's to fix.
- `seed_zero_generation` writes generation 0 as a zeroed KLV; a rebuild must
  reproduce generation 0 the same way rather than from `leave_rack_progress`, which
  for generation 0 does not exist.
- `purge_job` deletes `leave_generation_artifacts` rows without deleting the S3
  objects, so orphaned objects accumulate. That is benign for correctness — the
  worker artifact endpoint gates on the DB row existing, so an orphan is unreachable
  — but it means "the object exists" is never sufficient evidence and the DB is
  always the authority on what a valid key is. It also means a restored DB may
  reference keys whose objects were never deleted, which is the *lucky* direction.

**The ordering rule between the two stores.** Because objects are only ever added
and never deleted, **the artifact store's state must be at least as new as the
database's.** Restoring the database to time *T* is safe against an artifact bucket
at any time ≥ *T*: every key the restored DB knows about was written before *T* and
still exists. The reverse — rolling the bucket back while the DB stays current —
breaks the worker artifact endpoint for any generation completed in between. So:
never restore the artifacts bucket to an older version wholesale; restore individual
object versions only for the specific corrupted key; and when both must be restored,
restore the bucket first (or not at all) and the database second.

### Making backups visible

A backup that fails silently is not a backup. Three layers, cheapest first:

1. **Failure alarm.** An EventBridge rule on ECS Task State Change matching a
   non-zero exit of the backup task family → SNS → the admin's email. Catches
   crashes but not the schedule never firing.
2. **Staleness alarm.** A CloudWatch alarm on `AWS/S3` `NumberOfObjects` is too
   coarse; instead the backup task emits a `birdtest/backup Success`
   custom metric and the alarm fires on `missing data` for > 36 hours. Catches both
   crashes and a schedule that silently stopped.
3. **In-app surface.** An admin page listing recent backups.

The same topic carries the database's own warnings (`infra/rds.tf`): RDS's
`low storage`, `failure` and `notification` events, through an event
subscription — autoscaled storage stops at five times its first allocation, and
CloudWatch has no metric for how near the ceiling is (a `FreeStorageSpace`
threshold fired from the first apply and never cleared) — and CPU over 80% for
fifteen minutes. The `low storage` mail before each autoscaling step
(RDS-EVENT-0089, over 90% of the current allocation) is routine; the ones that
matter are allocation past 80% of the ceiling (RDS-EVENT-0225, `notification`)
and the ceiling reached (RDS-EVENT-0224, `failure`).

It also carries the site being down (`infra/ecs.tf`): no healthy backend or
frontend target behind the load balancer for ten minutes. Nothing else would
report a crash-looping task, a health check that never passes, or a rollback
onto a schema its image refuses. Ten minutes, not one, because the backend's
service keeps no healthy task through a deploy, and migrations run inside the health
check's grace. The alarms exist only while `desired_count` is above 0, so a
first apply or RUNBOOK §5's first step does not page. The apply that raises
it does, once: it creates the alarms before the task is healthy, and with no
data yet they start in ALARM, then clear (README's first deploy says to expect
it).

It carries a deploy the circuit breaker rolled back, too (`infra/ecs.tf`,
`-deploy-failed`: ECS's `SERVICE_DEPLOYMENT_FAILED` for either service). The
`-down` alarms rarely see one, since three failed launches usually end inside
their ten minutes with the old revision healthy again, and `apply` does not
wait. Yet a rollback is not over when the site is back: Terraform's state still
names the abandoned revision, so the next apply of any kind redeploys it; the
derived-data builder's schedule runs the family's latest revision, the
abandoned image, which with a moved MAGPIE pin keys nothing the web task needs
(KL-62's symptom, reached with no operator mistake); and every dump's manifest
names its `backend_image`. The thirty-third audit's pass 2 found it silent;
the mail points at RUNBOOK's "Rolling back a deploy". `S-TF-3` checks the
rule's pattern at plan time; that ECS's event reaches the topic needs a real
account.

And it carries mail's (`infra/ses.tf`, the audit's pass 24). Every account mail
is sent off its request and answered the same whether it went or not, so a
failed send reached no one but the log -- as `SES send failed: service error`,
the same for a paused account, an unverified address and a missing
permission. Now the log carries SES's code and message, a metric filter on
the failure line's `alarm = "mail_failed"` field alarms on the first one (the
SDK has retried by then; a JSON field, because the frontend's access lines in
the same log group carry whatever a visitor puts in a header), and two alarms
watch SES's own bounce and complaint rates at 4% and 0.08%, below the 5% and
0.1% at which SES reviews an account (it may pause one at 10% and 0.5%).
Addresses that hard-bounced or complained are suppressed account-wide; an
address is refused at registration unless SES would parse it (a dot-atom and
host-name labels); sends are spaced to the account's sending rate
(`MAIL_MAX_PER_SECOND`), one queue drained in order by one sender, so a burst
of registrations waits its turn rather than being refused (the audit's pass
25 — first as each send waiting on the limiter by itself, which let the newest
win every turn and starved the oldest; the queue is bounded, and a mail it
refuses or a reset that waited past its link alarms); and a send SES never answers fails after 30
seconds, logged and alarmed, where it hung silently. In SES's sandbox every
send to an unverified address still fails, and alarms (README). What a visitor
can still do to the rates, and to the mail queue, is KL-91.

Layer 3 had a design choice of its own. Having the backend list the backup bucket
directly would require giving the task role `ListBucket` / `GetObject` on it,
weakening the isolation the separate bucket exists to create. Instead **the backup
task writes a `backups` row into Postgres when it finishes**: the backend reads its
own database and needs no new S3 permission, and the row carries the manifest's
figures (bytes, sha256, row counts, times). A
restored database also restores the backup history: the rows from before the restore
are the source's runs, shown as they were and not marked as restored, which is harmless
— staleness is measured from the newest successful run, and the next nightly run adds
its own row. Failed runs insert a row with `ok = false` so
the admin page shows the failure rather than a gap. The table is insert-only and
nothing in the request path reads it.

A related, cheap safety feature belongs in the same phase and is built:
`purge_job` and `delete_job` write the counts of what they are about to destroy
into `audit_log` *before* destroying it, and `delete_user` what it destroys and
what it keeps. Restoring is far easier when the
log says what was lost.

### Restore

| Scenario | Mechanism | Data loss |
|---|---|---|
| Instance/AZ failure | RDS PITR restore to new instance, repoint `DATABASE_URL` | ≤ 5 min |
| Bad migration / dropped schema | RDS PITR to just before the statement | ≤ 5 min |
| Mistaken purge/delete of one job | Restore latest dump into a **scratch** instance, extract, re-insert (RUNBOOK §2) | Whatever arrived after the last dump, for those rows only |
| Mistaken deletion of an account | None documented: its work is kept; the owner registers again and makes new keys (the old name is free once deleted) | The account's name, address, password, admin flag, sessions and keys, and its credit on the account list: the work stays under the tombstone, which `/api/users` omits (the Contributors ranking, `/api/workers`, lists the tombstone with its count), so the new account starts at zero |
| Corrupted or overwritten artifact | S3 object version restore, or rebuild from `leave_rack_progress` | None |
| Region loss | Terraform apply in DR region, restore cross-region snapshot or replicated dump | ≤ 24 h |
| Local dev database wedged | `docker compose down -v` and re-seed, or restore a scrubbed dump | N/A |

The literal commands are in [RUNBOOK.md](RUNBOOK.md). What follows is the reasoning
the commands assume.

**Full restore (PITR).** Stop writes first — `aws ecs update-service
--desired-count 0`. This matters more than it looks: leaving the service up means
workers keep submitting results into a database that is about to be replaced, and
those submissions are silently discarded. Restore to a new instance, point
`/birdtest/DATABASE_URL` at the new endpoint, scale back up (new tasks read SSM at
start, so no image rebuild is needed), verify, and only then retire the old
instance. Before scaling up, re-apply what the restore undid for security —
revocations, resets, bans, deletions and demotions since the restore point —
from the old instance's audit rows, those the restored instance lacks by id,
reviewed and with what the review leaves out enforced (RUNBOOK §1); and end
every session with a new signing key, since a session ended after the restore
point matches the restored instance again: the restore brings every
credential back as it was. Confirm `deletion_protection` and `backup_retention_period` carried over:
a restored instance does **not** inherit automated-backup settings by default, and a
restore that leaves the new instance unbacked is a trap. The alternative shape —
restore and *swap identifiers* so the endpoint is unchanged — avoids touching SSM but
requires renaming the damaged instance first and is slower under pressure; prefer
repointing SSM.

**Selective restore** is the common case, and the database must not be rolled back
because everything else has moved on. Restore into a scratch database, extract the
affected rows in dependency order, re-insert them into production in batches, each
its own transaction, with `ON CONFLICT DO NOTHING` and a check that every row is
there as dumped, so a partial run resumes and a conflicting row stops it
(`scripts/restore-job.sh`, RUNBOOK §2.2), then repair
the denormalized counters — which is the part a naive row copy gets wrong.
`tasks.accepted_count` and `active_claim_count` must be recomputed from the restored
`task_claims`, and `tasks.state` / `completed_at` recomputed from them. `purge_job` deletes tasks precisely so they regenerate cleanly; a
restore that puts claims back without their counters leaves the scheduler
dispatching work that is already done. Finally, recompute what is not a simple copy:
the job's match-test verdict, and the rating pools (a refit, from data that is already
there).

Two ways to package that: a documented runbook plus SQL snippets (no code, no
maintenance, fully general, but every use is bespoke and under time pressure), or a
`birdtest-restore` subcommand in the backend binary (correct by construction,
testable in CI, and the counter repair reuses `registry::initialize_job_artifacts` and
`ratings.rs` rather than reimplementing them in SQL). **The runbook first, the
subcommand once the runbook has been used in anger at least once.** Writing the tool
before knowing which shapes of disaster actually occur builds the wrong tool; but
the counter-repair step is subtle enough that it should end up a tested function
rather than a snippet pasted under pressure, so the subcommand is the intended
destination.

**Restoring the database past artifact writes.** A database restored to time *T*
references keys written before *T*, all of which still exist, so the usual case
needs nothing. The case that needs care is a leave-gen job whose generation
transition happened *after* *T*: the restored DB shows the generation still in
progress, workers resubmit, `run_transition` runs again, and `artifacts.put` writes
the same key with contents derived from a different (smaller) set of
`leave_rack_progress` rows. The old object is retained as a noncurrent version, so
nothing is lost, but for a period some workers may have fetched the pre-restore KLV
and others the post-restore one for the same key. With
`leave_generation_artifacts.sha256` this is detectable rather than invisible, and
the `ON CONFLICT (job_id, generation) DO NOTHING` means the row keeps the *first*
checksum — so a mismatch is a signal to investigate, not a bug to fix in a hurry.

**Restoring across a schema change.** Before release there is one migration file,
edited in place, and sqlx refuses to start against a database whose applied checksum
differs. A dump taken under an older `0001_initial.sql` therefore restores into a
database the *current* backend will not run against. In order of preference:
restore with the image the manifest names (`backend_image` exists for exactly this),
confirm, and then migrate forward deliberately; hand-write a forward-fix migration
and update `_sqlx_migrations` to the current checksum; or `pg_restore --data-only`
into a freshly migrated empty database, which works when the change is additive and
fails noisily when it is not. This is the strongest practical argument for cutting
over to numbered migrations at release: the in-place policy makes every backup older
than the last schema edit restorable only with archaeology. Until then a production
reset (`scripts/reset-prod-db.sh`, `deploy.sh --reset-db`) deletes the nightly dumps
with the database -- every version under `pg/`, in the backups bucket and its DR
replica, past the governance lock -- so no dump of an older `0001` outlives the
schema it was taken under, nor the accounts it holds. RDS's point-in-time backups
are left to expire: for `db_backup_retention_days` (30) after a reset they are the
one way back across it (RUNBOOK, "Resetting the production database").

**Verifying a restore.** Do not declare one finished on "the page loads":

- `SELECT COUNT(*)` per major table, against the manifest's `table_row_counts` (for
  a dump restore) or pre-incident dashboard figures (for PITR).
- Referential sanity: no `leave_generation_artifacts` row whose key 404s through the
  worker artifact endpoint; no job whose `letterdist_id` or `layout_id` is missing
  from `input_data`; no `input_data` row with `role IN ('letterdist','layout')` and
  `content IS NULL`.
- Counter sanity: `tasks.accepted_count` and `active_claim_count` agree with
  `task_claims`.
- Functional smoke: run one real task against the restored stack with `magpie
  contribute` (`maxtasks 1`) and see it accepted. This exercises dispatch, data
  verification, the artifact fetch and the result write. **Not
  `worker/fake_worker.py`**: it submits invented results, and the server would
  record them as real contributions to real jobs.
- Confirm the restored instance has `backup_retention_period` and
  `deletion_protection` set.

In-flight claims are self-healing and need no action: claims open at the restore
point are reclaimed by the heartbeat timeout, and workers whose submissions land
against a claim the restored database has never heard of are rejected exactly as a
stale claim is — a path `fake_worker.py --mode stale` already covers.

**Drills.** A restore procedure that has never been executed is a hypothesis.
*Automated, monthly*: a scheduled task restores the latest dump into a Postgres of
its own, runs the row-count check and the SQL referential and counter checks,
and tears it down.
Its result is its exit status, which the `restore-drill-failed` rule mails, and
a `DrillSuccess` metric nothing reads; it writes no row anywhere (it has no
connection to the production database, by design), so nothing shows when it last
passed, and a drill that stops being scheduled raises nothing (KL-65). This is
the only thing that catches a dump that has been silently producing empty output for
three weeks. *Manual, twice yearly*: a full region-loss drill — `terraform apply`
into the DR region from scratch, restore, and check off every manual step (the two
SSM parameters, SES domain verification, DNS, the artifact bucket). The point of the
manual drill is to find the steps that only exist in someone's head.

### Local development

`docker compose` state is a Postgres volume and a MinIO volume, and losing them is a
re-seed rather than a disaster. Two things are still worth having, because they serve
development rather than recovery:

- `scripts/dev-dump.sh` / `scripts/dev-restore.sh` — snapshot and restore the local
  stack, so an experiment that needs a wrecked database is cheap.
- **Restoring a production dump locally** is the most valuable debugging tool here,
  and the one with a disclosure risk: the dump carries real email addresses and
  password hashes. `scripts/scrub.sql`, applied immediately after a local restore,
  rewrites `users.email` to `user-<id>@example.invalid`, replaces every
  `password_hash` with a known throwaway argon2 hash, truncates `api_keys`,
  `email_confirmations`, `password_reset_tokens` and `backups`, and replaces every
  anonymous worker's UUID, which `X-Worker-UUID` alone authenticates, with its
  claims, ban and audit rows following it, and every open claim's token
  (thirty-first audit; until then a scrubbed dump could submit as any anonymous
  contributor), and every ban's reason, in the ban and its audit row — an
  admin's free text about a person (the audit's pass 23). Usernames stay:
  they are public on the site. It refuses to run unless asked for by name
  (`-v dev_copy=1`, which `dev-restore.sh` passes): its usage line once pointed
  it at `$DATABASE_URL`, which in the ops shell is production; its transaction
  opens before that refusal, with `ON_ERROR_ROLLBACK` off, so the script
  pasted into an interactive psql, which goes on reading past an error, runs
  nothing after it (it ran the whole scrub, pass 24, and a psqlrc's
  `ON_ERROR_ROLLBACK` let it still). A dump is
  restored, and scrubbed, into a database of the run's own that replaces the
  stack's, in one transaction, only once both have succeeded: a dump cut
  short, a signal or a failing scrub drops the copy and leaves the stack as it
  was (restored in place, one that failed part-way was left unscrubbed, and one
  stopped by a signal went on restoring inside the container; swapped by two
  renames, a failure between them, or two restores at once, lost the stack's
  database, pass 24). A signal during the swap waits for it and says which way
  it went, and a stop after it brings the backend back. A snapshot is written
  aside and moved into place only when complete, the old one moved aside
  before and removed after, and nothing is left of one that fails. The
  artifact bucket is mirrored as the host's user and its objects counted: as
  the image's own user, mc wrote nothing on Linux and said nothing, so every
  snapshot's bucket was empty, and restoring one emptied the dev bucket; a
  snapshot with no objects now leaves the bucket alone.
  Restoring production data locally without the scrub is documented as
  something not to do.

The `scrub.sql` step is also what makes a public "sample database" possible later, if
birdtest ever wants to publish its analysis corpus.

### Implementation phases

**Phase 1 — harden what exists** (infrastructure only, no code). `backup_retention_period`
7 → 30, `copy_tags_to_snapshot`; CRR from the artifacts bucket to a DR-region bucket;
lifecycle rules expiring noncurrent versions after 90 days and aborting incomplete
multipart uploads after 7. Delivers a 30-day PITR window and artifacts that survive a
region.

**Phase 2 — the nightly dump.** The backup bucket with versioning, public access
block, SSE-KMS, lifecycle and Object Lock; the `birdtest-backup` ECS task definition;
the nightly schedule; `scripts/backup.sh` doing `pg_dump -Fd -j4 -Z6`, computing row
counts and the digest, writing `manifest.json`, syncing the directory, emitting the
CloudWatch metric and inserting the `backups` row; the alarms and the SNS topic.
Delivers portable, encrypted, off-instance backups with failure alerting.

**Phase 3 — application support.** The `backups` table and
`leave_generation_artifacts.sha256`, with the built KLV hashed at write time in
both `run_transition` and `seed_zero_generation`; `GET /api/admin/backups` and its
dashboard card; `POST /api/admin/jobs/:id/rebuild-artifacts`; destructive endpoints
recording what they destroyed before destroying it. Delivers backup state visible
where admins already are, and artifact drift that is detectable rather than
invisible.

**Phase 4 — restore tooling and drills.** [RUNBOOK.md](RUNBOOK.md) as literal
copy-pasteable commands with the counter-repair SQL spelled out; the local
dump/restore/scrub scripts; the monthly automated restore drill; a round-trip test
that brings up `docker compose`, seeds a row in each of seven core tables (users, input data, jobs, tasks,
claims, game results, backups) with
plain SQL, dumps, drops, restores and asserts the verification checks pass.
(`scripts/restore-roundtrip.sh` seeds directly rather than through a worker: what
it is testing is `pg_dump`/`pg_restore`, and going through the worker API would
add a client to the failure surface without adding a row shape.)

#### Where the implementation differed from the plan

- The **backup bucket is replicated cross-region too**, not just the artifact bucket.
  The 24-hour region-loss RPO is not met by replicating only derivable data, and the
  marginal cost of a second replication rule is a KMS key in the DR region.
- The **restore drill restores into a Postgres of its own, inside the drill task**
  (`DRILL_TARGET=local`): `initdb` on the task's ephemeral storage, a Unix socket
  only, durability off. It still needs no provisioning — the task runs the postgres
  image — and it never touches the production instance. It first restored into a
  second database *on* that instance, on the reasoning that `max_allocated_storage`
  covered a second copy; the eleventh audit found it does not reliably (RDS grows
  storage only after free space has sat under 10% for five minutes, one step at a
  time, six hours apart, and never shrinks it), so a drill near the free space could
  fill production mid-restore, and one that did not ratcheted its storage up for
  good and spent its burst credits. `DRILL_TARGET=server` keeps the old behaviour
  for a hand-run drill against a scratch instance. `restore_drill_enabled = false`
  turns it off.
- **`multi_az` is a variable defaulting to false**, not a change. It doubles the
  instance cost, and it is availability rather than backup — the call belongs to
  whoever pays for it.
- The **round-trip test is a script, run nightly rather than per pull request**:
  `.github/workflows/nightly.yml` runs `restore-roundtrip` and `backup-drill`
  beside tier 6 (TESTING.md, "Nightly").
- `scripts/backup.sh` and `scripts/restore-drill.sh` honour **`AWS_S3_ENDPOINT`**, so
  both run against the local MinIO. That is how they were tested — a real dump of the
  real schema, uploaded, downloaded, restored and verified.

**Phase 5 — optional hardening.** Not implemented: an AWS Backup vault with Vault
Lock and cross-account copy; the `birdtest-restore` subcommand, once the runbook has
been used in anger at least once (the counter-repair SQL it would replace is written
out in [RUNBOOK.md](RUNBOOK.md) §2.3 in the meantime); and a second AWS account, the
only configuration that survives full account compromise.

#### Decisions settled before Phase 2

1. **Object Lock on the backup bucket** — enabled at creation, governance mode,
   `backup_object_lock_days = 30`. It could not have been added later without
   recreating the bucket.
2. **Retention** — `backup_retention_days = 365`, Glacier IR at 30 days, noncurrent
   versions expiring at 90. Storage is cheap; the real limit on retention is that old
   dumps become unrestorable as the schema moves.
3. **Dump source** — production directly. Revisit when a dump exceeds ~30 minutes;
   `duration_seconds` in the manifest and the `DurationSeconds` metric are what to
   watch.
4. **`desired_count` stays 1**, and `infra/variables.tf` now refuses anything else:
   imports, rate limits and SSE subscribers are all per-process. The *service*
   enforces it too, with `deployment_minimum_healthy_percent = 0` and
   `deployment_maximum_percent = 100`: ECS's default rolling deploy runs the new
   task alongside the old one, and a starting process marks any import or export
   left `running` as failed on the assumption that whoever owned it is gone — so
   under the default, every deployment would fail the outgoing instance's live
   work. Stopping first costs a gap with nothing serving, against an
   invariant that otherwise does not hold exactly when the code changes. That
   gap is **a minute or two, not seconds**, and only because the Terraform
   says so: ECS waits out the target group's deregistration delay before it
   even sends `SIGTERM`, and the defaults (300 s of draining, then three health
   checks thirty seconds apart for the new task) made it seven or eight
   minutes — longer than the heartbeat timeout. `infra/ecs.tf` sets 10 s of
   draining (30 s until October 2026; a cut upload is retried) and two checks
   ten seconds apart. Only the backend's service works this way: the frontend
   has its own service, which rolls. Three things are sized to ride
   out what is left: MAGPIE retries a refused or `5xx` request for about
   fifteen minutes, and a task claim for as long as it takes (see [HTTP](#http-srccompatchttp--srcutilhttp_client)), a
   restarted server reclaims no claim until it has been up for the heartbeat
   timeout (see [Task States](#task-states)), and the process ends its SSE
   streams on `SIGTERM` rather than waiting for a `SIGKILL` (see [Health and
   startup](#health-and-startup)). The
   terminal writes are guarded on `state = 'running'` as well, so a reaped row
   stays reaped rather than coming back `staged` or `ready` with the reaper's
   error still on it.
5. **Regenerable tables stay in the dump.** Excluding `tasks` and the request tables
   would make every restore a partial restore that has to re-derive state, which is
   exactly the complexity a backup exists to avoid.

---

## Development

Everything needed to run birdtest locally runs on a laptop with no AWS access. `docker compose up` is the whole stack; the worker doing real work against it is MAGPIE, and `scripts/dev.py` runs both. AWS services (SES, SSM, S3) are stubbed or swapped for local equivalents in dev; only the deployed environment touches real AWS.

### Prerequisites

**Docker and a MAGPIE build.** The backend, frontend, Postgres and the S3
stand-in all run as containers, so no Rust, Node, Python or Postgres install is
required on the host; MAGPIE is built on the host, outside Docker.

| Tool | Used for |
|---|---|
| Docker / Docker Compose | The entire stack |
| A C toolchain (`make`, a C compiler) and a MAGPIE checkout with MAGPIE-DATA | The MAGPIE binary the backend runs and the contributors are |

The compose stack needs a MAGPIE build: the backend runs one for every derived
file and every leave-generation KLV, and refuses to start without it. Set
`MAGPIE_ROOT` to a MAGPIE checkout with a built `bin/magpie` (`make magpie
BUILD=portable_release`; it defaults to `../MAGPIE`), which compose mounts into
the backend; the backend's `MAGPIE_BIN` is set inside the compose file, so
setting it in the shell or `.env` has no effect on the stack. The same checkout is what runs work against the
stack, because MAGPIE is the only worker client. `worker/fake_worker.py` is an end-to-end-suite instrument that submits
invented results, not a way to develop against the stack. See [Contributing
locally](#5-contributing-locally).

Working directly on the host is still supported and needs Rust (stable), Node
(LTS) and Python 3.11+ per component; see
[Without Docker](README.md#without-docker) in the README.

### 1. The whole stack

```bash
docker compose up --build
```

This brings up five services:

| Service | Role |
|---|---|
| `postgres` | Postgres 16, on `${POSTGRES_PORT:-5432}` |
| `minio` | S3-compatible object storage, on `${MINIO_PORT:-9000}` |
| `minio-init` | One-shot: creates the artifact bucket, then exits |
| `backend` | The Axum server, on `${BACKEND_PORT:-8080}` |
| `frontend` | Nginx serving the SvelteKit build, on `${WEB_PORT:-5173}` |

The site is at `http://localhost:5173`. Nginx proxies `/api` to the backend —
the same split the ALB performs in production — so the app is single-origin
locally too, and the session cookie and CSRF double-submit behave identically.
SSE needs `proxy_buffering off` on that location, or the job stream is buffered
and the dashboard never updates.

`backend` waits on a Postgres healthcheck and on `minio-init` completing;
`frontend` waits on the backend's `/health`. Migrations run inside the backend
process before it binds, so there is no separate migration container.

Every host port is overridable via `.env` (see `.env.example`), so a machine
that already has something on 5432 does not need the compose file edited.

### 2. Resetting the database after a schema change

There is a single migration until release, and schema changes edit it in place.
sqlx records a checksum for each applied migration, so an edited `0001` will not
apply over a database that already has the old one — the backend will fail to
start with a "migration was previously applied but has been modified" error.

Drop the schema and let the backend rebuild it:

```bash
docker compose exec postgres \
  psql -U birdtest -d birdtest -c 'DROP SCHEMA public CASCADE; CREATE SCHEMA public;'
docker compose restart backend
```

`./scripts/dev.py --reset-db` does the same before starting the stack (and says
so when a start fails this way). `docker compose down -v` also works but
discards the MinIO bucket with it.

### 3. Configuration

The backend's environment is set inline in `docker-compose.yml` rather than
from a file, so the default stack has nothing to copy first. `MAIL_BACKEND` is
`console`, which logs confirmation codes and reset links to the container's
stdout (`docker compose logs -f backend`) instead of sending them — there is no
local SES, and standing up a real mail sink is not worth it for dev.
`S3_ENDPOINT` points at MinIO; the AWS SDK works against it unmodified, so
there is no separate code path.

`backend/.env.example` documents the same variables for running the server
directly on the host with `cargo run`.

### 4. Optional profiles

```bash
docker compose --profile dev up          # adds Vite with HMR on :5174
docker compose run --rm derived-builder  # on demand: build the wordmaps and rack
                                         # info tables queued jobs are waiting on
```

The builder is the same image with the entrypoint production's scheduled task
uses (`infra/derived.tf`), run once and exited. Nothing else in the stack
builds a derived file, so a job whose players ask for a wordmap or a rack info
table -- a leave-generation job does by default -- is not dispatched until it
has run; `/admin/derived-data` shows the queue. `scripts/e2e_magpie.py` runs
it for the jobs it creates that need one, which is what makes the server-built
hash, the worker's own build and the comparison between them part of the
nightly end-to-end run.

The `dev` profile runs the Vite dev server with `frontend/` bind-mounted and
`node_modules` in a named volume, so hot reload works without Node on the host
and the container's install never collides with a host one built for a
different platform. It runs *alongside* the Nginx build rather than replacing
it, on a separate port.

### 5. Contributing locally

A real contributor client is MAGPIE itself, not anything this compose file
builds — point a local `contribute.txt` at `http://localhost:${WEB_PORT:-5173}`
and run `magpie contribute`. See [Worker Client](#worker-client-1).

The worker needs an actual admin-created, activated job to have anything to
claim (see step 6) — with no active job a claim just gets 204s and the worker
sleeps in its retry loop, which is expected and not an error. It also needs the
job's pinned input data on disk, or it will decline every task it is offered and
say which file it is missing; see [Capability negotiation](#capability-negotiation).

### 6. Seeding a local admin and a first job

There's no seed script needed for the minimum path — the first registered user isn't automatically an admin (avoids a footgun where every dev DB has an implicit admin), so promotion is a one-line manual step against the local DB:

```bash
# 1. Register normally through the frontend (or POST /api/auth/register), then confirm
#    the email — MAIL_BACKEND=console means the confirmation code is in the backend's
#    log (docker compose logs -f backend) rather than an inbox.
# 2. Promote that user to admin directly in Postgres (no API for this by design —
#    is_admin is not settable through any endpoint):
docker compose exec postgres \
  psql -U birdtest -d birdtest -c "UPDATE users SET is_admin = true WHERE username = 'you';"
```

From there, use the now-admin account's session to create a player config and a job through `/admin/player-configs/new` and `/admin/jobs/new` (or the equivalent `POST /api/admin/...` calls directly), then give the job an allocation on `/admin/allocation` (`PUT /api/admin/jobs/allocations`), which activates it. Once a job is active, `magpie contribute` (step 5) will start claiming and completing real tasks against it, and the dashboard at `http://localhost:5173/jobs/:id` updates live via SSE — this is the fastest way to confirm a full change (backend, frontend, and worker together) actually works end to end.

### Running the checks

```bash
(cd backend  && cargo test --lib --bins)  # unit and contract tests; no database needed
(cd backend  && TEST_DATABASE_URL=postgres://birdtest:birdtest@localhost:5432/birdtest \
                TEST_S3_ENDPOINT=http://localhost:9000 \
                cargo nextest run)        # plus tiers 2-3 in backend/tests/
(cd backend  && cargo clippy --all-targets)
(cd frontend && npm run check && npm test) # svelte-check, then the tier-1F unit tests
```

[TESTING.md](TESTING.md) is the full picture: seven tiers, what each may touch,
the shared seed and fixture data, and how the development environment is the
end-to-end suite with its assertions and teardown removed.
