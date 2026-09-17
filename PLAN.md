# birdtest — Project Plan

## High-Level Design

### Overview

birdtest is a crowdsourced word game analysis platform, modeled after Fishnet (which crowdsources chess game analysis for Lichess). Users contribute compute by running tasks locally and submitting results back to the site. Admins define jobs and allocate work; the site aggregates results and presents them on a polished dashboard.

---

### Jobs

Jobs are long-running research goals defined and managed by admins. The following job types are supported:

- **Analyze all possible opening racks**
- **Run games** — autoplay using any player configuration; supports pure static players (no simulation), simming players, or any mix.
- **Run game pairs** — same as games but run as matched pairs (same seed, players swapped) to reduce variance.
- **Leave generation**

Each job has a **percentage allocation**, and nothing else decides who gets work: every claim goes to the active job furthest behind its share (measured from when the job last joined the jobs on offer, not over its whole life — see [Workflow](#workflow), step 2), and the active jobs may allocate at most 100% between them (enforced at the application layer, not via a DB constraint, and serialized so two concurrent activations cannot jointly exceed it). There is no priority. A job that should get nothing for now is set to **0%**, which is exactly what the inactive state means, and a job that should get everything is the only one above 0% — so allocation alone expresses every ordering an admin needs, without a second axis to keep coherent with it. Jobs and their allocations are managed by admins only.

#### Job Lifecycle Controls

Jobs are created by admins in the **inactive** state and only start receiving work once explicitly activated. The following states are supported:

- **active** — workers are assigned tasks from this job normally.
- **inactive** — the job exists and retains all its tasks and results, but workers are not assigned tasks from it. Admins can reactivate it at any time. An active job at **0% allocation** is in the same position — offered to nobody — so the two are interchangeable ways of parking a job; the state is what records that an admin switched it off, and 0% is what an admin sets while rebalancing the others.
- **completed** — all tasks have been completed, either automatically when the finish condition is met or manually by an admin.

Jobs are created in the **inactive** state. Allocation is not set at creation time — it is supplied by the admin when they activate the job. This keeps the allocation budget coherent: an admin reviews the full set of active jobs, decides the new job's share, and activates it with a specific percentage in a single action.

Admins can deactivate, reactivate, purge, force-complete, or delete a job at any time. **Purge deletes tasks outright** rather than returning them to `available`: every job type generates its tasks on demand, so a purged job regenerates them from the start of its space at the next claim. Leaving the rows behind would advance the seed cursor past work that was never done. Purging also rebuilds the generation-0 zeroed KLV a leave-generation job needs before it can dispatch; the generation-1 rack universe it deleted is seeded again by the first claim, as every generation's is. It takes the job's **merge lock** before anything else — a merge of a leave job's staged results takes the staged rows and then the per-rack rows, a purge deletes them the other way round, and run together the two deadlocked, with the purge the likelier victim: a `500` and nothing deleted (`leave_gen::lock_merges`; it is first in the lock order everywhere, because nothing takes it while holding another lock). Then the job's dispatch lock, the same one every claim takes: a claim in flight has already read the seed cursor and is about to insert its task, which the purge's deletes cannot see, so without the lock the purge finishes and the claim then commits a task into the job it just emptied. It then waits out every open claim's in-flight submission before taking the job's row, which is the order every submission locks in (claim, then task, then job): taken the other way round, a submission arriving mid-purge deadlocked against it, and one that committed between the purge counting contributions and deleting claims was never handed back. Delete takes the same three locks for the same reasons.

A completed job cannot be reactivated, and for the same reason cannot be deactivated: deactivate-then-activate would otherwise restart it. Force-completion is unconditional.

---

### Tasks

Tasks are the atomic units of work that workers execute. The following task types are supported:

- Analyze a contiguous range of opening racks
- Play a batch of games from a given starting seed
- Play a batch of game pairs from a given starting seed
- Play a batch of leave-generation games over a forced subset of racks

Every task carries a **seed** — a `uint64` value — because every job type plays games, and what a game samples must be a function of the task and never of the worker. The combination of `(job_id, seed)` must be unique; duplicate tasks are prevented at the database level. Games and game pairs seed their batch from it and step by one per game, so the seed is also the cursor that tiles the job's seed space. An opening-rack task's seed is the index of its first rack in the job's rack space — rack `i` of the batch is analysed from `seed + i`, the same number on every worker — so the same column and the same index tile that space too. A leave-generation task's seed is drawn when the task is created and stored with it, so a reissued task replays it; two leave tasks are otherwise kept apart by the rule that a claim is never handed a rack another open claim is already forcing (see claim step 2 under Leave Generation), and a collision between two random draws is a unique violation the claim path retries.

#### Task States

Tasks move through an explicit state machine:

```
available → claimed → completed
               ↑          
         (heartbeat timeout on a claim: one slot reopens; task returns to available if it had been at capacity)
```

State is determined by denormalized counters (`accepted_count`, `active_claim_count`) relative to the job's `redundancy` value X:

- **available**: `accepted_count + active_claim_count < redundancy` — open capacity remains; workers can claim a new slot.
- **claimed**: `accepted_count + active_claim_count = redundancy` but `accepted_count < redundancy` — all slots are filled with in-flight claims; waiting on results.
- **completed**: `accepted_count = redundancy` — all X results have been submitted and accepted.

Individual claims are rows in `task_claims`. When a claim's heartbeat times out, that claim row is flipped to `abandoned`, `active_claim_count` is decremented, and if the task was at capacity it returns to **available**. Reclamation is lazy — it runs at the moment the next task is requested and the job is a candidate, not via a background process. A job nobody asks for work from — one that is inactive, completed, or parked at 0% — therefore keeps a lapsed claim on its books until something reclaims it: activation above 0% makes it a candidate again, and starting an [export](#exports) reclaims the job's lapsed claims first, since a completed job is never claimed from again.

#### Task Generation

Every job type generates its tasks **on demand**: the next task request is generated, inserted and claimed in one transaction at claim time. A task with capacity left — its claim lapsed, or its redundancy is not yet filled — returns to `available` and is re-dispatched before anything new is generated. There is no pre-populated strategy (see [Creation Strategies](#creation-strategies)).

---

### Workflow

1. The worker sends a **task claim** to the server — a minimal message identifying itself and signaling it is ready for work.
2. The system selects the active job **most behind its configured allocation share** — specifically, among active jobs with an allocation above 0%, the one with the lowest ratio of `(claims_issued - claims_baseline) / allocation`, where `jobs.claims_issued` counts every claim ever issued for that job, **including abandoned and declined ones** — a claim consumed real dispatch capacity at the moment it was issued regardless of what happened to it afterward, so the count only ever goes up (a purge, which deletes the claims it counts, resets it). It is a counter rather than a `COUNT(*)` over `task_claims` because selection runs on every claim request, and a count grows with each job's whole history. Excluding abandoned claims would let a job with flaky or slow workers accumulate a disproportionate share by having its timeouts discounted, and would make the count non-monotonic — the opposite of what the deficit-based scheduler needs. Ties are broken by job creation order (oldest first). This is a deterministic deficit-based selection; no randomness is involved.

   **A job's share is measured from when it joined, not from when it was created.** `jobs.claims_baseline` is reset — on activation, which is also how an allocation is changed, and on a purge — so that the job's ratio equals the **lowest ratio among the other jobs being served** (`scheduler::join_at_parity`): it joins level with the job furthest behind and takes its share from then on. *Being served* is narrower than *offering work*, and the difference matters: a job can be active and above 0% and still stand still — its derived files are building or failed, it is pinned to data the fleet does not have yet, its MAGPIE floor is above what the workers run, its generation is mid-transition — and its ratio does not move while the others' climb. Put level with *that* job, a newcomer was, for every worker that could not run the lagging one, first in the list until it had caught up with the jobs that were running: a veteran at 100,000 claims, a job nobody could run at zero, and a newcomer took twelve of the next twelve claims. So a job counts toward parity only if it issued a claim within the heartbeat timeout — `jobs.last_claimed_at`, which rides the `UPDATE jobs` every claim already makes — and when no job has (a quiet server, the first activation in a while) every job on offer counts, as before. One case this does not reach, and no single ratio can: a job only a *minority* of the fleet can run is served, recently, and still lags, so a newcomer level with it is ahead of the rest for the majority. A fleet split that way is two queues, and the scheduler has one. This is start-time fair queuing's rule, and it is what makes "no starvation" true. Measured over a job's whole life, as it was, every change to the set of jobs was a takeover: a job activated beside one that had issued two million claims had a ratio of zero, so it was first in every candidate list until it had issued two million of its own, and the older job — at the same 50% — got *nothing* for as long as that took. A purge (which zeroes `claims_issued`), a reactivation after a week switched off, and an allocation raised from 10% to 50% (which cuts the ratio to a fifth) all did the same. With the baseline, the shares an admin sets are the shares the fleet sees from that moment, selection is still one deterministic statement, and the long-run ratios still converge on the allocations, because every job's numerator counts from the same point in the fleet's history. The baseline can be negative (a job with no claims joining a busy fleet is credited the claims that put it level); ratios never are. One thing it deliberately does not cover: a job that stays a candidate but hands out nothing for a long stretch — waiting on a derived-file build, or on workers that can run it — still returns behind its share and is favoured until it catches up. That is a job being owed work it was offered and could not take, which is what a deficit is for; re-activating it forgives the debt if an admin would rather.
3. Expired claims for the candidate jobs are lazily reclaimed, in one statement: each timed-out `task_claims` row is flipped to `abandoned`, `active_claim_count` is decremented, and tasks that were at capacity return to `available`.
4. The system acquires the next task (one being re-dispatched, or else one generated on demand), inserts a `task_claims` row, increments `active_claim_count`, and issues a claim token (UUID) to the worker.
5. The server responds with the **task request** for that job type.
6. The worker performs the task and submits a **task response** along with the claim token.
7. If the claim token matches a `task_claims` row that is not abandoned and was issued to the identity presenting it (a token presented by any other identity is treated as unknown, so bans and audit rows mean what they say), the task response is accepted, a **task record** is stored keyed to the `task_claim_id`, `accepted_count` is incremented, and `active_claim_count` is decremented. When `accepted_count = redundancy` the task is marked **completed**. If the token is stale (the claim was abandoned due to timeout, or this result was already accepted), the submission is answered `{"accepted": false}` and changes nothing. The claim row is locked from lookup to commit, so a timeout reclaiming it concurrently cannot count it as well.

---

### Workers

Workers are the clients that perform tasks and submit results. Two types are supported:

- **Anonymous workers**: Identified by a UUID **the server mints**, not one the client invents. A worker with no credentials sends no identity header at all on its first claim; the server draws a UUID, and only when that claim actually hands out a task does it insert the `anonymous_workers` row (in the claim's own transaction) and return the UUID in the response body. A claim answered `204` writes nothing — otherwise every idle poll from a new contributor would mint and orphan an identity — and every worker endpoint other than the claim answers `401` without an identity. The client persists it and sends it as `X-Worker-UUID` from then on. A UUID that does not already exist in `anonymous_workers` is rejected with `401` and a message naming the fix, because a client-invented identity is one the server never got to validate — it would let anyone manufacture contributors, attribute work to identities that never claimed anything, and hand per-worker anomaly detection a population it does not control. Contributions are tracked per UUID but **displayed under a pseudonym**: the first 16 hex characters of the UUID's SHA-256, labelled "Anonymous". The UUID is the worker's only credential, so no public endpoint returns it (`/api/workers`, job stats `workers` and `/api/jobs/:id/results` all carry `anon_id`, and `?worker=` accepts it); only `GET /api/admin/workers` returns real UUIDs, for banning. A request from a known UUID refreshes `last_seen_at`, but at most once a minute — it answers "is this worker still around", which a per-minute resolution answers just as well as a write on every request would (`api_keys.last_used_at` is throttled the same way).
- **Authenticated workers**: Identified by an API key tied to a user account. Contributions are tracked per user.

#### Worker Integrity and Anomaly Detection

- **Plausibility checks at submission time** — every submission is checked against what is *possible*, not against what is usual: a negative standard deviation, a play scoring negative points, a rack with eight tiles, a batch reporting a different number of games than the task dispatched. Implemented in [`backend/src/jobs/plausibility.rs`](backend/src/jobs/plausibility.rs). This is the only active integrity mechanism at submission time, and the rest of this section explains why it is the only one that can be.
- **Worker ban list** — a persistent table of banned worker identities; banned workers cannot claim or submit tasks. Meaningful for authenticated workers; for anonymous workers, banning targets the UUID. Applied by an admin; nothing bans automatically. **One row per identity**, enforced by a partial unique index: enforcement is an `EXISTS`, so a second row's reason is never read, while unban deletes by row id — so a duplicate would leave an identity banned after an admin had lifted the ban, with nothing to say why. Banning again with a different reason is unban-then-ban, which the audit log records as both halves.
- **Redundant task execution** — each job specifies a redundancy value X; X independent workers must each complete the task. All X results are stored independently. No consensus or agreement check is performed at submission time — reconciliation is a downstream analysis question deferred past v1. Every aggregate that treats results as observations — SPRT, progress counts, the job list, rating evidence and a leave generation's occurrence totals — reads **one result per task**, the first accepted: games are seeded and deterministic (a leave-generation task carries its seed too), so the other copies replay the same games, and counting them would multiply the evidence by the redundancy. "First" is the task's `accepted_count` read as zero under the task's row lock, which every submission takes before storing anything. A later result for a leave task is credited to its worker (`leave_records`) and not folded into `leave_rack_progress` — a guard rather than a path, since a
leave-generation job is created at redundancy 1 only.

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
statistic available is the win rate, and that is precisely what SPRT is
measuring. A test on it cannot separate "this worker is broken" from "these
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
| A play scores between 0 and 2,000 | A pass scores 0 and the theoretical maximum play is a little over 1,700 |
| Win percentages and blended utilities are inside their ranges | A probability is bounded by definition |
| A rack has 1–7 tiles | More tiles than a rack holds cannot be dealt |
| `num_moves` is at least the number of moves reported | A worker cannot report more moves than it says it generated |
| A leave submission lists no rack twice | Occurrences are **summed** on receipt, so a duplicate silently inflates a generation's coverage |
| A batch reports exactly the games the task dispatched | The size was fixed when the task was handed out |
| An opening-rack batch analyses exactly the racks the task dispatched | The racks themselves were named when the task was handed out |

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

The natural next step, when a job first runs at redundancy > 1, is **cross-checking
replicated tasks**: at that point N workers do run the same seed with the same
configs, `game_results` already stores each claim's row separately, and games are
deterministic, so disagreement becomes proof rather than evidence. That is where
detection with real teeth lives, and it needs no population statistics at all.
It applies to jobs whose players are all static: a simulation samples, and its
thread count alone makes two honest workers' rankings differ, so simming jobs are
excluded from any equality cross-check.

---

### Worker Client

Contributors run a client program that loops continuously: it sends a **task claim** to the server, receives a **task request**, executes the work, and submits a **task response**. The client handles authentication (API key or anonymous UUID) and heartbeating automatically. See the [Worker Client](#worker-client-1) section for technical details.

---

#### Letter distributions are stated, not inferred

Every job config names its `letter_distribution` alongside its lexicon, and the
name is carried on the request the worker receives. MAGPIE derives a
distribution from the lexicon's prefix; birdtest used to mirror that inference,
which meant guessing at something the job can simply say.

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
rather than discarded, turning a job run to settle an Elo question into a corpus
of analysed positions as well.

They share `position_analysis_records` with opening racks -- the request differs
by job type, but what comes back is a position analysis either way. In-game rows
carry the CGP, the game index and the turn number, which are NULL for an opening
rack.

Two things this design turns on, both consequences of games being deterministic:

- **Redundant claims replay identical games**, so in-game positions are keyed on
  `(task_id, game_index, turn_number)` rather than on the claim, with
  `ON CONFLICT DO NOTHING`. The first accepted claim records them and the rest
  are no-ops, so redundancy still verifies the *result* without multiplying the
  corpus. Opening racks keep their per-claim key, so redundant analyses of the
  same rack can still be compared.
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

**An opening-rack job cannot use a `best` recorder to rank moves.** `-r best`
is `MOVE_RECORD_BEST`: move generation keeps the single top play and discards
the rest, so every rack comes back with one move however many
`num_plays_recorded` asks for — and a simming player has nothing left to choose
between, which makes `num_plies` and `num_plays` inert too. Nothing downstream
notices: the racks are analysed, the results accepted, `racks_analyzed` climbs,
and the corpus quietly holds a fraction of the analysis the job was configured
for. Job creation therefore refuses `recorder_type = 'best'` together with a
`num_plays_recorded` above 1, and names the remedy. `best` with
`num_plays_recorded` of 1 stays legal, because "the best opening play for every
rack" is a real job. The rule is scoped to opening racks: a `games` job's
players go through autoplay, where a simmer's candidate list is sized by
`num_plays` rather than by the move recorder, and `best` is right there.

`position_analysis_plies` is populated only for **simming** player configs. A static player produces no per-ply statistics, so for the common case the table stays empty rather than filling with placeholder rows.

### Statistical Result Evaluation

For game and game-pair jobs, results are evaluated using the Sequential Probability Ratio Test (SPRT). A job has two finish conditions:

1. **SPRT significance**: SPRT is evaluated as results are submitted (debounced, below), and acted on once `min_games` (or `min_pairs`) have been completed. The job auto-completes when a check finds the LLR past the significance boundary.
2. **Hard cap**: the job auto-completes when `max_games` (or `max_pairs`) is reached, regardless of SPRT outcome. **No task is generated past the cap**: once every game (or pair) up to it has been handed out, a claim finds nothing new to generate, tasks whose claims lapse are still re-dispatched, and the job completes as their results land. Generating beyond the cap handed out work that could not change the verdict — as many batches as workers asked before the debounced check next ran.

SPRT is evaluated inline on the submission path (no background sweep), and
**debounced**: every eighth submission for a job, plus unconditionally whenever
that job has nothing left in flight. The server flips the job to `completed`
automatically when either condition is met.

The debounce trades *when* a job notices it is finished for the cost of
noticing, and nothing else — the check still reads `game_results`, so a
debounced check is late, never wrong. That is what separates it from replacing
the read with a counter, which would make a drifted counter able to stop a job
early (see [What these reads cost](#what-these-reads-cost-measured)). The cost
is bounded at seven extra tasks, and the first several of those are free: when
the LLR crosses, the job flips to `completed`, but every task already claimed
across the fleet is still played and still accepted, because the submit path
validates the claim rather than the job's status. A bound at or below the number
of tasks typically in flight therefore wastes nothing that was not already going
to be wasted.

The unconditional check when nothing is in flight is a correctness cover rather
than an optimisation. The check is triggered *by* submissions, so a job whose
contributors all stop between checks would not be evaluated again until work
resumed — which, for a job that has already reached its stopping point, means
never: it would sit `active` holding its allocation.

#### How the LLR is computed

The hypotheses are stated in Elo: H0 says the difference is `elo_low`, H1 says
it is `elo_high`. The test uses the normal approximation fishtest uses — treat
the sample as draws from a distribution with unknown mean and compare the
likelihood of the observed mean under the two hypothesised means:

```
expected_score(elo) = 1 / (1 + 10^(-elo/400))

llr = n · (µ₁ - µ₀) · (mean - (µ₀ + µ₁)/2) / variance
```

where `µ₀ = expected_score(elo_low)` and `µ₁ = expected_score(elo_high)`. The
acceptance bounds are `ln(β / (1-α))` and `ln((1-β) / α)`.

What differs between the two job types is **what one observation is**, and that
choice is the whole statistical content of the test:

| Job type | Unit | Score | n |
|---|---|---|---|
| `games` | one game | 1 / 0.5 / 0 | games played |
| `game_pairs` | one **pair** | `i / 4` for pentanomial bucket `i` | pairs played |

For a plain `games` job the sample is per-game, with
`mean = (wins + 0.5·draws)/n` and `second_moment = (wins + 0.25·draws)/n`.

#### The pentanomial, and why pairs are the unit

A `game_pairs` task plays each seed twice with the players swapped. The two
games of a pair **share a seed**, so they are not independent of each other —
counting them as two observations overstates how much evidence there is. The
pair is the independent unit, and its outcome is one of five: player 1 lost
both, lost one and drew one, split, won one and drew one, or won both. MAGPIE
reports those five counts directly (see [MAGPIE reports the
pentanomial](#magpie-reports-the-pentanomial)), indexed by player 1's half-point
score across the pair, and the sample's mean and variance are taken over pair
scores of `i/4`. That puts the mean on the same per-game scale
`expected_score(elo)` expects while `n` honestly counts pairs.

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

| Sample | Score rate | Implied difference |
|---|---|---|
| Every pair (10,000 of them) | 0.4975 | about **-1.7 Elo** |
| The 50 divergent pairs alone | 0.01 | about **-800 Elo** |

Same games. A test fed the second number crosses any boundary it is given,
almost immediately, on a hundredth of the evidence — so an SPRT configured to
resolve ±10 Elo stops being able to resolve anything at all, and reports every
difference as decisive. The divergent counts are still collected and still
shown, as a **diagnostic** of how often two configs differ at all. Nothing is
tested on them.

**The LLR is 0 for a degenerate sample** — no observations yet, or zero observed
variance, which is what a run in which every pair split produces. The test has
not begun to discriminate, and reporting 0 rather than dividing by zero is what
keeps the dashboard honest about that.

**The sample size and the progress count are the same number.** `min_pairs` and
`max_pairs` gate on pairs played, and the pentanomial's sample is pairs played,
so the two cannot drift apart. (They could, and did, when the LLR ran over a
filtered subset of games while the gates counted pairs — a job would then either
end early or never end.)

The status is one of `running`, `passed`, `failed`, `terminated_at_max`. Below
`min_units` the LLR is computed and reported but never acted on — except that
the hard cap still applies, so a job whose `max_units` is below its `min_units`
terminates rather than running forever.

### Ratings

Ratings are **siloed from job control flow entirely**. Nothing in the rating
system is read while dispatching, claiming, validating or completing a task, and
no job decision reads a rating. The coupling runs one way: a fit reads finished
`game_results` and writes a snapshot. SPRT stays where it belongs, on the job
config tables — it is a per-job **stopping rule**, not a measurement, and
`elo_low`/`elo_high` are hypotheses about one comparison rather than anyone's
rating.

Everything below lives in four `rating_*` tables and one module. See
[Rating pools](#rating-pools) for the schema and the fit.

#### In one place: when a rating is computed, and where it is shown

A rating is never updated by a result landing. It is recomputed for a whole
pool, from scratch, by `ratings::fit_and_store`, on exactly three triggers:

| Trigger | When | Who | What is written |
|---|---|---|---|
| **evidence** | Every two minutes, from a sweep started by `main.rs` (`ratings::recompute_stale`), for each pool whose count of eligible pairs differs from its last run's `pairs_used` | The server, unattended | A new `rating_runs` row with a `player_config_ratings` row per member and a `rating_run_residuals` row per head-to-head — or nothing, if the evidence has not moved |
| **membership** | Immediately, inside the request, when an admin adds or removes a member (`POST`/`DELETE /api/admin/rating-pools/:id/members`) | An admin | The same, unconditionally |
| **manual** | Immediately, on `POST /api/admin/rating-pools/:id/recompute` | An admin | The same, unconditionally |

So a result submitted now is in a rating within two minutes, and a job that
finishes at 03:00 is rated by 03:02 without anyone doing anything. Nothing on
the claim or submit path reads or writes a rating, and no job decision depends
on one; SPRT is a separate, per-job stopping rule.

Ratings are **displayed on the ratings pages and nowhere else**:

- `/ratings` (`GET /api/rating-pools`) — every pool with its conditions,
  member count and the time of its last fit.
- `/ratings/[id]` (`GET /api/rating-pools/:id`) — the pool's **newest run**:
  each member's rating with its standard error as a dot plot and a table, the
  run's provenance (trigger, iterations, convergence, evidence consumed), and
  the residual table showing where the fit disagrees with the games. A config
  with no path of games to the anchor is listed as unrated rather than drawn.
  Admin membership controls appear inline here for admins.
- The **history chart** on the same page (`GET /api/rating-pools/:id/history`)
  — one line per rated member across the stored runs, thinned to at most 500
  evenly spaced points with the first and newest always kept.

A job's own pages (`/jobs/[id]`) show its SPRT verdict, win rate and
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

What birdtest actually has is a static tournament: N configs and a matrix of
pairwise results. The right tool is a **batch maximum-likelihood fit** over the
whole matrix at once — Bradley-Terry, solved by minorization-maximization,
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

MM is used rather than a gradient method because each step is a closed-form
ratio with no step size to tune, it cannot overshoot, and it converges
monotonically. For a pool of any plausible size it lands in microseconds, which
is what makes "refit everything on every change" affordable rather than
aspirational.

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
ratings page therefore carries the scalar rating as the headline and the residual
table beside it, and says so when the residuals are large enough that the
ranking should not be read as one.

#### What counts as evidence

Only `game_pairs` jobs, and only those matching the pool's scope, with **both**
configs in the pool.

Plain `games` jobs are excluded deliberately. `-gp` plays both orderings of every
seed, so a pair is **side-balanced by construction**; an unpaired job is not, and
going first in a word game is worth real Elo. Pooling unbalanced results would
bias every rating toward whoever happened to start more often. Including them
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
scale.

#### Two things the fit has to handle honestly

- **Separation.** A config that has never lost sends the unregularised maximum
  likelihood to infinity, and that is not an edge case — it is what a strong new
  bot's first job looks like. The fit adds a small prior (virtual drawn games
  against a player of the anchor's strength), which keeps every rating finite and
  pulls the barely-observed toward the anchor, where a wide standard error then
  says how little the number is worth.
- **Connectivity.** If two configs only ever played each other and neither
  connects to the anchor's component, their ratings are unidentifiable — the fit
  would otherwise return a confident number produced entirely by the prior. Each
  rating carries `connected_to_anchor`, and the page shows an unconnected config
  as **unrated** rather than as a plausible-looking 1500.

Standard errors come from the diagonal of the Fisher information. They ignore
off-diagonal terms, so they under-state the true uncertainty, but they are more
than good enough for the distinction the page needs to draw: 1700 ± 15 and
1700 ± 200 must not look alike.

#### When a fit runs

On membership change and on demand, immediately; on new evidence, from a periodic
sweep (two minutes) rather than a hook on result submission. A fit is global to a
pool, an active job submits results far faster than any rating needs to move, and
— unlike SPRT — nothing blocks on the answer. The sweep compares the pool's
current pair count against the last run's `pairs_used`, so no dirty flag is
needed anywhere; it reads the evidence once and fits from the same read, rather
than reading it to decide and again to fit.

**A fit holds the pool's lock while it runs** (`pg_advisory_xact_lock`, per
pool, for the fitting transaction). It is a read-then-write over state an admin
can change underneath it — it reads membership and evidence, then writes a run
stamped `now()` — so two at once interleave, and the fit that *started* first
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
resolution the history chart draws at anyway — it thins to 500 points over the
pool's whole life — so the chart keeps its shape and its ends; deleting
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

API keys are stored as hashes (never raw values) in the database. The raw key is shown to the user exactly once at generation time. Users may hold up to **100 API keys**, and a request to create the 101st is refused rather than silently evicting one. The count and the insert run under the account's row lock: the limit lives in the application rather than the schema, and counted and inserted as two bare statements, requests arriving together each read the same count. Each key can be independently marked **active** or **inactive** — only active keys are accepted for worker authentication. This lets contributors rotate or temporarily disable a key without deleting it. Every worker request authenticated with a key stamps its `last_used_at`, so a contributor can tell which of their keys is actually in use before revoking one.

**v1 account scope**: The sole v1 purpose of a user account is to generate an API token, which attributes task submissions to that account instead of an anonymous UUID. No other feature is gated behind registration. Anonymous workers can complete tasks fully, with no account or API token required.

#### Account Creation Flow

1. User fills out the registration form (`/register`) with username, email, and password.
2. The server validates, and returns `400` with field-level errors listing **every** problem at once rather than the first:
   - Username is 3–32 characters, trimmed.
   - Email contains `@` and is at least 3 characters, trimmed and lowercased — so
     case is not a way to register the same address twice.
   - Password scores at least **3** on zxcvbn, with the username and email passed
     in as context so a password derived from either is rejected. Scored
     server-side; the client shows the same feedback but is not trusted.

   A **taken username** returns `409` naming it. The user has to choose another
   one to get anywhere, and `GET /api/users` publishes the whole list anyway, so
   there is nothing here to protect.

   A **taken email** does not. Saying "that address is already registered" would
   make this endpoint an oracle for whether a given person has an account —
   something login and password reset both go out of their way not to reveal,
   and which registration should not undo. The caller gets byte-for-byte what a
   new registration gets, no account is created, and a notice goes to the
   address's owner telling them someone tried and pointing them at login and
   password reset. A real person who has forgotten they signed up still finds
   out; someone probing the address learns nothing.

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

1. User submits the login form (`/login`) with username and password. Attempts are rate limited per client IP and, separately, per username (10 a minute each), checked before any Argon2 verify runs.
2. The server looks up the user by username. If not found, or the password does not verify, it returns `401` with an identical message for both, so the response body cannot be used to enumerate accounts.

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
5. On submit, `POST /api/auth/reset-password/confirm` re-scores the new password, validates the token (hash match, not expired, not already used), sets `used_at`, hashes and stores the new password, and **spends every other outstanding reset token for that account** so an earlier link cannot be replayed. It clears the caller's session cookie.

   The reset also **revokes every existing session**. Each session token carries the account's `session_generation`, and `CurrentUser` compares it with the `users` row it already reads on every request; the reset increments it, so every token minted before it — an attacker's included — stops working. `POST /api/auth/sign-out-everywhere` (the "Sign out everywhere" button on the account page) and account deletion increment it the same way.
6. The user is redirected to `/login` with a success message.

---

### Security

**CSRF**: CSRF protection applies to session-cookie-backed endpoints only (Auth API, Account API, Admin API). Worker endpoints (`/api/worker/*`) use bearer tokens or the `X-Worker-UUID` header — neither is sent automatically by browsers, so they are not susceptible to CSRF and are exempt.

**Rate limiting**: Public unauthenticated endpoints are protected against abuse with per-IP (and per-UUID for worker endpoints) rate limiting enforced at the Axum middleware layer using the `governor` crate (token bucket algorithm). Rate-limited responses return `429 Too Many Requests` with a `Retry-After` header. The specific limits, and how a client's address is determined behind a proxy, are in [Rate limits](#rate-limits).

For v1, rate limit state is held in-memory (resets on process restart). A persistent backend can be added later for cross-instance coordination.

API keys and session tokens are handled as described in the Auth tech stack note below.

---

### Dashboard

The dashboard has two levels: a **job list page** and a **job detail page** per job.

Live updates are delivered via **Server-Sent Events (SSE)**. The client subscribes to a per-job SSE stream; the server pushes a new event after accepted task results — coalesced, at most one a second (see [Live updates](#live-updates)). SSE is one-way (server → client) and sufficient since the client never needs to send data over the live connection.

#### Job List Page

Shows all jobs with: job type, status, allocation, and a completion counter (tasks completed / total, or games completed / max for on-demand jobs).

#### Job Detail Page — Common Elements (all job types)

- Job metadata: type, status, config summary, created by, created at.
- Completion progress.
- **Per-worker contribution table**: worker identity (username, or an anonymous worker's pseudonym — never its UUID, which is its credential), tasks completed for this job. Sorted by tasks completed descending.

#### Job Detail Page — By Job Type

**Games / Game pairs**

- SPRT status text: one of `running`, `passed (H1 accepted)`, `failed (H0 accepted)`, or `terminated at max games`.
- The pentanomial (game pairs only): the five pair outcomes the LLR is computed from. Ratings are not here — they are pool-scoped and live on the [ratings page](#the-ratings-page).
- Running result counts and percentages: wins / losses / draws for player 1.

**Opening rack analysis**

- Progress: racks analyzed against the size of the rack space, and nothing else.
- Search input: enter a rack string to look up its analysis. Returns the full ranked move list (all N plays that were evaluated) for that rack, sourced from `position_analysis_moves`.

  The panel used to carry the average best equity and a breakdown of what the
  best opening play was — placement, exchange or pass — and both are gone.
  They aggregated over every stored move row of the job, which made them the
  most expensive read in the payload and one that grew without bound; and they
  counted per *claim* where `racks_analyzed` counts per task, so at
  `redundancy > 1` the move-type total was twice the rack count displayed beside
  it. Nothing is lost from storage: every ranked move is still there,
  `GET /api/jobs/:id/results` still returns the best move, score and equity per
  rack, `?rack=` still returns a rack's full ranked list, and an
  [admin export](#exports) is the path for analysing the corpus properly.
  Summarising millions of racks in two numbers on a progress panel was not
  where that analysis belonged.

**Leave generation**

- Current generation number and the configured per-generation minimum rack target (e.g., "Generation 3 — target: 500 occurrences per rack").
- **Live**, on every accepted result: tasks completed and games played in the in-progress generation.
- **As of the last merge**, and labelled with its time: racks at target out of the generation's total, and the rack with the fewest occurrences with its count. Accepted results are staged and merged into the per-rack totals in batches — every half hour, and about once a minute as a generation nears its end — so these lag by that much; see [What a merge costs](#what-a-merge-costs). Neither comes from a worker's heartbeat.

---

#### The stats payload

One function computes job statistics, and both `GET /api/jobs/:id` and the SSE
push use it, so a live update is byte-for-byte what a page reload would produce.

```
JobStats {
  job:               { id, job_type, status, allocation, redundancy,
                       min_magpie_version, created_at, created_by, lexicon, variant }
  tasks_total, tasks_completed, tasks_available, tasks_claimed, results_accepted
  games?:            { unit: "game" | "pair", wins, losses, draws,
                       units_completed, pentanomial?, divergent_pairs?, min_units, max_units,
                       win_pct, loss_pct, draw_pct,
                       sprt: { llr, lower_bound, upper_bound, status } }
  opening_racks?:    { racks_analyzed, racks_total }
  leave_generation?: { current_generation, generation_count, target_rack_count,
                       tasks_completed, games_played,            // live
                       racks_at_target, racks_total, min_rack, min_rack_count,
                       progress_as_of }                          // as of the last merge
  workers:           [ { user_id, anon_id, username, tasks_completed } ]
  other_workers:     number
  eta_seconds?:      number
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
maintained in the submit transaction once per task, on its first accepted result
— the same row the aggregates they replace selected, since redundant claims
replay the same deterministic work. `tasks_total` rides on the claim's existing
`UPDATE jobs`, and `tasks_completed` on the moment a task actually *reaches*
completed, which the submit path's own update already computes.

They exist because the reads did not scale: the job list re-derived per-task
game totals for every job on every page view (2.2 s at the test volume) and
counted each job's tasks twice more besides, and counting distinct analysed
racks cost seconds at a million racks on every detail view and every live push.
Nothing that *decides* anything reads them: SPRT still reads `game_results`, so
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

**ETA** is extrapolated from claims completed in the last hour. It is `null` for
an inactive job and `null` when nothing completed in that window — there is
nothing to extrapolate from, and a fabricated number is worse than a blank. For
SPRT jobs the remaining work is measured in units against `max_units`, converted
into tasks using the observed units-per-completed-task ratio; for everything else
it is remaining tasks. A job already past its cap reports 0.

#### Live updates

Each job with at least one dashboard subscriber gets a broadcast channel, created
on first subscribe and dropped when the last receiver goes away, so an idle
server holds no per-job state. The submission path checks for a subscriber before
building a payload at all. `GET /api/jobs/:id/stream` sends the current stats
immediately as its first event, then an event per accepted result, all named
`stats`, with a 15-second keep-alive so an idle connection survives an
intermediary's timeout.

**An event per result, not one per result.** Building a payload is several
aggregates over the job's history, so it happens off the submitting request and
one at a time per job: a submission that finds a build already running marks it
to repeat rather than starting a second. A burst of submissions therefore
collapses into the one payload that follows it, which is both fewer reads and
strictly fresher data than a queue of payloads would deliver. What the page
loses is a guaranteed event per result, which it was not counting: every event
carries the whole payload rather than a delta, so a merged one says everything
the ones it replaced would have. Rounds are also spaced at least a second apart:
on a busy job another round is always pending, and without the gap the payload
was rebuilt back to back for as long as a dashboard stayed open.

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
on-demand SPRT jobs the list also reports `units_completed` against `max_units`,
because a task count that grows as work is handed out is not a meaningful
denominator.

#### Exports

A completed job's whole corpus is read **once**, not once per caller.

The results stream scans from a cursor and holds a pool connection for as long
as its caller keeps reading. That is fine for a spot check and wrong for a
corpus: a full English opening-rack job is tens of millions of rows, and one
caller per scan is one connection per scan against a pool of twenty. So bulk
reads are admin-only, at most **two streams run at once** (a semaphore permit
held for the life of the response body, released when a caller disconnects as
well as when one reads to the end), and a completed job is served from an
artifact instead.

`POST /api/admin/jobs/:id/export` spawns a task that streams the job's rows out
as gzipped NDJSON straight into an S3 multipart upload — nothing larger than one
8 MiB part is ever resident, which is what lets it run against a job whose
results do not fit in memory. The row is written before the task starts, polled
through `GET`, and answered with a presigned URL once ready, so **the bytes never
pass through the backend** and never touch the connection pool the cap exists to
protect.

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
gunzip-untar-hash of a whole tarball; every Argon2 hash and verify; each
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
queries, so the two are the same corpus.

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

**Only completed jobs can be exported**, and that restriction is what makes the
artifact worth having: a completed job's results are immutable, so an export is
built once and reused by every later download, where an export of an active job
would be stale as it was written. It is also why the stream can safely redirect
to one.

**And only once its results have settled.** A job is marked completed the
moment its stopping rule is met or an admin forces it, but the claims already
out are still played and still accepted, so its results keep arriving for up to
the heartbeat timeout. An export started in that window missed them, and every
later download was redirected to it. So an export is refused (`409`) while any
claim of the job is still open; no claim can be issued against a completed job,
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
after 30 days, and they are **not** cross-region replicated — losing one costs a
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
tenth task completed twice (as under redundancy 2); 40 more paired jobs over 20
configs in one rating pool, 600,000 paired results in all; an opening-rack job of
1,000,000 analysed racks at 10 moves each, a third of a full English job; and a
leave-generation job's 3,199,724 progress rows. Warm times, best of two:

| Query | Runs | Time |
|---|---|---|
| `game_pair_stats` over 400,000 pairs | every paired submission and SSE push | 54 ms |
| `game_stats` over 400,000 games | every game submission and SSE push | 50 ms |
| `list_jobs`, 42 game jobs — **as it was**, re-deriving per-task game totals | every job-list page view | **2,188 ms** |
| `list_jobs` task counts — **as they were**, two `COUNT(*)`s over `tasks` per job | every job-list page view | linear in every listed job's task history |
| Contributor lists — **as they were**, grouping every completed claim in the database | every `/api/users` and `/api/workers` page view | 93 ms at 44,000 claims, linear from there |
| `opening_rack_stats` — **as it was**, racks analysed and average equity in one query | job detail and every SSE push | **2,086 ms** (3,296 ms at 1,000,000 racks on a later run) |
| `opening_rack_stats`: the average alone, after the split | job detail and every SSE push | 343 ms |
| `opening_rack_stats`: best-move types | job detail and every SSE push | 541 ms (322 ms on the later run) |
| `opening_rack_stats` **as it is now** — two counters, after both aggregates were dropped | job detail and every SSE push | two single-row reads |
| Rating sweep `build_matrix`, 600,000 paired results | every two minutes, and on every public read of a pool | 452 ms |
| `worker_contributions`, 44,000 claims | job detail and every SSE push | 136 ms |
| Public worker list, all claims | page view | 93 ms |
| Leave `next_step` rack selection — **as it was**, ordering on `(occurrence_count, rack)` through an index without `rack` | every leave claim, inside the dispatch lock | 225–390 ms at **400,000** racks (an eighth of English; a scan and sort of the generation, so linear from there — 2–3 s at full size). The 47 ms first recorded here was measured on counts that rarely tied |
| Leave `next_step` rack selection **as it is now** — a sweep of the primary key from a cursor while many racks are below target, lowest count first on the narrow index once few are | every leave claim | sweep: 3.9 ms at 400,000 racks with 97% of them at target (16,000 rows stepped over for 501 racks; nearer 0.1 ms early in a generation), and the same with nothing staged or ten thousand results staged; tail: 0.5 ms |
| `leave_gen_stats` — **as it was**, counting the generation's racks at target | job detail and every SSE push | 210 ms |
| `leave_gen_stats` **as it is now** — the generation's summary row | job detail and every SSE push | one single-row read |
| Transition: stream generation 1 by rack | once per generation | 674 ms |
| Materializing a generation's rack universe | once per generation | **56–66 s** (measured as a SQL copy inside the transition, which is where it used to run; it now runs from the first claim of the generation it belongs to, off the critical path) |

What the numbers settled:

- **The SPRT path still reads the rows, but not on every submission.** About
  50 ms at 400,000 units, and linear in the job's history from there. The
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
- **The two questions about recent completions read a time index.** The ETA
  (on every detail view and live push) counts a job's claims completed in the
  last hour, and the job list's `stalled` flag asks whether any completed in
  the last day. `task_claims` has no job column, so both used to walk every
  task of the job and every claim of each — the job's whole history, for a
  question about its last hour — on a job whose age is exactly what makes the
  walk long. A partial index on completed claims by time
  (`task_claims_completed_idx`) bounds both by the fleet's recent completions
  instead, whatever the job's age.
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
  `task_claims`' indexed identity columns: 1–3 ms for the same requests. The
  filtered query is sent unprepared, so it is planned for the identity asked
  about — a heavy contributor is found at the head of the job's feed index and
  a rare one through their own claims, and a cached generic plan picks one of
  those for everybody.
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
- **Copying the rack universe to the next generation is the one slow write**, and
  it is slow on an under-provisioned database: a minute here, against about 15
  seconds for a whole transition on the smaller dev database. It runs once per
  generation, detached from the request, so it costs time rather than
  correctness. The production instance class has not been measured;
  `scripts/leave-gen-bench.sh` does it in one command, and if the copy is slow
  there it can be removed rather than tuned — treat a missing row as zero
  occurrences and select a generation's racks by anti-joining the previous
  generation's rows instead of copying them.

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
| Charts | LayerCake |
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

**ECS with Fargate**: Two containers share a single ECS task definition — one running the Axum backend, one running an Nginx container serving the SvelteKit static build. The frontend will be split out to S3 + CloudFront in a later phase.

**Database migrations**: `sqlx migrate run` executes at container startup before the server accepts connections. No separate migration runner needed.

Until birdtest is deployed there is only ever **one migration**. Schema changes edit `0001_initial.sql` in place rather than adding a numbered migration, because there is no live database whose history needs preserving, and a single file is far easier to read than a schema reconstructed from a chain of diffs. The cost is that sqlx records a checksum per applied migration, so editing `0001` means any existing development database must be reset — see [Development](#development). Once there is a deployment to migrate, this convention ends and migrations become append-only.

**Observability**: Structured JSON logs via `tracing` + `tracing-subscriber` (JSON formatter). No metrics or distributed tracing for v1.

**Auth**: Sessions are PASETO **v4.local** tokens (encrypted and authenticated with a 32-byte symmetric key) in an httpOnly cookie named `birdtest_session`, default TTL 7 days. The token carries the user id, username and admin flag, but **only the user id is consumed** — username and `is_admin` are re-read from the database on every request, so a deleted or demoted account cannot keep acting on a token minted before the change.

Two different hashes, for two different threat models. **Passwords** get Argon2 with a per-user salt: they are low entropy and are only ever verified against one known row. **API keys, email confirmation codes and reset tokens** get SHA-256: they are 24–32 bytes of randomness with no low-entropy secret to protect against offline guessing, and a worker request looks a key up by exact hash match on every call — a per-key Argon2 salt would force a full table scan and a verify per row. Raw API keys are `bt_` followed by 64 hex characters.

**Secrets**: Database credentials, signing keys, and SES credentials are stored in AWS SSM Parameter Store and injected into the ECS task at runtime.

**Configuration** is read entirely from environment variables — `.env` locally,
task-definition values in ECS, with the secret ones pulled from SSM at task
start. The process has no SSM code path of its own.

| Variable | Default | Notes |
|---|---|---|
| `DATABASE_URL` | — | **Required**, unless the `DB_*` parts below are set. |
| `SESSION_SIGNING_KEY` | — | **Required.** 32 bytes, hex-encoded. Startup fails if absent or the wrong length. |
| `BIND_ADDR` | `0.0.0.0:8080` | |
| `SESSION_TTL_SECONDS` | `604800` (7 days) | |
| `SECURE_COOKIES` | `false` | `true` in any deployment served over TLS. |
| `MAIL_BACKEND` | `console` | `console` or `ses`. Anything else fails startup. |
| `MAIL_FROM` | `no-reply@birdtest.local` | |
| `PUBLIC_URL` | `http://localhost:5173` | The base for links in emails. |
| `HEARTBEAT_TIMEOUT_SECONDS` | `300` | How long a claim survives without a heartbeat. |
| `S3_BUCKET` | `birdtest-artifacts` | |
| `S3_ENDPOINT` | unset | Set to MinIO's address locally; the AWS SDK works against it unmodified. |
| `MIN_MAGPIE_VERSION` | `0.1.0` | The enforced global floor, and the default floor for a new job. Also a floor the server's own pinned MAGPIE must clear: startup fails if `MAGPIE_BIN` reports less, since the server would be publishing hashes built by a MAGPIE its workers may not run. |
| `MAGPIE_BIN` | `/usr/local/bin/magpie` | The pinned MAGPIE the server runs for every derived file and every leave-generation KLV. The backend image builds one in; locally, a checkout's `bin/magpie`. Startup fails without a working one. |
| `MAGPIE_THREADS` | `1` | Threads given to a conversion. The web task keeps 1; the derived-file builder task sets its vCPU count. |
| `MAGPIE_SCRATCH_DIR` | the system temp directory | Where a conversion's throwaway data directory goes. The builder task points it at its ephemeral volume, since a rack info table is 1.9 GB. |
| `MAGPIE_DOWNLOAD_URL` | the MAGPIE repository | Sent in a shutdown directive. |
| `MAGPIE_DATA_REPO` | `jvc56/MAGPIE-DATA` | Where import fetches tarballs from. Configuration, never user input. |
| `GITHUB_TOKEN` | unset | Optional in development, set in production: unauthenticated ref resolution is 60 calls per hour per IP. |
| `TRUSTED_PROXY_HOPS` | `0` | Reverse proxies in front of the process that append `X-Forwarded-For`. Per-IP rate limits key on the entry this many from the right; `0` keys on the TCP peer. `1` behind the ALB and behind the compose Nginx. |
| `DB_HOST`, `DB_PORT`, `DB_NAME`, `DB_USER`, `DB_PASSWORD`, `DB_SSLMODE` | unset | Read only when `DATABASE_URL` is unset, and assembled into one with the password percent-encoded — so a deployment can inject a managed password without hand-writing a URL. |

A numeric setting that does not parse, a `SECURE_COOKIES` other than `true` or
`false`, or a `MIN_MAGPIE_VERSION` that is not a version fails startup rather than
silently taking the default.

There is deliberately no `DATA_PATH`. The server reads letter distributions out
of the `input_data` row a job pins, so there is no filesystem copy to drift from
what the workers are checked against.

**Live dashboard updates**: Each job detail page subscribes to `GET /api/jobs/:id/stream` (SSE). The server pushes an event after accepted task results for that job — coalesced, and at most one a second — carrying the updated aggregate stats. The client merges the event into its local state without a full page reload. Axum supports SSE natively via `axum::response::sse`.

---

### Design Decisions & Rationale

| Decision | Rationale |
|---|---|
| Allocation-only job scheduling | One axis: every claim goes to the active job furthest behind its share. A priority tier was a second axis that expressed nothing allocation cannot — a job at 0% gets nothing, exactly as an inactive one does, and a job alone above 0% gets everything — while adding a rule (100% per tier) that had to be kept coherent with it |
| Admin-only job management | Avoids abuse prevention and quota complexity in v1 |
| Seed uniqueness for seed-based tasks | `(job_id, seed)` unique index prevents duplicate work at the DB level; uint64 seed stored as signed BIGINT, reinterpreted at the application layer |
| No JSONB in schema | All `config` and audit `metadata` are expanded into typed columns and per-job-type config tables; avoids schema-less data and keeps queries typed |
| Lazy timeout reclamation | No background process needed; simpler to operate |
| Claim token for stale result rejection | Race-condition-free; no timestamp comparison needed |
| Anonymous workers identified by UUID | Enables per-worker contribution tracking and result filtering without requiring account creation |
| No pre-aggregation for dashboard v1, except measured exceptions | Simple stats don't require it; avoids premature optimization. Measurement (Dashboard, "What these reads cost") found the reads that grew with history badly enough to matter, and only those are kept as running totals: four on `jobs` and a contribution counter per identity |
| AWS throughout | Learning goals; avoids future migration pain; production-grade from day one |
| Batch Bradley-Terry instead of incremental Elo/Glicko | Player configs have fixed strength, so there is no drift for a sequential filter to track; a batch fit is order-independent and makes add/remove a refit rather than an unwind |
| Named `player_configs` table | Reusable across jobs; maps directly to MAGPIE per-player arguments (`-r1`/`-r2`, `-s1`/`-s2`, etc.); **immutable once created** — no update endpoint exists; deletion only if no job references the config |
| Frontend dark mode only | Single theme simplifies the component library configuration; no light/dark toggle in v1 |
| Deficit-based job selection, measured from when a job joins | Deterministic; guarantees long-run allocation accuracy regardless of claim timing; no randomness means reproducible behavior. No starvation of any job above 0% — which holds because a job's deficit is measured from a baseline reset to parity on activation, allocation change and purge (`claims_baseline`), not over its lifetime: a lifetime deficit let a newly activated job take every claim until it had issued as many as the oldest job beside it |
| Seed gap of batch size | Prevents two tasks from covering overlapping game seeds; `next_seed = MAX(seed) + batch_size` so seeds tile without gaps or overlaps |
| Ratings pooled across jobs, scoped by (variant, letterdist, layout) | A rating is only comparable under fixed conditions, but it is not a property of one job; pooling is what lets a config's whole record produce one number |
| Only paired jobs feed ratings | `-gp` swaps seats on every seed, so a pair is side-balanced; unpaired games would need an explicit side-advantage term to avoid biasing every rating |
| Two finish conditions for SPRT jobs | `min_games`/`min_pairs` prevents early false-positive termination; `max_games`/`max_pairs` bounds compute cost |
| Jobs created inactive | Allocation is set at activation time, not creation, so the admin reviews the full active job set and assigns percentages as a single deliberate act |
| API keys active/inactive toggle | Lets contributors rotate or temporarily suspend a key without losing it; only active keys accepted for auth |
| Account deletion is app-layer, not CASCADE | Task counters must be decremented and tasks may revert state; a DB-level cascade cannot update denormalized counters |
| SPRT evaluated inline on the submission path, debounced | No background sweep needed; a debounced check is late, never wrong (see Statistical Result Evaluation) |
| Two containers per ECS task | Axum backend + Nginx for SvelteKit static files; cleaner than co-mingling in one process |

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
| `win_pct_model` (per player) | `"winpct"` | `strategy/winpct.csv` | 836,150 B |
| `letter_distribution` | `"english"` | `letterdistributions/english.csv` | 489 B |
| (board layout, never named) | — | `layouts/standard15.txt` | 244 B |
| `use_wordmap` | true/false | `lexica/NWL23.wmp` — built locally from the `.kwg` | ~104 MB |
| `use_rit` | true/false | `lexica/NWL23.NWL23.rit` — built locally from the `.klv2` and the `.wmp` | ~1.9 GB |

Two contributors could both honestly report running `NWL23` with `winpct` and be
running different bytes. This is not hypothetical: `download_data.sh` installs
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
- **`games` / `game_pairs`** — SPRT is a decision procedure over an aggregate. A
  minority of workers on a different lexicon biases the win rate and SPRT
  reaches a confident, wrong conclusion. Nothing about the output looks anomalous.
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
| `winpct` | each player, **only if it simulates** | `strategy/<name>.csv` | `DEFAULT_WIN_PCT "winpct"`; loaded lazily by `config_load_win_pcts` |
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
(`DATA_VERSION="20251004"`), probes for `data-<version>.tgz.aa`, walks the chunk
suffixes `aa`, `ab`, `ac`, … while they exist, concatenates them, and pipes the
result through `tar -xzf`. Today that is three 40 MB chunks, ~94 MB total. Two
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
the versioned path changes with it, and the name stays `20251004`. The only
thing that pins content is the built tarball — which is what production installs
and what import reads. The dedupe rule below is what makes this survivable: if
`20251004` is ever re-cut with different bytes, importing it again produces *new
rows*, visibly, rather than silently redefining what `20251004` meant.

### The `input_data` table

One row per distinct file — a `(path, sha256)` pair, so the same path with
different bytes is a different row, which is the entire point. `tarball_date`
records the tarball a row was **first** seen in: provenance, not membership. A
file unchanged between two tarballs stays one row labelled with the older date,
because it is the same bytes and a job pinning it is pinning those bytes
regardless of which tarball the contributor installed. The full definition is in
[Schema](#schema).

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

**Phase 1 — fetch and diff, in the background.** The archive is ~94 MB, so this
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
   allowlist before it is trusted enough to hash: **regular files only**; the
   path must be relative, carry no `..` segment, and match the expected
   `data/<dir>/<basename>` shape. **Never construct a filesystem path from an
   archive name** — the import hashes bytes and has no reason to form one.
   Anything failing aborts the whole import rather than skipping the entry,
   because a malformed archive is not a partially trustworthy one.
4. Upload every `kwg` and `klv` entry's bytes to the object store, keyed by
   digest, while the archive is still in memory — re-downloading 94 MB at
   confirmation time for files already hashed would be pure waste. The server
   builds reference wordmaps and rack info tables from these; a `winpct` file is
   neither read nor built from server-side, so it stays digest-only. An object
   already present is skipped, so a tarball whose lexica have not changed
   uploads nothing. This happens **before** anything is staged: a row that names
   an object has to be a row whose object is there, or the first derived build
   from it fails with a missing key instead of a reason.
5. Compare each `(path, sha256)` against `input_data`. Stage the result, keeping
   the bytes of every `letterdist` and `layout` entry, and the object key of
   every `kwg` and `klv` entry, so confirmation does not have to download again.
6. Mark the row `staged`, or `failed` with the reason in `error`.

**Phase 2 — confirm.** The admin sees three groups: **new** rows, **known** rows
(the majority, and the reason the diff exists), and **path collisions** — a path
already known under a different `sha256`. That last group deserves a second look,
because it is either a legitimate data update or a tarball re-cut under a name
that was already used. Confirmation inserts only the new rows, in one
transaction, with `tarball_date` set to this import's date.

**Limits, all enforced during the walk:**

| Limit | Value |
|---|---|
| Compressed bytes downloaded | 512 MiB |
| Chunks walked (`aa`, `ab`, …) | 64 |
| Total uncompressed bytes | 1 GiB |
| Uncompressed : compressed ratio | 20× |
| Single entry | 128 MiB |
| Entry count | 5,000 |
| Whole task | 30 min, 30 s connect, 120 s idle read |

Every one aborts the import rather than skipping the entry. The ratio is checked
continuously rather than at the end, since that is the zip-bomb case a total cap
alone lets through. The archive URL is built from `MAGPIE_DATA_REPO` and never
from user input, so the residual exposure is a compromised upstream; that is the
threat model these checks are written against.

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
unauthenticated ref resolution is 60 calls per hour per IP. A `403` from GitHub
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
pins — generated rather than typed. `cloned_from_id` records the lineage, because
a clone starts with no rating history and that would otherwise read as a bug: the
config page shows "cloned from simmer-NWL23-4ply@20250101 — ratings restart on
new data".

**`jobs`** carries `variant`, `letterdist_id` and `layout_id`. These were
duplicated across all four job config tables, which was three chances to disagree
and no way for a query to ask "what letter distribution is this job on" without
knowing its type first. Putting the letter distribution here rather than on the
player is not arbitrary: MAGPIE takes one `-ld` for the whole game, and two
players cannot draw from different bags. The same is true of the board.

**The per-type tables** keep only what is genuinely per-type.
`job_leave_config` is the one place a lexicon still sits on a job, because leave
generation has one bot and no `player_configs` row to hold it.

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
again. Rather than writing that twice, both paths call one `release_claim(tx,
claim_id, terminal_state)`, differing only in the state they pass — which is the
only thing that should differ. It acts only on a claim that is *still* `claimed`
and reports whether it did anything, because the counter it decrements is what the
scheduler believes about live work: releasing one claim twice would decrement
twice, and a drifting counter makes a job look saturated so dispatch quietly stops
days later, nowhere near the cause. `'declined'` is its own value on the `claim_state`
enum rather than a reuse of `'abandoned'`: the two mean different things, and
only one of them is diagnostic.

That enum addition has a sharp edge worth remembering: the unique indexes
`task_claims_user_unique_idx` and `task_claims_anon_unique_idx` are partial, and
are `WHERE state NOT IN ('abandoned', 'declined')` for a reason. Were `'declined'`
left out, a worker that declined a task would be permanently barred from
claiming it again after fixing its data.

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
so the columns are `NOT NULL` and default to **`0.1.0`**, the same value as the
server's `MIN_MAGPIE_VERSION`, which `create_job` writes explicitly.

`0.1.0` is `birdtest-contribute`'s **pre-release version**. Neither birdtest
nor the branch is in production yet, so everything the protocol relies on —
every result-changing setting stated on the request rather than taken from the
worker's build, input data and derived files checked against the hashes the job
pins, the word info table switched off before every load, a seed on every task
— is in `0.1.0`. The version was incremented through `0.5.1` as birdtest and
MAGPIE changed together during development; none of those numbers ever
shipped, so they carry no meaning a floor could use, and it is `0.1.0` until
the first release. From then on the version moves only when a release changes
what a task computes, and the floor moves with it: a floor is a minimum, not a
pin, so it exists precisely so that the first such release can raise it. The
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
`both`. When both apply, say so but **lead with the MAGPIE version**: updating
MAGPIE is the remedy that fixes both, since a release bumps `DATA_VERSION` and
the contributor runs `download_data.sh` as part of updating. Telling someone to
fix their data first sends them on a trip they would have made anyway. The global
floor short-circuits all of this: a client below `MIN_MAGPIE_VERSION` gets
`magpie_too_old` on its first claim without any job being consulted.

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
  one needs output-side checks: redundant claims compared for equality (games are
  deterministic, which the scheduler already assumes) or occasional canary tasks
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

3. **Lazy reclamation**: Before acquiring a task, any claimed tasks whose `last_heartbeat_at` (or `claimed_at`, if no heartbeat has been received yet) exceeds the heartbeat timeout are returned to `available`. One statement covers every candidate job rather than one per job: `task_claims` has no job column, so the planner reaches expired claims through the partial index on open claims — one entry per claim in flight across the fleet — and filters by job afterwards. Per job, a claim request paid that scan once per candidate for a set of rows that does not depend on the job at all.

4. **Task acquisition** — strategy-dependent:
   - **Re-dispatch first**, under the job's dispatch lock like everything else here: `SELECT ... FOR UPDATE SKIP LOCKED` on the job's `available` tasks — a lapsed claim's task, or one with redundancy left to fill — **excluding any task this worker already holds a slot on**. Redundancy means independent workers; without the exclusion, a worker holding a slot on the oldest open task is offered it again on every attempt, refused by the per-identity unique index each time, and gets no work at all.
   - **Otherwise generate**: produce the next task request for the job type and insert + claim it atomically in a single transaction.

5. **Response**: The server serializes the job-type-specific task request and returns it to the worker along with the claim token.

#### The claim loop in full

The steps above are the happy path. The whole exchange is one function returning
a four-way `ClaimOutcome`, mapped to HTTP once at the edge so the compiler
enforces that every branch is considered:

```
claim(identity, capabilities) -> Task | Idle | NoWorkExists | Shutdown
```

**Before any job is consulted**, the worker's version is checked against the
server-wide floor. A client below it cannot run any job that could ever exist, so
it is sent a `magpie_too_old` shutdown without a single job being queried.

Then, up to **three attempts**:

1. Select candidate jobs (the CTE above). Empty → decide between `Shutdown`,
   `NoWorkExists` and `Idle`.
2. For each candidate in deficit order: reclaim its expired claims, then try to
   acquire a task from it. Acquisition returns one of four things:
   - **Task** — for a worker that arrived with no identity, insert its
     `anonymous_workers` row; insert the claim, bump the task's counters and the
     job's `claims_issued`, commit, and return it with the job's
     `expected_data` from its template. The `claims_issued` update is guarded on the job still being
     `active`: a claim that selected the job before the stopping rule or an admin
     completed or deactivated it waits on that update's row lock, then finds the
     job no longer active and hands out nothing.
   - **NoWork** — this job has nothing to hand out; try the next candidate.
   - **JobFinished** — the job's space is exhausted; flip it to `completed` and
     try the next candidate. The flip is written inside the claim transaction,
     under the dispatch lock, so a purge cannot restart the job between the
     decision and the write.
   - **NeedsUniverse** — leave generation only: the current generation's rack
     universe has not been written. Roll back, start the seeding on its own task
     (it is millions of rows, and a request a client gives up on would roll it
     back), and try the next candidate.
   - **NeedsLeaveMerge** — leave generation only: nothing is left to hand out
     or in flight, but accepted results are still staged, so whether the
     generation is complete is not yet known. Roll back, start the merge on its
     own task (it gives up at once if one is already running, so a fleet asking
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
3. If no candidate produced work and nothing asked for a restart, return `Idle`.

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
way a dispatchable job's derived-file hashes are. It cannot go stale: a job's
config rows have no update path, player configs are immutable, and an
`input_data` row cannot be deleted while a job or a config pins it. A purge
changes none of it; deleting the job forgets it. Before the template, a games
claim made five reads of those rows inside the lock — the config, the
distribution, a three-way join per player and the six-table `expected_data`
union — and a re-dispatched task's request made six; now the reads under the
lock are the seed cursor, the available tasks, and the request row of a task
being reissued. The submit path uses the same template for the batch size a
task was dispatched with and the number of moves to keep per position, inside
the task's row lock, instead of reading the request row and the player config
again.

**The wait for it is bounded** (`lock_timeout`, two seconds), and a claim that
gives up treats the job as having nothing right now and tries the next
candidate. Ordinary contention is milliseconds, so this is never reached in
normal operation; it exists for the one holder that is not ordinary. Seeding a
leave generation's rack universe is millions of rows and tens of seconds, and
the task seeding it holds this lock throughout — so without a bound every
other claim for that job blocks for the duration *while holding a pool
connection*, and the pool is twenty. One slow claim on one job would stall
submissions and the dashboard for the whole server. The bound turns that into
those workers being told to look elsewhere.

**Two things legitimately restart an attempt**, and both are ordinary rather than
exceptional:

- A **lost race on `(job_id, seed)`**, when two workers generate the same
  on-demand task simultaneously. One insert wins; the loser retries and lands on
  the next seed. The dispatch lock makes this rare — it was the common case
  before that lock existed, where past three-way contention on one job a worker
  was answered `204` while work existed. A purge takes the same lock, so it is
  no longer a way around it; the retry stays because a lock that can time out is
  not a lock that always held.
- A **lost race on the per-identity partial unique index**, which rejects a second
  concurrent slot on the same task by the same worker. That is not a failure —
  this worker already holds a slot here — so it re-runs selection and lands
  somewhere else.

The three-attempt cap bounds the loop; exhausting it returns `Idle`, and the
worker simply asks again.

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

1. Look up the claim by token, requiring `state = 'claimed'`, **inside the
   submission's transaction and `FOR UPDATE`**. A token that matches nothing means
   the claim already lapsed and was reclaimed, or this result was already
   accepted: respond `200` with `{"accepted": false}` rather than an error. The
   lock is what makes that sound — looked up outside the transaction, a timeout
   could abandon the claim between the lookup and the write, leaving it both
   abandoned and completed and the task's live-claim counter decremented twice.
2. Decode and validate the payload against the job type's response shape. A
   malformed body is `400` naming what was wrong.
3. Normalize it into the record shape and insert it.
4. Mark the claim `completed`, increment `accepted_count`, decrement
   `active_claim_count`, and recompute the task's state against the job's
   `redundancy`, stamping `completed_at` when it reaches it.
5. Commit. No audit row is written: the completed claim and the stored result
   already record the submission. Ratings are deliberately **not** touched
   here: a fit is global to a rating pool and nothing in this path depends on
   it, so it runs on a periodic sweep instead.
6. **After** the commit: evaluate the finish conditions on only the aggregates
   they need — the SPRT statistics, or an opening-rack job's task counts.
   Inline, because SPRT decides whether the job keeps dispatching. The job row
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
   task issues them, at most once a second. The dashboard can therefore lag up to
   a second behind under load, which is the intended trade.

#### What a submission has to satisfy

Validation is per job type, and is the server's only defence against a
submission written straight into the largest tables in the schema. A body over
64 MiB is refused before it is parsed (`413`); batch size is the admin's lever for
staying under it.

**Games and game pairs.** `wins + losses + ties == games`, all non-negative, on
every aggregate. A `games` result must contain at least one game. A `game_pairs`
result must additionally carry a `pentanomial` whose five counts are
non-negative and agree with the game aggregate on both the pair count and player
1's half-points — the cross-check described under [MAGPIE reports the
pentanomial](#magpie-reports-the-pentanomial). `divergent_games` is optional; when
present its own counts must be consistent, its `games` even, and no larger than
the total. A plain `games` result carrying either has it ignored rather than
stored, since a job that does not play pairs has no pairs to describe.

**Captured positions**, when present:

- `game_index` must fall inside the batch the task actually dispatched. The batch
  size is the only thing that legitimately bounds this.
- `turn_number` must be within a generous per-game ceiling (400), so a malformed
  number cannot masquerade as a valid one.
- Every position must carry at least one ranked move.

**Opening racks.** At least one rack, and every rack must carry at least one
move — the moves arrive ranked best-first, so an empty list means nothing was
analysed and there is no best move to record. The submission must also name
**exactly the racks the task dispatched**, as a set; see below.

**Leave generation.** At least one rack occurrence.

**On top of all of the above, the plausibility rules** in
[`plausibility.rs`](backend/src/jobs/plausibility.rs) — finite score moments, a
non-negative standard deviation, bounded play scores and probabilities, racks of
1–7 tiles, no rack listed twice in one leave submission — reject impossibilities
rather than oddities. See [Why impossibility, and not per-worker anomaly
detection](#why-impossibility-and-not-per-worker-anomaly-detection) for the
reasoning and the full table.

Two of them cannot run in the pure validation step, because they need the
request. They run in `store_result`, where the task id is in hand, and they are
the only submission-time checks that catch a worker reporting work it did not
do:

- **A game batch must report exactly the games the task dispatched**
  (`num_games`, doubled for pairs).
- **An opening-rack batch must analyse exactly the racks the task dispatched.**
  The request names the racks rather than only how many, so the whole set is
  compared rather than its size; order is not part of the contract. The range is
  re-expanded from `rack_start`/`rack_count` against the job's pinned
  distribution, which costs what dispatching it cost. This matters in both
  directions: too few racks and the task still *completes*, leaving a hole in
  the space nothing revisits, because the job's finish condition only asks
  whether every task completed; racks from nowhere are stored as analyses of
  this job and added to `jobs.racks_analyzed`, the progress counter the
  dashboard reads.

#### How much of an analysis is stored

The server keeps the leading `num_plays_recorded` moves per position, read from
the player config that produced them — the same number that told the worker how
many to report. It is required on every player config (at least 1), so the
worker and the server can never disagree about it. `num_moves` on
the record preserves how many were actually ranked, so the discarded tail stays
visible as a count.

Per-ply rows are written only where the worker reported them, which in practice
means only for simming players; a static player produces none and the table stays
empty rather than filling with placeholders.

In-game positions are inserted with `ON CONFLICT DO NOTHING` and, when the insert
is a no-op, their moves are skipped too — another claim already recorded that
position, so its moves are already there.

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

A task claim is the message a worker sends to initiate the exchange. It carries no job-type-specific payload — the server decides the assignment. The worker's identity and auth are conveyed via request headers; the body carries only what the worker says about itself — `magpie_version` and `unsupported_jobs` — and is required (see [The Worker API Contract](#the-worker-api-contract)).

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

A task response is what the worker submits after completing a task. It is validated on receipt and then transformed into a task record for storage. Response types may differ from their corresponding request types (e.g., a single seed request may yield a batch of game results). Response and record types are shared across job types where the stored shape is identical regardless of how the task was generated — games and game pairs both submit the aggregate MAGPIE's autoplay reports for a batch (`{games, wins, losses, ties, score means and standard deviations}`), with game pairs adding the pentanomial over every completed pair and a second aggregate over the divergent ones. Autoplay does not emit individual games, and nothing downstream needs them: SPRT and the dashboard both work off counts.

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

The unranking order must stay stable: results are recorded against racks expanded from an index, so changing it would silently re-point existing results.

At claim time (all in one transaction):
1. Compute the next start: `SELECT COALESCE(MAX(seed) + $racks_per_batch, 0) FROM tasks WHERE job_id = $job_id`. If it has reached `total_racks`, the job has no work left.
2. Unrank that range into racks.
3. `INSERT INTO tasks (job_id, seed, state) VALUES ($job_id, $next_start, 'available')` — the range's start is the task's seed: rack `i` of the batch is analysed from `seed + i`, its index in the job's rack space, which is the same number on every worker.
4. `INSERT INTO opening_rack_requests (task_id, variant, letter_distribution, board_layout, rack_start, rack_count, player_config_id)` — the range, not the racks. There is no lexicon column: the player config carries it.
5. Return the expanded racks, the seed, and a claim token.

#### Games — On-demand

Each task represents one batch of games (`games_per_batch` from the job config) played starting at a given seed. MAGPIE uses seeds S, S+1, …, S+N−1 for a batch starting at seed S with batch size N. To prevent two tasks from overlapping on the same game seeds, consecutive task seeds are spaced `games_per_batch` apart.

At claim time (all in one transaction):
1. Compute next seed: `SELECT COALESCE(MAX(seed) + $games_per_batch, 1) FROM tasks WHERE job_id = $job_id`. This yields seed 1 for the first task, then `1 + games_per_batch`, `1 + 2*games_per_batch`, etc. The insert in step 2 will conflict on the unique seed index if two workers race; the loser retries.
2. `INSERT INTO tasks (job_id, seed, state) VALUES ($job_id, $next_seed, 'available') RETURNING id`; the claim in step 4 moves it to `claimed` in the same transaction, through the counter update every claim uses.
3. `INSERT INTO game_requests (task_id, variant, letter_distribution, board_layout, seed, num_games, capture_positions, player1_config_id, player2_config_id)` — denormalize the job's settings so the worker receives a self-contained request. Each player config carries its own lexicon.

   The seed crosses the wire to the worker as a **decimal string**, not a JSON number. It is a full `uint64`, and JSON numbers are doubles, so any client using a conventional JSON library would silently lose precision above 2^53. It is stored as a signed `BIGINT` and reinterpreted at the application layer as before.
4. `INSERT INTO task_claims (task_id, claim_token, state, claimed_by_...)`.
5. Return the request + claim token to the worker.

SPRT and finish-condition checks run during result submission, not at claim time.

---

#### Game Pairs — On-demand

Same as games, except the batch size is `pairs_per_batch` from the job config. Each task seed is spaced `pairs_per_batch` apart: `SELECT COALESCE(MAX(seed) + $pairs_per_batch, 1) FROM tasks WHERE job_id = $job_id`. The job type sets MAGPIE's `-gp` flag, so both orderings of each seed are played in a single invocation.

Results are a `GameResultsResponse` — the same type games use — carrying the aggregate over every game played, the **pentanomial** over every completed pair, and the divergent subset. SPRT runs on the pentanomial: the pair is the independent unit (the two games share a seed), and it is also the unit `min_pairs` and `max_pairs` bound, so the sample size and the progress count are the same number. The divergent aggregate is stored and displayed as a diagnostic of how often the two configs differ, and nothing is tested on it — see [The pentanomial, and why pairs are the unit](#the-pentanomial-and-why-pairs-are-the-unit).

---

#### Leave Generation — On-demand, partitioned generations

Leave generation has sequential phases: generation N must complete before generation N+1 begins. Within a generation, work is **partitioned across many parallel workers**: each task forces a different subset of the racks that still need occurrences for the current generation and plays a bounded batch of games. A generation is not "one worker, one task" — it's many small tasks that collectively drive every rack up to the configured per-generation occurrence target.

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
120-second request timeout. SSE streams are unaffected either way, since they send
keep-alives every 15 seconds.

At claim time:
1. Determine the current generation: the lowest generation number that hasn't been marked complete. If none exists and `configured_generation_count` generations are already done, return "no work."
2. Check that the generation's rack universe exists — every generation's, the first included, is written by a task the first claim to find it missing starts. That check is one indexed `EXISTS`. A claim that finds the universe missing rolls back, starts the seeding on its own task, and treats the job as having no work yet: the seeding takes the job's lock without waiting and holds it while it writes, so no claim reads a half-written universe, and a seeding a client or a deploy interrupts rolls back whole and is started again by the next claim. (It used to run inside the claim itself, where MAGPIE's 120-second request timeout could cancel it — on a database slower than that at writing 3.2 million rows, every claim restarted it and none finished.) Then select up to `racks_per_task` racks below `target_rack_count`, in one of two ways (`leave_gen::next_step`), chosen from the generation's summary row — how many racks were below target as of the last merge, the same age as the counts both selections read.

   **While many racks are below target — more than a hundred tasks' worth (`SWEEP_WHILE_TASKS_REMAIN`) — a sweep.** The generation's racks are handed out in primary-key order from a cursor remembered between claims (`leave_selection_cursors`, one row per generation, read and written only under the job's lock), one *lap* over the universe at a time, skipping racks already at target. A lap **starts only with no claim of the generation in flight and nothing staged**. From there every rack that is out — forced by an open claim, or by a result not yet merged — was handed out during this lap and so lies behind the cursor, and nothing ahead of it is out: a claim needs no list of what is out, and selection costs the same with one result staged as with ten thousand. (A task whose claim lapsed is reissued as it stands, before anything new is selected, so its racks stay behind the cursor with it.) Each selection reads one rack more than a task holds, so the task that takes a lap's last racks knows it and deletes the cursor in its own transaction; after that the job hands out nothing until the lap's last results are in and merged, and the next lap selects on exact counts. That pause is one task's duration and one merge per lap — some 6,400 tasks for English — and it is the wait that already precedes closing a generation, which is simply a lap that starts and finds nothing below target. A cursor lost to a purge or a partial restore is a lap not started: the same rule applies and nothing is handed out twice. A claim that hands out nothing commits rather than rolls back, so a lap found finished stays found.

   This replaced lowest-count-first selection for the bulk of a generation because of what that costs between merges: `leave_rack_progress` shows a finished task's racks at the counts they had before it played, so they are the *lowest* in the generation the moment their claim completes. They have to be held out — or they are handed straight back out, to every claim until the next merge — and holding them out meant every claim hashing and stepping over every staged rack, about a microsecond each, inside the dispatch lock: 160–290 ms with 400 results staged, a second at a hundred workers, where the lock's other claimants give up after two.

   **Once few remain, lowest count first**: `ORDER BY occurrence_count ASC` — on the count *alone*, ties falling in whatever order the index holds them (see [What a merge costs](#what-a-merge-costs) for why not by rack) — excluding the racks named in the `forced_racks` of an open claim for this generation, so concurrent claims are not handed overlapping subsets, **and the racks forced by a task whose result is staged but not yet merged**, for the reason above. A sweep would spend the end of a generation stepping over racks already at target; here the set below target is small by construction, so the excluded set is too. Going from the first selection to the second is safe at any moment, because the second excludes everything that is out; nothing goes the other way, since counts only grow.

   Either way, if no rack is left to hand out *and* no `task_claims` row for this generation is still `claimed`: with results still staged, whether the generation is complete is **not yet known** — the claim starts a merge, off the request, and is told there is nothing here right now (`NeedsLeaveMerge`), and the next claim decides on exact figures; with nothing staged, the generation is complete — run generation transition (below) instead of dispatching a task. Claims in flight are read *before* what is staged, and the order matters: submissions are not serialized with claims, and read that way round a result is always one or the other. A claim that *is* handed racks, but fewer than `racks_per_task`, has found the generation (or the lap) nearly done, and asks for a merge too, at most once a minute per job: that is when stale counts cost most, since tasks go out forcing racks that may already be at target and the generation cannot close until a merge shows that they are.

   **The whole of step 2 runs under a per-job advisory lock** (`pg_advisory_xact_lock`, taken before anything is read and released when the claim transaction ends). Without it every read here is made against a view of the job that a concurrent claim may be in the middle of changing, and two races follow: a claim still being issued is not yet visible as in flight, so a generation could be closed while a task for it was going out — work that lands in a generation whose KLV is already built — and two claims could both find the generation complete and both start its transition, each streaming millions of rows and uploading a KLV. The lock is per job, so claims for other jobs never wait on it, and it is *not* held across the transition itself: a transaction held open across an S3 upload is what step 2 of the transition exists to avoid.

   **Reopened tasks are reissued here, not before.** For every other job type a task whose claim timed out is re-dispatched before anything new is generated. For leave generation that happens only after the lock is taken and the current generation determined, and only for a task whose `leave_requests.generation` is that generation *and* only while no transition for that generation is running; step 2 runs when there is none. The transition check is separate from the generation check and both are needed: a generation does not read as *closed* until its transition commits the artifact row, so throughout the tens of seconds a transition takes, the current generation is still the closing one and a reopened task of it would otherwise be handed straight back out — its occurrences folded into the very rows the transition is streaming, leaving the uploaded KLV irreproducible from the database. A transition past the takeover timeout does not count, or a job whose transition process died would stall forever instead of being taken over. Reissued before the lock, a claim would be invisible to the in-flight check exactly as a new one was. Reissued for any generation, a task from a generation that has since closed would be handed out again, played with an outdated KLV, and its result discarded. A task left over from a closed generation stays `available` and is never dispatched again, so the job list's task counts for a leave job can show a few such tasks as never completed.
3. Draw the task's seed and `INSERT INTO tasks (job_id, seed, state) VALUES ($job_id, $seed, 'available') RETURNING id`, claimed in the same transaction.
4. `INSERT INTO leave_requests (task_id, lexicon, variant, letter_distribution, board_layout, generation, seed, forced_racks, num_games, previous_artifact_key, use_wordmap)` — `seed` is drawn fresh for the task, and stored so a reissued task replays it; `forced_racks` is the chosen rack subset (see Schema); `previous_artifact_key` is the prior generation's combined KLV, which for generation 1 is the server-built zeroed KLV stored at generation 0, so it is never NULL.
5. `INSERT INTO task_claims (...)`.
6. Return the request and claim token.

**Worker behaviour**: The worker downloads the previous generation's combined leave file through `GET /api/worker/artifact` — including generation 1, which fetches the server-built *zeroed* KLV stored as generation 0, so there is no first-generation branch and no fallback to the lexicon's shipped leaves. It seeds the run from the request's `seed`, hands the request's `forced_racks` to `leavegen` as an **in-memory rack list**, plays `num_games` games, and reads the rack-equity table out of `RackList`, submitting it as an inline `{rack, count, mean}` list in the `LeaveResponse`.

Neither the forced racks nor the results touch the filesystem: they arrive in the task's JSON request and go back in its JSON response. (There is no `-forceracksfile` / `-writerackequitycsv` round trip through scratch files — see [Leave generation on the client](#leave-generation-on-the-client).)

The reported list covers **every rack that occurred during the batch, forced or not**, since racks the games happen to draw naturally also count toward that rack's occurrence target. It therefore scales with distinct racks drawn per batch — potentially thousands of rows, not just `racks_per_task` — but that is still an ordinary-sized POST body (tens to low hundreds of KB), not something warranting object storage.

`num_games` is the only thing that ends the task. The generation's rack target is deliberately **not** sent: the server owns the running totals across every task in the generation, no single task can observe whether the target has been reached globally, and stopping early at the forced racks' own target would discard coverage the server would have folded in anyway.

**On result acceptance**: every reported rack must be a full 7-tile rack (plausibility refuses anything else). A result for a generation that has already been aggregated is credited to the worker and *not* folded in: its KLV is built and uploaded, so the occurrences would change nothing anyone reads, and adding them would leave the rows disagreeing with the artifact built from them — which is the one signal reserved for a corrupted or stale object (see [Artifacts: back up, or rebuild?](#artifacts-back-up-or-rebuild)). The claim flow no longer produces that state: a generation closes only when none of its claims is still `claimed`, a timed-out claim is abandoned and its late submission refused before it reaches this point, and a closed generation's tasks are never reissued (step 2). The check stays as a guard against state the flow never writes, such as a partial restore. Otherwise, within the same transaction that accepts the task result, the submission is **staged**: one row in `leave_rack_staging` carrying its racks, counts and equity sums as three parallel arrays (which Postgres compresses and stores out of line), and a single-row bump of the generation's live counters in `leave_generation_progress` (tasks completed, games played). It touches no per-rack row. A **merge** (`leave_gen::merge_staged`) later takes everything staged for the generation and applies its sum in one statement — `DELETE … RETURNING` feeding an `UPDATE … FROM` over the aggregated racks, so a staged result is either still staged or folded in, never both and never neither — and refreshes the generation's summary (racks at target, the rack furthest from it) while the rows are warm. An update rather than an upsert: the universe is seeded, so a rack with no row is not a rack of this distribution and must not create one. Merges of one job serialize on an advisory lock, since two of them would update overlapping racks in whatever order their plans visited them; the transition *waits* for that lock and everything else gives up if it is held, because whoever holds it is doing the same work. Why it is built this way, and what it costs, is under [What a merge costs](#what-a-merge-costs).

**Generation transition (aggregation)**: once claim-time step 2 finds no rack below target, no claim in flight and nothing staged, the server first **drains** — a merge that waits for any merge already running, so the totals it is about to read hold every accepted result; nothing new can be staged meanwhile, since the generation closes only with no claim in flight and nothing is dispatched for it while its transition runs — and then streams that generation's full-rack results into a CSV, runs `magpie convert rackequity2klv` over it to build the generation's KLV artifact, uploads it to S3, records it in `leave_generation_artifacts` along with the builder that wrote it, and marks the generation complete.

The transition **does not write the next generation's rack universe.** That is millions of rows (3.2 million for English); inside the closing transaction it made every worker on the job wait the write out, and made the close and the copy stand or fall together, so anything that failed cost a full re-derive and re-upload as well. The universe is seeded when its generation *opens* instead — on a task of its own, started by the first claim that finds it missing, under the job's lock so two seedings cannot both do it — from the pinned letter distribution, through the same `seed_generation` for every generation, the first included. One implementation of what a generation's universe *is*, derived from the source of truth rather than from the previous generation's rows, and off the critical path.

The transition takes tens of seconds and runs *outside* the claim transaction, on its own task so that a worker or proxy giving up on the request cannot cancel it part-way. That leaves the deciding claim holding no lock while it works, so ownership is recorded instead: the claim transaction that finds the generation complete inserts `leave_generation_transitions (job_id, generation)` and **commits** — its only write is that row, and committing is both what makes the row visible to everyone else and what releases the job's advisory lock before the upload starts. The row's primary key is what means every other claim arriving meanwhile is told there is no work yet rather than starting the same transition again. `completed_at` is set in the same transaction as the artifact row, and setting it is **conditional on the row still being there and still open** — that is how a transition finds out it no longer owns anything. A purge deletes the transitions row along with the artifacts and progress rows, so a transition spawned before it would otherwise hand the purged job a generation-1 KLV derived from results it no longer has. When the close is refused nothing is written and the uploaded object is left behind; it is keyed by job and generation, so a later transition of the same generation overwrites it, and `GET /api/worker/artifact` serves no key that no `leave_generation_artifacts` row names. A transition that never finishes — the process died, or the object store refused the upload — is taken over by a later claim once `started_at` is older than the takeover timeout (30 minutes, far longer than any measured transition), and `attempts` records that it happened; a failure the server survives hands ownership back immediately instead of waiting out the timeout. So does a **restart**: a transition runs on a spawned task, the service is a single instance whose old task stops before the new one starts, and so at startup every open transition belongs to a process that is gone — exactly as every `running` import and export does. Startup backdates them (`leave_gen::release_orphaned_transitions`) and the next claim takes over; without that, every deployment that landed inside a transition left its job with nothing to hand out for half an hour.

The derivation is MAGPIE's own `rack_list_write_to_klv`. Each full rack `R` has a mean `m(R)` — `equity_sum / occurrence_count`, or 0 if it never occurred — and a weight, the ways to draw it from a full bag (the product over letters of `C(dist, R)`). The average is the weighted mean of `m(R)` over every full rack. Every proper, non-empty sub-multiset `L` of `R` receives `m(R)` weighted by the ways to draw the rest of `R` once `L` is held (the product of `C(dist − L, R − L)`), and a leave's value is its weighted mean minus the average, or 0 if nothing contributed.

**The server used to do this itself**, in a Rust translation of that function (`jobs/klv.rs`, 868 lines). It was a faithful translation, cross-validated against a real MAGPIE binary — but it had to change whenever MAGPIE's did and nothing made it, and its KLVs differed in bytes from MAGPIE's for the same leave values, because it built a plain trie where MAGPIE builds a minimized DAWG. Both are correct and they load to the same values; having two of them was a standing source of confusion, and the second one was the one nobody would notice going stale.

So the transfer is a CSV instead: `generation_klv` streams the generation's roughly 3.2 million `rack,count,equity_sum` rows into a scratch directory, `convert rackequity2klv` reads them into a `RackList` and writes the KLV, and the server reads the bytes back and uploads them. Neither side holds a generation in memory. Before `klv.rs` was deleted, the two were run against each other over the whole 149-rack test distribution and agreed on every leave value.

`rackequity2klv` is new on the MAGPIE side, along with a `RackList` setter that takes a rack's count and mean outright: `rack_list_add_rack` folds one game's equity at a time, which is what a `leavegen` run has and not what a whole generation's aggregate is. Every full rack must appear exactly once in the CSV — a rack the file omits would contribute a mean of zero at full weight to every leave it contains, which is a real leave value and indistinguishable from a measured one — so MAGPIE marks every rack unset before reading and refuses a file that leaves any of them that way.

The generation-0 zeroed KLV is `magpie createdata klv`, which builds exactly that from the letter distribution alone. MAGPIE_DEPENDENCY.md proposed a `convert zero2klv` for it; `createdata klv` already is it, through the same `klv_create_empty`, and one spelling is better than two.


**Dashboard progress**: two kinds of figure, and the page says which is which. *Live*, on every accepted result through the per-job SSE stream: tasks completed and games played in the in-progress generation, from the counters the submit transaction bumps. *As of the last merge*, shown with its time: racks at target and the rack with the fewest occurrences, from the summary each merge writes. Both are one row read (`leave_generation_progress`); counted from `leave_rack_progress` on read, as they were, the rack figures were a pass over 3.2 million rows on every detail view and every live push (210 ms measured). No heartbeat payload is needed for either. An admin who wants the rack figures current can merge on demand (`POST /api/admin/jobs/:id/merge-progress`, "Merge progress now" on the admin job page).

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

What is left unsolved is that a merge still rewrites most of a 432 MB relation as non-HOT updates. Removing that means taking `occurrence_count` out of the index selection uses — for instance selecting "any rack below target" through a partial index on a flag the merge maintains, rather than "the racks furthest below" — which changes the selection policy, and has not been decided.

### Position Capture From Games

**Status: implemented.** Verified end to end: MAGPIE captured 98 positions across
4 real games, with the CGP evolving turn by turn, and the redundancy
deduplication held under `redundancy = 2`.

A worker playing a game already analyzes a position on every turn: it generates
candidate moves, ranks them, and picks one. Those analyses used to be discarded.
With `capture_positions` set on a `games` or `game_pairs` job they are kept, so a
job run to settle an Elo question also produces a corpus of analyzed positions.

#### What is capturable, and what it costs

This is the constraint that shapes everything else, and it splits by player type.

**A simming player's analysis is free.** `autoplay_worker->move_lists[player]` is
sized by that player's `num_plays` and, at the moment a move is chosen, holds
exactly that many candidates ranked by simulation. The work is already done and
thrown away; capturing it costs only serialization.

**A static player's analysis does not exist yet.** Static play calls
`get_top_move_for_player_on_turn`, which forces `MOVE_RECORD_BEST`, so the move
list ends up holding one entry. Capturing a *ranked list* from a static player
means relaxing that override, which makes every turn of every game record and sort
moves it currently discards — a real slowdown on the job's primary purpose.

| Player | Capture cost | What you get |
|---|---|---|
| Simming | Serialization only | The simulated ranking the player actually used |
| Static | Slower move generation on every turn | A ranked list the player did not need |

The feature is most defensible for simming players; for static players it is
possible but clearly marked as slowing the job down. Relaxing the
`MOVE_RECORD_BEST` override is the one phase not yet done.

The asymmetry is directly visible in the stored data. A `games` job pairing a
simming player against a static one, at `num_plays_recorded` of 6:

| Turn | Player | Ranked | Stored |
|---|---|---|---|
| 0 | simming | 6 | 6 |
| 1 | static | 1 | 1 |
| 2 | simming | 6 | 6 |
| 3 | static | 1 | 1 |

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

Turning capture on roughly doubles the storage a job produces per unit of Elo
information, and does so in the largest table in the schema. That is why it is off
by default.

**`games_per_batch` becomes the memory and payload control.** With no per-task cap,
submission size is a direct function of batch size: a batch of 20 games is about
450 positions and a few hundred KB of JSON, while a batch of 1,000 games is 22,500
positions and on the order of 15 MB. That is the existing knob rather than a new
one.

There is deliberately no sample rate, no turn limit and no per-task cap. Capture is
all-or-nothing per job, which removes the need for sampling to be deterministic and
removes the failure mode where redundant claims sample different positions and
defeat the deduplication below.

#### Redundancy would multiply the corpus, and the fix is cheap

Games are seeded and deterministic, so with `redundancy > 1` every worker on a task
plays *identical* games and captures *identical* positions — X copies of the same
analysis.

Keying captured positions on `(task_id, game_index, turn_number)` rather than on
the claim, with `ON CONFLICT DO NOTHING`, makes the first accepted claim the one
that lands and the rest no-ops. Redundancy keeps doing its job for the *result* —
each claim's aggregate is still stored separately, so agreement between workers
can be checked later — without multiplying the corpus.
Opening racks keep their per-claim key, so redundant analyses of the same rack can
still be compared.

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

How many ranked plays come back per position is player 1's config's
`num_plays_recorded`, for both players' positions, not a job setting — MAGPIE has
one cap for the whole run, and the server truncates with the same number. It
pairs with `num_plays`, which is how many the player simulates; note that with
capture on, MAGPIE raises each simming player's `num_plays` to at least that cap,
so job creation refuses a capture job whose simmers would be raised: each must
already consider at least that many. Per-ply statistics pair the same way:
`num_plies_recorded` against `plies`.

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
  int pair_game_number;       // 0 or 1 within a pair; 0 when not pairing
  int turn_number;            // turn within the game
```

All are available at the call site: the candidates are
`autoplay_worker->move_lists[player_on_turn_index]`, and `game_runner` already
tracks the rest. Widening `autoplay_results_add_move`'s signature is the bulk of
the change, and it touches the three existing `add_move` recorders only insofar as
they ignore the new fields.

**The `positions` recorder** is `AUTOPLAY_RECORDER_TYPE_POSITION` alongside the
existing enum values, registered through `autoplay_results_set_recorder()` with the
same seven function pointers the others use, and selectable through the options
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
and **`MOVE_RECORD_BEST` leaves one entry**, so for a static player the recorder
captures a one-move "ranking" — correct behaviour, not a bug, but it means capture
on a static-player job produces much less than it looks like it should.

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

**Emitting it.** The existing `str_func` produces the `-hr false` summary lines the
client parses for game results; positions are far too large for that shape, and the
client reads results in-process anyway, so the recorder exposes a typed accessor
instead:

```c
typedef struct AutoplayPosition {
  char *cgp;
  char *rack;
  int game_number;
  int pair_game_number;
  int turn_number;
  int num_moves;               // how many were ranked, before truncation
  AutoplayPositionMove *moves; // num_plays_recorded of them
  int num_stored_moves;
} AutoplayPosition;

// Borrowed, owned by the recorder; valid until the next autoplay run.
const AutoplayPosition *autoplay_results_get_positions(
    const AutoplayResults *results, int *count);
```

This mirrors `autoplay_results_get_game_summary()`, added for exactly this reason —
so the client reads results out of the structs instead of parsing formatted output.
`config_contribute_games` serializes the array into the `positions` field of its
submission, and sets the recorder option when the task request asks for it, so
`capture_positions` on the birdtest job becomes `autoplay games,positions` on the
MAGPIE invocation.

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
      "num_moves": 412,
      "moves": [ { "move": "8D RETAINS", "score": 74, "equity": 81.2,
                   "win_percentage": 62.1, "blended_utility": 0.64 } ] }
  ]
}
```

`previous_move` / `previous_move_score` are absent on turn 0 of a game, where
nothing preceded it. `blended_utility` — the win%+spread blend, sometimes used to
rank moves instead of equity or raw win percentage — has the same nullability as
`win_percentage`: present only for a simming player. The whole array is absent when
capture is off, which keeps every existing client valid.

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
concurrent claimers, and a decline for each reason. Pointed at a real server,
every result it invents would be recorded as a genuine contribution, so it is
never run against anything but a disposable test stack.

birdtest's backend runs a pinned MAGPIE, built into its image from a recorded
commit. It has two jobs, and the second is the reason the first became worth
doing (see [MAGPIE_DEPENDENCY.md](MAGPIE_DEPENDENCY.md)):

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
or any other platform test. This invariant currently holds exactly — grepping the
tree for those outside `src/compat/` returns nothing — and the client work is the
largest new source of platform behaviour MAGPIE has taken on, so it must not be
what breaks it. One new piece of platform behaviour is needed, and it goes in
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
Networking is meaningless there, so the compat layer defines `MAGPIE_NO_NETWORK`
for wasm and `chttp_request()` becomes a stub that pushes an error onto the
stack. Nothing above compat needs to know.

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

**`dlopen` rather than link-time binding.** Only seven symbols are needed
(`curl_easy_init`, `_setopt`, `_perform`, `_getinfo`, `_cleanup`,
`curl_slist_append`, `curl_slist_free_all`). Resolving them at first use means a
machine without libcurl still runs every offline MAGPIE command, and `contribute`
fails with "libcurl not found; install libcurl4" rather than MAGPIE refusing to
start at all. Try `libcurl.so.4`, then `libcurl.so`, then `libcurl.4.dylib`.

Requirements that hold on every backend: **TLS certificate verification is on and
cannot be disabled** — no flag, no environment variable; redirects followed,
bounded at 5; `timeout_seconds` covers the whole exchange; the response body is
length-delimited rather than NUL-delimited, since artifacts are binary; and no
global process state at exit, because `contribute` may run many requests.

`src/util/http_client.c` holds everything not platform-specific: building the
header list, the `Authorization` / `X-Worker-UUID` choice, the JSON content type,
and the retry policy, applied uniformly:

| Response | Action |
|---|---|
| 2xx | Return it. |
| 204 | Return it; the caller decides (for `/task` it means "no work"). |
| 429 | Sleep `Retry-After` (default 1s) and retry, up to 5 times. |
| 5xx, or a transport error | Exponential backoff 1s, 2s, 4s, 8s, 16s; then fail. |
| 4xx other than 429 | Return it; the caller decides. Never retried. |

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
`key value`. Blank lines and lines beginning with `#` are ignored.

```
# birdtest contribution settings
server    https://birdtest.example
apikey    bt_9f2c...
threads   7
maxtasks  0
idlewait  5
uuid      6f3d7198-178a-47c8-9ccc-6aa6995a5a9c
```

| Key | Required | Default | Meaning |
|---|---|---|---|
| `server` | **yes** | — | birdtest base URL |
| `apikey` | no | absent | Attributes work to an account. Without it the worker is anonymous, identified by `uuid`. |
| `threads` | no | cores − 1 | Threads given to MAGPIE while working |
| `maxtasks` | no | `0` | Tasks to complete before stopping; `0` runs until stopped |
| `idlewait` | no | `5` | Seconds to wait after the server reports no work |
| `uuid` | no | assigned by the server | The anonymous worker identity |

An unknown key is an error rather than a silent ignore — a typo'd `apikey` should
not quietly downgrade someone to anonymous. If `server` is missing, `contribute`
fails with a message naming the file and the missing key, not a usage string,
since the fix is editing a file.

The file is **user-authored and MAGPIE does not rewrite it**, with exactly one
exception: once the server assigns a `uuid`, MAGPIE **appends a single line**.
Appending rather than rewriting means comments, ordering and formatting the
contributor put there survive untouched.

#### The worker UUID

**The server mints it, not the client.** A worker with no `apikey` and no `uuid`
yet sends no identity at all on its first request; the server responds with a
UUID in the body of the first successful `/api/worker/task` claim — once there is
actually a task to hand out — and the client persists it and sends it as
`X-Worker-UUID` from then on, for the rest of this run and every run to come.

This is a deliberate reversal from letting the client generate its own UUID: a
client-generated identity trusts a value the server never gets to validate.
Having the server mint it costs one extra round trip for a brand-new anonymous
worker's first task and nothing after that, and means `contribute` needs no
cryptographically secure random source at all.

Because the file is resolved relative to the working directory, a contributor who
runs MAGPIE from a different directory has no `contribute.txt` there and
`contribute` stops with a clear error — a better failure than silently becoming a
new anonymous worker and losing their contribution history.

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

Implemented as `impl_contribute(Config *config, ErrorStack *error_stack)` in
`src/impl/contribute.c`, following the other `impl_*` entry points.

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
   **required** and carries this build's `magpie_version` plus
   `unsupported_jobs`, the in-memory set of jobs this worker has already found it
   cannot run. The server filters on both before it picks a job, so a worker that
   cannot run the job furthest behind its share still gets offered the next.
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
   task. An `algorithm` this build does not know: run unverified and say so once.
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
8. Stop the heartbeat. If `maxtasks` is reached, stop; otherwise repeat.

**Digests are cached** by (resolved path, size, mtime, inode, ctime), with
nanosecond timestamps where the filesystem records them, so a 15 MB lexicon is
hashed once per run rather than once per task. A cached digest must never be the
reason a bad file passes.

**Stopping is cooperative.** A stop request during a task lets the task finish and
submit; a stop request while idle returns immediately. A hard interrupt simply
abandons the claim, which the server's heartbeat timeout reclaims.

**Errors during execution** are reported and the claim is handed back with a `task_failed` decline, so
its slot does not wait out the heartbeat timeout — and so is a result the server
refuses; the loop continues. An error *claiming* or *submitting* is handled by
the retry policy above, and only stops the loop if it exhausts retries.

Because it is a normal command it inherits `-mode async`, so a GUI can start it,
poll status, and stop it with the existing machinery.

### Per-job-type executors

Each builds the in-memory configuration the equivalent command line would, calls
the implementation function directly, and reads results out of the result
structs. No subprocess, no stdout, no parsing.

**Player configuration** arrives as a JSON object per player and maps onto
MAGPIE's per-player settings, where `N` is 1 or 2:

| JSON field | Setting | Notes |
|---|---|---|
| `recorder_type` | `-rN` | `best` for all birdtest jobs |
| `sort_strategy` | `-sN` | `equity` or `score`, for every player: a simmer sorts its candidates before simulating them |
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
| `min_play_iterations` | `-miN` | |
| `threshold` | `-thN` | `'none'` \| `'gk16'` |
| `sampling_rule` | `-saN` | `'round_robin'` \| `'top_two_ids'` |
| `inference_margin` | `-imN` | |
| `utility_w_winpct` | `-uwinN` | blended-utility weight on win% |
| `utility_w_spread` | `-uspreadN` | blended-utility weight on spread |
| `utility_spread_scale` | `-uspreadscaleN` | |
| `movegen_margin` | `-mmargin` | |

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
multi-threading mode and small plays — so nothing is inherited from an earlier
task or the contributor's `settings.txt`. `letter_distribution` and `board_layout` are
**required** on every request, like the settings above: both change what a
task computes, and absent used to mean the build's defaults — a distribution
inferred from the lexicon's name and the layout named for the compile-time
board size — which made them the last two settings a request could leave to the
worker's build. birdtest states both on every request of every job type, so
MAGPIE refuses one that does not (`config_contribute_validate_common`).
`win_pct_model` and
`movegen_margin` are carried on each player object but are really one shared
MAGPIE setting for the whole run, so birdtest validates that a job's two player
configs agree on them before the job is created — the win% model only where both
state one, since a static player has none — and the worker reads each from
whichever player states it.

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
load it as leaves: it goes to `lexica/<lexicon>_birdtest_previous.klv2`, under
the same directory the shipped lexicon data lives in, overwritten per task. The
name starts with the lexicon's because MAGPIE checks leaves against their
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

- **`games` jobs.** SPRT consumes wins, losses and draws, which is exactly what
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
the two aggregates plus the pentanomial, with pairs SPRT computed from the
pentanomial and the divergent counts kept only as a diagnostic.

### Wordmap and rack info table provenance

Whether either file is used is the **job's** decision, not the client's: both
are player settings like any other, sent as `use_wordmap` and `use_rit` on each
player object (and, for `leave_generation`, which has one bot rather than a
player pair, `use_wordmap` on the request itself). A job that omits them runs
without them. Games run dramatically faster with a wordmap, so most jobs will
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
  { "role": "wmp", "name": "NWL23", "sha256": "214a…", "bytes": 104857600,
    "builder": "wmp-1", "build_target": "nehalem" },
  { "role": "rit", "name": "CSW24.CSW_quackle_leaves", "sha256": "157b…",
    "bytes": 1885416048, "builder": "rit-1", "build_target": "nehalem" }
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

A claim that pins **nothing** for a wordmap — an older server — falls back to
the `.wmp.src` sidecar, which is still what protects the CLI. A claim that pins
nothing for a table means no table is loaded at all: a table that cannot be
checked would rank every full rack on leave values nothing verified, and running
without one is always correct, just slower.

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
(`.wit`), a per-substring letter mask move generation prunes with. birdtest
offers no setting for it and pins no hash, so contribute switches it off for
both players before every load. It is opt-in on the CLI (`-wit`), and left
alone a contributor's `settings.txt` — or an earlier command in the same
process — would have carried it into every task: built from the lexicon on disk
it prunes nothing legal, built from an older one it prunes plays that exist,
and nothing in a task would have checked which.

#### Leave generation keeps tables off

Every generation plays with a different KLV, so a table — which caches leave
values — would have to be rebuilt per generation at 1.9 GB and several minutes
on every worker, to replace exactly the values being generated. The server pins
none for a `leave_generation` job and the client loads none.

#### Where the builds run

On the server, in a separate scheduled ECS task (`infra/derived.tf`), not in the
web task: a table build peaks at about 2.4 GB and writes a 1.9 GB file, against
the web task's 1 vCPU and 2 GB. It drains a queue (`derived_data`) under a lease
and exits. **A job whose derived files are not built is not dispatched** — the
same wait as a leave-generation job whose universe is not seeded — because
dispatching without the hash would mean sending a worker no `derived` entry,
which it reads as a server that checks nothing. `/admin/derived-data` is where
that wait is visible.

The gate is a query over the job's config, its players, `input_data` and
`derived_data`, and it runs before the dispatch lock for every candidate job on
every claim. Once a job has been found dispatchable the answer is remembered
in the process for good (`derived::DerivedCache`): what a job needs is fixed at
creation, a `derived_data` row only ever moves toward `built`, and the builder
the query matches on is a constant of the running binary, so nothing can make a
remembered answer wrong. A job still waiting is asked about on every claim,
which is what lets it be dispatched the moment its last file is built.

On the worker, during task execution, after the heartbeat has started: a table
takes minutes, and the heartbeat is what keeps the claim alive through it.

### Heartbeat thread

`POST /api/worker/heartbeat` with `{"claim_token": "..."}` every 30 seconds for
the lifetime of a claim, using `cpthread` and a stop flag. Failures are logged and
ignored: the server treats a missed heartbeat as a lapsed claim and reassigns the
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
`GET /api/worker/client-version` accordingly means "minimum MAGPIE version"
rather than "script version and download URL".

### The Worker API Contract

Six endpoints. Authentication on all of them is either
`Authorization: Bearer <api-key>` **or** `X-Worker-UUID: <uuid>`, never both. A
claim may also carry neither, which is how a new worker asks to be issued a
UUID; every other endpoint answers `401` without an identity, since each acts on
something a claim created.

#### `POST /api/worker/task`

The body is required:

```json
{ "magpie_version": "1.4.0",
  "unsupported_jobs": ["4c7b64ad-8e5e-4db7-aeb0-afc44ee1ebf5"] }
```

Both fields are load-bearing. The version drives the per-job minimum filter —
without it the server would have to assume one, which is a wrong answer dressed as
a safe one — and `unsupported_jobs` is every job this worker has found it cannot
run, for any reason. It is attacker-controlled input flowing into a query, so it
is capped at **200** entries and silently truncated past that (far above any
honest client, since the list is bounded by the jobs a worker has actually been
offered) and bound as an array rather than interpolated. A claim whose body is
missing, malformed or without `magpie_version` is rejected `400` with a message
that names the fix — what to send, and that a MAGPIE which sends neither field
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

`reason` is `data_out_of_date`, `magpie_too_old`, or `both`.

`200` with a task:

```json
{
  "claim_token": "6f3d7198-178a-47c8-9ccc-6aa6995a5a9c",
  "job_id": "4c7b64ad-8e5e-4db7-aeb0-afc44ee1ebf5",
  "min_magpie_version": "1.4.0",
  "worker_uuid": "6f3d7198-178a-47c8-9ccc-6aa6995a5a9c",
  "expected_data": {
    "algorithm": "sha256",
    "files": [
      { "role": "kwg", "name": "NWL23", "path": "lexica/NWL23.kwg",
        "sha256": "3e74af98...", "bytes": 4719596, "tarball_date": "20251004" }
    ]
  },
  "task_request": { "job_type": "games", "...": "..." }
}
```

`expected_data` lists every file this task will load — the deduplicated union over
the job and its players — with the digest the job pins. `role` and `name` are what
the client resolves through `data_filepaths`; `path` and `tarball_date` are for the
message it prints when something does not match. A `leave_generation` task carries
exactly three entries: `kwg`, `letterdist`, `layout`.

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
  "previous_play": null,
  "player": { "name": "static", "recorder_type": "best", "sort_strategy": "equity",
              "lexicon": "NWL23", "leaves": "NWL23", "win_pct_model": null,
              "max_iterations": null, "num_plies": null,
              "num_plays": null, "num_plays_recorded": null,
              "stopping_pct": null, "use_inference": null,
              "time_limit_secs": null } }

{ "job_type": "games",
  "variant": "classic", "letter_distribution": "english", "board_layout": "standard15",
  "seed": "1", "num_games": 10, "game_pairs": false,
  "capture_positions": false,
  "player1": { }, "player2": { } }

{ "job_type": "game_pairs", "...": "as games, with game_pairs true",
  "num_games": 10 }

{ "job_type": "leave_generation",
  "lexicon": "NWL23", "variant": "classic", "letter_distribution": "english",
  "board_layout": "standard15",
  "generation": 2,
  "seed": "7",
  "forced_racks": ["AA", "AB"],
  "previous_artifact_key": "leaves/<job>/generation-1.klv2",
  "use_wordmap": true,
  "num_games": 10000 }
```

`racks` is a batch, not a single rack: the rack space runs to millions and one
rack per task would spend a claim/submit round trip on each.

`previous_artifact_key` is **never null**, generation 1 included: the server
builds a zeroed KLV for it at `generation-0` when the job is created, so every
generation fetches its leaves the same way and the client has no first-generation
branch.

`num_games` is the only thing that ends a leave-generation task: play that many
games, then report. The generation's minimum rack target is **not** sent, and the
client must not stop early on it. Every game contributes occurrences for every
rack it draws, not just the task's `forced_racks`, and the server folds all of
them into its per-generation totals — so games played after the forced racks have
filled still produce coverage the server uses. The target belongs to the server,
which owns the running per-rack totals across every task in the generation and
decides on its own when the generation closes.

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
`derived_mismatch` — the worker built the wordmap or rack info table the job pins
and got different bytes — or `task_failed` — the worker ran the task and could
not produce a result the server accepted;
`missing` is present for `missing_data` and `derived_mismatch`, where `actual`
is the digest of what the worker built. `actual: null` means the file was not
found at all, and a hex string means it was found with different content.

The server derives the task and job from the token, releases the claim immediately
rather than waiting out the heartbeat timeout, and records the gap so an admin can
see what the fleet is missing. Declining is an ordinary outcome, not an error: the
worker adds the job to its unsupported set and claims again.

#### `POST /api/worker/heartbeat`

`{"claim_token": "..."}` → `204`.

#### `POST /api/worker/result`

`{ "claim_token": "...", "result": { } }` → `200` with `{"accepted": true}`, or
`{"accepted": false}` when the claim had already lapsed or the result was already
accepted — which is **not an error**: the work was reassigned or is done. `400`
when the result does not satisfy its shape; `413` over 64 MiB.

```json
{ "racks": [ { "rack": "ABDEELT", "num_moves": 412,
               "moves": [ { "move": "8D BEADLET", "score": 76, "equity": 81.5,
                            "plies": [ { "ply": 0, "bingo_percentage": 0.0,
                                         "average_score": 24.0 } ] } ] } ] }

{ "all_games": { "games": 20, "wins": 11, "losses": 9, "ties": 0,
                 "p1_score_mean": 429.5, "p1_score_sd": 60.8,
                 "p2_score_mean": 403.4, "p2_score_sd": 55.9 },
  "positions": [ ] }

{ "all_games": { "...": "as above" },
  "pentanomial": [12, 3, 140, 5, 40],
  "divergent_games": { "...": "same shape, the divergent subset" } }

{ "racks": [ { "rack": "AA", "count": 30, "mean": 1.5 } ] }
```

Server-side validation, so the client must satisfy it:

- `wins + losses + ties == games`, all non-negative.
- `game_pairs`: `games` is even and non-zero; `pentanomial` is required and must
  agree with the aggregate on the pair count and on player 1's half-points;
  `divergent_games`, if present, has consistent counts with `games` even and
  `<= games`.
- `moves` and `racks` must be non-empty. `moves` carries at most the player
  config's `num_plays_recorded`; `num_moves` says how many were ranked and must
  not be below the number reported. It is optional, for builds that predate it.
- `positions` is present only when the job set `capture_positions`, and each entry
  must fall inside the task's own games — see
  [Position Capture From Games](#position-capture-from-games).

#### `GET /api/worker/artifact?key=<key>`

Returns `application/octet-stream`. Only keys the server itself minted resolve;
anything else is `404`. Used for a generation's KLV.

#### `GET /api/worker/client-version`

`{"min_magpie_version": "...", "download_url": "..."}` — the oldest MAGPIE a
client may contribute with, and where to get it. Not a self-update.

#### Rate limiting

Worker endpoints are limited to roughly one request per second per identity with
a small burst. A `429` carries `Retry-After` in seconds. A task costs at least two
requests, so this is reached under normal operation and must be handled as
backoff, not as an error.

### Client security

- TLS certificate verification on by default, with no way to disable it.
- The API key is never accepted on the command line, never written to
  `settings.txt`, and never appears in status output, logs or errors.
- The worker UUID is minted by the server, never trusted from the client, so a
  client cannot pick or collide an identity on its own.
- **Every field of a task request is untrusted input.** It becomes file paths
  (`previous_artifact_key`) and numeric parameters. Validate lexicon and variant
  against known values before they reach `data_filepaths`, and reject artifact
  keys containing `..` or a leading `/`.
- Bound everything the server can ask for — batch sizes, rack-subset sizes,
  iteration counts. A compromised or buggy server must not be able to make a
  contributor's machine allocate without limit.

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
  read — an assignment of each of the three request shapes (games, opening
  racks, leave generation) carrying `expected_data`, a claim carrying
  `unsupported_jobs` and `magpie_version`, a decline, and each shutdown reason.
  Opening racks earn their own fixture because theirs is the one request that
  carries `racks` and a single `player` rather than a player pair, so nothing
  else pins those two names.

  birdtest's half is enforced (`routes::worker::contract_fixtures` parses every
  fixture against the real wire types, comparing field structure rather than
  bytes so fields stay free to move before release). MAGPIE's half is now too:
  the assignment fixtures are copied into MAGPIE's `test/birdtest_contract/`, and
  `test/contribute_test.c` fails if any key the executors read is missing from
  them.

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
`forbidden`, `not_found`, `conflict`, `payload_too_large`, `rate_limited`,
`unavailable`, `internal`) mapping one-to-one onto the status. `unavailable` is `503` with a `Retry-After`:
every connection of the pool asked was busy for the whole acquire timeout, or a
display read outran its statement timeout (see [Two connection
pools](#two-connection-pools)). That is load rather than a fault, and MAGPIE's
client already backs off and retries a `5xx`. `fields` is omitted when empty and carries per-field
messages so form endpoints can mark individual inputs. A `rate_limited`
response also carries a `Retry-After` header in whole seconds.

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
identically. Anything else from the database is a 500 with a generic message.

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
non-admin session gets `403`, and so does a worker credential.

#### Rate limits

In-memory token buckets, per process, reset on restart.

| Endpoint | Limit | Keyed on |
|---|---|---|
| `POST /api/auth/register` | 10 / hour | Client IP |
| `POST /api/auth/login` | 10 / minute | Client IP **and**, separately, the username tried |
| `POST /api/auth/reset-password/request` | 5 / hour | Client IP **and**, separately, the address asked for |
| `POST /api/worker/{task,result,heartbeat,decline}`, `GET /api/worker/artifact` | 1 / second, **burst 5** | Worker identity (`u:<user-id>` or `a:<uuid>`) |
| `POST /api/worker/task` with no identity | 5 / second, **burst 30** | Client IP, shared by every new contributor behind one address until each is issued a UUID |

"Client IP" is the `X-Forwarded-For` entry `TRUSTED_PROXY_HOPS` from the right —
the ALB's or Nginx's view of the caller — or the TCP peer when that is 0. Keying
on the peer behind a proxy would put the whole site in one bucket.

The burst matters: a task costs at least two requests, so a strict one-per-second
limit with no burst would throttle a well-behaved client.

Password reset is checked twice, and both halves are load-bearing. Without a
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
(`.env` locally, task-definition variables in ECS), connects the pool, **runs
migrations before binding** so a container never serves traffic against an
out-of-date schema, connects the display pool, fails any input-data import or
export left `running` by a previous process, releases any leave-generation
transition one left open (so the next claim takes it over rather than the
half-hour takeover timeout), and only then listens.

On the way out it **shuts down gracefully**: `SIGTERM` (what ECS sends before it
escalates to `SIGKILL` at the stop timeout) and `SIGINT` stop it accepting new
connections and let in-flight requests finish. Without that, a deployment drops
whatever is in flight — and a worker that has just uploaded a completed batch
loses it, because the claim it was for is still `claimed` and stays that way
until the heartbeat timeout, so the retry is answered `accepted: false`.

#### Audit actions

Every significant action writes an `audit_log` row inside the same transaction
as the action itself, so an audit failure rolls back what it describes. Claims
and submissions are the deliberate exception: `task_claims` already records who
claimed and completed which task and when, so a row per claim and per submission
duplicated it, cost a write each on the path a worker waits on, and made up most
of the log's growth.

| Action | Written by |
|---|---|
| `user.registered` | Registration |
| `task.declined` | A worker declining, with the reason in `reason` |
| `job.created` / `job.activated` / `job.deactivated` / `job.completed` | Admin job lifecycle |
| `job.purged` / `job.purged.census` | Purge |
| `job.deleted` / `job.deleted.census` | Delete |
| `user.deleted` / `user.deleted.census` | Account deletion |
| `job.artifacts_rebuilt` | Artifact rebuild, with counts |
| `job.export_started` | An admin starting a results export |
| `input_data.import_staged` / `input_data.import_confirmed` | Tarball import |
| `worker.banned` | Ban, with the free-text reason |
| `worker.unbanned` | Lifting a ban, naming the identity rather than the ban row, which is gone |
| `user.signed_out_everywhere` | "Sign out everywhere" on the account page |
| `rating_pool.created` / `rating_pool.member_added` / `rating_pool.member_removed` | Rating pool membership, each of which refits the pool |
| `derived_data.retried` | An admin re-queueing a failed wordmap or rack info table build |

The `.census` rows are the reason the destructive ones are worth having.
`purge_job`, `delete_job` and `delete_user` each count what they are about to
destroy — tasks, claims, results, progress and artifact rows — and write that as a
single line into `audit_log.reason` **before** the delete runs, inside the same
transaction. After the delete commits, that row is the only surviving
description of what the job or account held, and it is what a selective restore
is scoped against. `audit_log` deliberately has no foreign keys: one to `jobs` or
`users` would either block those deletions outright — every job has a
`job.created` row, every user a `user.registered` one — or rewrite the history the
log exists to keep.

---

### Worker API

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/worker/client-version` | The oldest MAGPIE a client may contribute with (`MIN_MAGPIE_VERSION`) and where to get it. Not a self-update: MAGPIE is a compiled binary and the client cannot replace itself. |
| `POST` | `/api/worker/task` | Send a task claim. The body is **required** and carries `magpie_version` and `unsupported_jobs`. Returns a task assignment, a `shutdown` directive, or `204`. |
| `POST` | `/api/worker/decline` | "I claimed this and cannot do it." Releases the claim immediately rather than waiting out the heartbeat timeout, and records the gap. |
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

All Admin API endpoints require the requesting user to have `is_admin = TRUE`. Requests from non-admin authenticated users or anonymous workers are rejected with `403 Forbidden`.

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/admin/player-configs` | List all player configurations. |
| `POST` | `/api/admin/player-configs` | Create a new player configuration. |
| `GET` | `/api/admin/player-configs/:id` | Get a single player configuration. |
| `DELETE` | `/api/admin/player-configs/:id` | Delete a player configuration. Rejected if any job, rating pool, rating history or clone references it. |
| `POST` | `/api/admin/jobs` | Create a new job. Created in the `inactive` state — see `.../activate` to set its allocation and start dispatching work. |
| `POST` | `/api/admin/jobs/:id/deactivate` | Set a job to inactive. Workers will no longer be assigned tasks from it. Refused (`409`) for a completed job. |
| `POST` | `/api/admin/jobs/:id/activate` | Activate an inactive job. Body: `{ "allocation": int }`. Sets allocation and transitions status to active. |
| `POST` | `/api/admin/jobs/:id/complete` | Force-complete a job immediately, regardless of task progress. |
| `POST` | `/api/admin/jobs/:id/purge` | Delete every claim, result, leave-gen progress and staged-result row, selection cursor, artifact row and task for a job, reset its dispatch counter and rejoin it at parity with the other jobs (`claims_baseline`), then re-seed its initial state. Ratings are untouched: they belong to rating pools, and the sweep refits a pool whose evidence changed. Returns `{ tasks_reset }`. Writes a census of what it destroyed to the audit log first. |
| `DELETE` | `/api/admin/jobs/:id` | Delete a job and all its tasks. |
| `DELETE` | `/api/admin/users/:id` | Delete a user account: anonymize it in place, keeping its claims and records so no donated compute is lost (see Admin API semantics). |
| `POST` | `/api/admin/workers/ban` | Ban a worker by user ID or anonymous UUID. One ban per identity: a second is `409`, so that unban means what it says. |
| `DELETE` | `/api/admin/workers/ban/:id` | Remove a ban, and with it the identity's only ban. |
| `GET` | `/api/admin/audit-log` | Query the audit log with filtering and pagination. |
| `GET` | `/api/admin/input-data` | List known input data rows — path, role, name, digest, tarball date. |
| `DELETE` | `/api/admin/input-data/:id` | Delete an input data row. A row referenced by a job, player config or rating pool cannot be deleted; the foreign key is the safety mechanism and the error is rendered as "used by N jobs". |
| `POST` | `/api/admin/input-data/imports` | Start a tarball import. Returns `202` and an import id immediately; the fetch and diff run as a background task. |
| `GET` | `/api/admin/input-data/imports/:id` | Poll an import: progress while running, the staged diff once staged, or the failure reason. |
| `POST` | `/api/admin/input-data/imports/:id/confirm` | Insert the staged **new** rows, in one transaction. |
| `GET` | `/api/admin/jobs/:id/data-gaps` | What workers reported they were missing for this job, from `worker_data_gaps`. |
| `GET` | `/api/admin/jobs/:id/results/stream` | Newline-delimited JSON (`application/x-ndjson`) of every record for the job, streamed straight from a database cursor so a download never buffers a whole job in memory. The source table follows the job type: position analyses (each with its ranked moves and plies nested), game results, or leave-rack progress — the export's own queries. `?positions=true` (games and game-pairs jobs) streams the positions the job captured instead of its result rows. At most two run at once; a completed job with a ready export gets a `303` to it instead. |
| `POST` | `/api/admin/jobs/:id/export` | Build a **completed** job's results into one gzipped NDJSON object in the artifact store. `202` with an id; the work runs on a background task. `409` for a job that is not completed, or whose last claims are still in flight. |
| `GET` | `/api/admin/jobs/:id/export` | The newest export for the job, with a presigned `download_url` once it is ready — and, for a games or game-pairs job that captured positions, a `positions_download_url` for the second object holding them. |
| `GET` | `/api/admin/workers` | The contributor list with anonymous workers' real UUIDs, which banning one needs; the public list carries pseudonyms only. |
| `GET` | `/api/admin/derived-data` | Every wordmap and rack info table the server has been asked to build: state, builder, hash, attempts and the last error. A job whose files are not `built` is not dispatched, and this is where that wait — or the failure behind it — is visible. |
| `POST` | `/api/admin/derived-data/retry` | Put a `failed` build back in the queue (`{ role, name }`). Explicit rather than automatic: a build is a pure function of its inputs, so three failures mean a missing input or a broken binary, which a fourth attempt does not fix. |
| `GET` | `/api/admin/fleet` | What the field is running, from `task_claims.magpie_version`. |
| `GET` | `/api/admin/backups` | Recent backup runs and how stale the newest successful one is. Read-only: backups are performed by a scheduled task, never by the server — see [Backups and Restore](#backups-and-restore). |
| `POST` | `/api/admin/rating-pools` | Create a rating pool: name, scope, and the anchor config that fixes the scale. The anchor joins as a member automatically. |
| `POST` | `/api/admin/rating-pools/:id/members` | Add a player config to the pool and refit it. Returns the new run id. |
| `DELETE` | `/api/admin/rating-pools/:id/members/:config_id` | Remove a config and refit. Refused for the pool's anchor, which every other rating is measured against. |
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
iteration and recorded counts at least 1, `stopping_pct` strictly between 0 and
100, margins and weights finite and non-negative — and a config with any
simulation setting must simulate at least one ply, because MAGPIE decides whether
a player simulates on plies alone: a "simmer" without plies would play
statically on every worker. A simmer must also set `max_iterations`, and
`time_limit_secs` of 0: MAGPIE applies a time limit only above 0 (a null means
its 60-second default), and a limit makes how far a simulation gets depend on the
contributor's hardware, so the iteration budget bounds it instead. `use_rit` is
accepted: a rack info table carries precomputed leave values that move
generation uses in place of the leaves the config pins, which is why it was
refused until the server could build the table for this exact (lexicon, leaves)
pair and pin its hash. A job whose players ask for one is not dispatched until
it is built.

Whatever the body leaves out is filled from MAGPIE's defaults
(`backend/src/magpie_defaults.rs`) before the row is written, so the stored config
is exactly what every request built from it states: `sort_strategy` (`equity`),
`num_plies` (0, static), `num_plays` (100), `num_plies_recorded` (2),
`movegen_margin` (5), `use_wordmap` (false), and for a simmer every simulation
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
| `redundancy` | 1 |
| `min_magpie_version` | the server-wide floor |
| `games_per_batch` / `pairs_per_batch` | 1 |
| `racks_per_batch` | 500 |
| `rack_size` | 7 |
| `sprt_alpha` / `sprt_beta` | 0.05 |
| `elo_low` / `elo_high` | −10 / +10 |
| `generation_count` | 1 |
| `use_wordmap` (leave generation) | **true** — it is the most game-heavy job type there is and a wordmap is a large speedup; workers build one on demand |
| `capture_positions` | false |

A job's run-wide MAGPIE settings are not fields of the body: `bingo_bonus` (50)
and `sim_cutoff` (0.005) are written onto the job from MAGPIE's defaults at
creation and stated on every request, for the reason player configs state
theirs.

Beyond role matching, creation enforces six rules the schema cannot express:

- **Settings a worker can run and a test can evaluate.** `redundancy` at least 1;
  `variant` is `classic` or `wordsmog`; batch sizes at least 1 (`racks_per_batch`
  at most 10,000); `rack_size` 1–7; `max_*` at least 1 and
  `min_*` at least 0; `sprt_alpha` and `sprt_beta` strictly between 0 and 1 with a
  sum below 1; `elo_low` below `elo_high`. Every violation is reported at once as
  a field error. A zero batch would make every claim regenerate the seed the last
  one took and retry forever; inverted hypotheses flip the LLR's sign.

- **Cross-player compatibility**, ported from MAGPIE's own name-prefix rules.
  Both players' lexicons must be compatible with each other and each with its own
  leaves, and both with the job's single letter distribution. birdtest must not
  be able to build a job MAGPIE would refuse to load.
- **Shared-option agreement.** `win_pct_model` and `movegen_margin` are sent per
  player but are really one MAGPIE setting for the whole run, so two *different*
  configs must agree on both — the win% model only where both state one, since a
  static player has none and static-versus-simmer is the mix a games job most
  often wants. Skipped when both slots name the same config, which
  is a legal and useful degenerate case.
- **A recorder that can rank, for opening racks.** `recorder_type = 'best'` with
  `num_plays_recorded` above 1 is refused: `best` records one move, so the job
  would store one move per rack while claiming to store ten. See
  [How much of an analysis is kept](#how-much-of-an-analysis-is-kept).
- **A capture job's simmers consider at least what is captured.** With
  `capture_positions` on, MAGPIE raises each simming player's `num_plays` to
  player 1's `num_plays_recorded`, so a smaller `num_plays` would make turning
  capture on change the games. Refused, naming the player.
- **Leave generation runs at redundancy 1.** Its redundant copies replay the
  same seed only when MAGPIE is single-threaded; multi-threaded, they are
  different samples, and there is no integrity use for them today.

The response is `{ job }`. Creation writes no rows up front for any job type: no
tasks, and no leave-generation rack universe, which the first claim seeds as it
does every generation's.

Job creation also writes generation 1's zeroed KLV, **after** the transaction
commits rather than inside it: it is a multi-megabyte build and an object-store
write, and holding a transaction open across it would be wrong.

**Activating a job** sets its allocation and requires that the active jobs
still sum to at most 100% — checked here rather than as a database constraint,
because the intermediate states an admin passes through while rebalancing would
violate a constraint even when the end state is fine. The error names how much
room is left. An allocation of 0 is accepted and means what inactive means: the
job is offered to nobody until it is raised. Activation also resets the job's
`claims_baseline`, so it joins level with the job furthest behind rather than
with a lifetime deficit to work off at the others' expense — and since
activating an active job is how its allocation is changed, a changed share
takes effect from that moment rather than being applied retroactively to every
claim the job ever issued. A completed job cannot be
reactivated. Activations are serialized with an advisory lock, so two at once
cannot jointly exceed 100%. Activating a leave-generation job whose generation-0 KLV was never
written — creation writes it after committing, so an object-store failure there
leaves the job without one — builds it first.

**Deleting a user** anonymizes the account rather than removing it. Personal
data and credentials go: the username becomes `deleted-<id>`, the email
`<id>@deleted.invalid`, the password hash an unusable value, `is_admin` false,
API keys, confirmation codes and reset tokens are deleted, `session_generation`
is incremented so every session ends, and `deleted_at` is set. Login, password
reset and `CurrentUser` all refuse a deleted account, and `/api/users` omits it.
Contributions stay: the account's claims and results are kept under the
tombstone and **no counter is rolled back**, so no donated compute is lost —
including captured in-game positions other redundant claims deduplicated
against, and leave-generation occurrences that could not have been subtracted
anyway. Open claims are left to time out; nothing can submit for them once the
keys are gone. `jobs.created_by`, `player_configs.created_by` and
`worker_bans.banned_by` keep pointing at the tombstone, and `audit_log` records
the census taken before the change. Deleting an already-deleted account is a
404, and an admin cannot delete their own account.

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
| `GET` | `/api/jobs` | List jobs with status and summary stats. Paginated. |
| `GET` | `/api/jobs/:id` | Job detail, configuration, and aggregate statistics. |
| `GET` | `/api/jobs/:id/results` | Task records for a job, paginated by cursor (`?cursor=`; see [Pagination](#pagination)). `?worker=` filters to one contributor by username or anonymous pseudonym (`anon_id`), resolved to an identity before the job is read; a name that is nobody's is an empty page. `?rack=` is opening-rack jobs only and switches to a single-rack lookup, returned whole. |
| `GET` | `/api/jobs/:id/stream` | SSE stream of live stat updates for a job. Pushes an event after accepted results, coalesced to at most one a second. |

| `GET` | `/api/users` | List all registered user accounts with contribution stats. Paginated. |
| `GET` | `/api/workers` | Contributor stats for all workers (anonymous and authenticated), paginated. |
| `GET` | `/api/rating-pools` | Rating pools with their conditions, member counts and last fit time. |
| `GET` | `/api/rating-pools/:id` | Latest fit for one pool: run provenance, every member's rating with uncertainty, and the residuals. |
| `GET` | `/api/rating-pools/:id/history` | Stored runs' ratings, oldest first, thinned to at most 500 runs evenly spaced over the pool's history — the history chart's series. |

**Rack lookup** canonicalizes the query before matching: uppercased, whitespace
trimmed, letters sorted. A rack is a multiset of tiles, so `AEINRST` and
`TSRNIEA` are the same rack and a user typing either should find it. It matches
only opening-rack records (`game_index IS NULL`), so an incidentally-captured
in-game position with the same rack does not surface as an opening-rack analysis.
The full ranked move list is returned in one page rather than paginated. Under
redundancy above 1 a rack has one record per accepted claim, and the lists come
back one record after another, each in rank order, rather than interleaved by
rank.

---

## Frontend Routes

SvelteKit uses file-based routing under `frontend/src/routes/`. Each directory with a `+page.svelte` is a page. Layout files (`+layout.svelte`) apply to all routes nested beneath them.

### Public Routes

| Route | Page |
|---|---|
| `/` | Landing page — brief description of birdtest, links to the job list and the worker setup guide. |
| `/jobs` | Job list — all jobs with type, status, allocation, and completion counter. Loaded on visit rather than live: there is no job-list stream, only a per-job one. |
| `/jobs/[id]` | Job detail — job-type-specific stats and per-worker contribution table. Live-updated via SSE. |
| `/users` | Registered user list — all user accounts with contribution stats. |
| `/workers` | Contributor leaderboard — all workers (anonymous and authenticated) ranked by tasks completed. |
| `/ratings` | Rating pool list — each pool's conditions, member count and last fit. |
| `/ratings/[id]` | [The ratings page](#the-ratings-page) — ratings with uncertainty, history, and residuals. Admin controls for membership appear inline for admins. |

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
| `/account` | Account overview — username, email, confirmation status, API key list (labels only), generate/revoke API keys. |

### Admin Routes

Protected by a layout guard (`/admin/+layout.svelte`) that requires `is_admin = true`; redirects non-admins to `/`.

| Route | Page |
|---|---|
| `/admin` | Admin overview — redirects to `/admin/jobs`. |
| `/admin/jobs/new` | Create job form — job type selector, then type-specific config fields. |
| `/admin/jobs/[id]` | Admin job view — same stats as the public detail page plus controls: activate, deactivate, force-complete, purge, delete (the last three ask first: none can be taken back), an artifact check and "merge progress now" for leave generation, and for a completed job the export panel — start, poll, download. |
| `/admin/player-configs` | Player config list — name, recorder type, sort strategy, sim parameters. |
| `/admin/player-configs/new` | Create player config form. |
| `/admin/users` | User account list — delete accounts. (Contribution stats are shown publicly at `/users`.) |
| `/admin/workers` | Worker ban management — ban / unban workers by user ID or anonymous UUID. |
| `/admin/audit-log` | Audit log viewer — filterable by action type, actor, and target; paginated. |
| `/admin/input-data` | Input data browser and import wizard — pick a tarball date, watch the import, review the staged diff, confirm. |
| `/admin/fleet` | What MAGPIE versions have claimed work recently, from `task_claims.magpie_version`. |
| `/admin/derived-data` | The wordmap and rack info table build queue: what is built, pending or failed, and a retry for the failures. |
| `/admin/backups` | Recent backup runs and the staleness of the newest successful one. |

---

### The ratings page

`/ratings/[id]` is where a pool's fit is read. Four panels, each answering a
different question, and the design choices in them are load-bearing:

**Ratings, as a dot plot with error bars.** Deliberately not a bar chart: a bar
encodes magnitude from zero, and Elo has no meaningful zero — the scale is
anchored wherever the pool's anchor was pinned, so bar length would imply a ratio
that does not exist. A dot on a common scale encodes position, which is what a
rating is. The error bar matters as much as the dot, because a config with two
hundred pairs and one with two million otherwise produce identical-looking
numbers and only the interval says which to believe. A config with no path to the
anchor is listed beneath the chart as **unrated** rather than drawn at a number.

**A table of every config**, since the chart caps what it draws and the table
must not. This is also the accessible view of the same data.

**Rating history**, one line per config, from the run snapshots — thinned to at
most 500 runs spaced evenly over the pool's life, the first and the newest always
kept, since a pool with an active job is refit every two minutes. The categorical
palette is a fixed list rather than a generator, so past six configs the page
shows the top six by rating and says how many it left out — inventing a seventh
hue nobody can distinguish would be worse than omitting it. Every line is
directly labelled at its right end, so identity never depends on colour alone.

**Where the model disagrees with the games.** The residual table: actual score
versus predicted, per head-to-head, largest disagreement first. This is the panel
that makes non-transitivity visible instead of letting it quietly distort the
ranking, and when enough head-to-heads are badly mispredicted the page says
outright that the ratings should be read as a summary rather than a ranking.

Admin controls live inline on this page rather than under `/admin`, because
adding or removing a config is an act whose consequence — every other rating
moving — is only legible next to the ratings themselves.

## Directory Structure

```
birdtest/
├── .github/
│   └── workflows/
│       ├── ci.yml                  # per pull request: clippy + backend tests (with Postgres),
│       │                           # frontend check/build, both images, terraform validate,
│       │                           # and MAGPIE's half of the message contract
│       └── nightly.yml             # tier 6: a real MAGPIE runs one task of every job type
├── docker-compose.yml               # the whole local stack: Postgres, MinIO (S3 stub), backend,
│                                    # frontend, plus `dev` and `fake-worker` profiles — see Development
├── .env.example                     # compose port overrides
├── docker/
│   └── Dockerfile                  # backend (with a pinned MAGPIE built in), derived-builder and
│                                   # fake-worker targets; only the fake worker needs no MAGPIE
├── scripts/                        # backup, restore and local-snapshot shell scripts —
│                                   # see Backups and Restore
│   ├── backup.sh                   # the nightly pg_dump, its manifest, and the `backups` row
│   ├── restore-drill.sh            # the monthly automated restore drill
│   ├── restore-roundtrip.sh        # dump -> drop -> restore -> verify, against the local stack
│   ├── dev-dump.sh                 # snapshot the local Postgres + MinIO state
│   ├── dev-restore.sh              # put it back
│   ├── leave-gen-bench.sh          # time a generation transition's SQL against any database,
│   │                               # in a rolled-back transaction — see Dashboard,
│   │                               # "What these reads cost"
│   ├── dev.py                      # bring the stack up with real MAGPIE contributors
│   ├── seed.py                     # seed an admin, input data, player configs and jobs
│   ├── e2e_magpie.py               # tier 6: one real `magpie contribute` task per job type
│   └── scrub.sql                   # strip emails, password hashes and tokens after a local restore
├── backend/                        # Axum web server (Rust)
│   ├── Cargo.toml
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
│       │                           # data directories — see MAGPIE_DEPENDENCY.md
│       ├── magpie_standard15.txt   # the board layout every scratch directory carries so MAGPIE starts
│       ├── magpie_defaults.rs      # MAGPIE's defaults, written into player configs and jobs at creation
│       ├── derived.rs              # wordmaps and rack info tables: what a job needs, the build
│       │                           # queue, the builds, and the dispatch gate
│       ├── backups.rs              # reads the `backups` table; never performs a backup
│       ├── auth/
│       │   ├── mod.rs              # CurrentUser / AdminUser / WorkerIdentity extractors
│       │   ├── session.rs          # Paseto token creation / validation
│       │   ├── api_key.rs          # API key, password and code hashing
│       │   └── csrf.rs             # CSRF double-submit verification
│       ├── email.rs                # SES / console mail backends
│       ├── artifacts.rs            # S3 (MinIO in dev) artifact store; multipart upload and presigned reads
│       ├── exports.rs              # a completed job's results as one gzipped NDJSON artifact
│       ├── ratelimit.rs            # in-memory governor token buckets
│       ├── audit.rs                # append-only audit log writes
│       ├── scheduler.rs            # job selection, lazy reclamation, task claiming
│       ├── jobstats.rs             # aggregate job stats (REST + SSE payload)
│       ├── ratings.rs              # rating pools: evidence, fits, snapshots
│       ├── stats/
│       │   ├── mod.rs
│       │   ├── sprt.rs             # SPRT LLR and boundaries
│       │   └── bradley_terry.rs    # batch anchored rating fit (MM)
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
│       │   └── leave_gen.rs
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
│       │   └── public.rs           # /api/jobs/*, /api/users, /api/workers
│       └── sse.rs                  # SSE broadcaster (job result push)
│
├── frontend/                       # SvelteKit app
│   ├── Dockerfile                  # static build served by Nginx — the same artifact ECS runs
│   ├── docker/nginx.conf           # SPA fallback + /api proxy (SSE needs proxy_buffering off)
│   ├── package.json
│   ├── svelte.config.js
│   ├── vite.config.ts
│   └── src/
│       ├── app.html
│       ├── app.css
│       ├── lib/
│       │   ├── api.ts              # typed fetch wrappers for every API endpoint
│       │   ├── auth.ts             # session store (current user, is_admin)
│       │   ├── sse.ts              # SSE subscription helper
│       │   ├── format.ts           # shared display formatting (job type labels, durations)
│       │   └── components/         # shared UI components
│       │       ├── JobStatusBadge.svelte
│       │       ├── WorkerTable.svelte
│       │       ├── Pagination.svelte
│       │       ├── ProgressBar.svelte
│       │       ├── OutcomeChart.svelte   # LayerCake: win/loss/draw over time
│       │       ├── RatingDotPlot.svelte  # ratings with error bars (not a bar chart: Elo has no zero)
│       │       ├── RatingHistoryChart.svelte  # rating over time, one line per config
│       │       ├── ResidualMatrix.svelte # actual vs predicted per head-to-head
│       │       ├── Bars.svelte           # LayerCake mark layer
│       │       └── AxisY.svelte          # LayerCake axis layer
│       └── routes/
│           ├── +layout.svelte      # global layout (nav bar, footer)
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
│               ├── +page.svelte                    # /admin (redirects to /admin/jobs)
│               ├── jobs/
│               │   ├── new/
│               │   │   └── +page.svelte            # /admin/jobs/new
│               │   └── [id]/
│               │       └── +page.svelte            # /admin/jobs/[id]
│               ├── player-configs/
│               │   ├── +page.svelte                # /admin/player-configs
│               │   └── new/
│               │       └── +page.svelte            # /admin/player-configs/new
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
    └── ssm.tf                      # SSM Parameter Store entries (names only; values set manually)
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
    username             TEXT NOT NULL UNIQUE,
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
    last_completed_at    TIMESTAMPTZ,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Serves /api/users, which ranks accounts by contribution. Partial because a
-- deleted account is never listed.
CREATE INDEX users_contribution_idx ON users (tasks_completed DESC, created_at ASC)
    WHERE deleted_at IS NULL;

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

CREATE TABLE api_keys (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    key_hash     TEXT NOT NULL UNIQUE,
    label        TEXT,
    is_active    BOOLEAN NOT NULL DEFAULT TRUE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ
);
-- Enforce the 100-key limit per user at the application layer, not via a DB constraint.

-- Workers

CREATE TABLE anonymous_workers (
    uuid          UUID PRIMARY KEY,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Any request from this identity touches this, at most once a minute: it
    -- answers "is this worker still around".
    last_seen_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- The contribution counters, mirroring users.tasks_completed /
    -- last_completed_at; see there for why they are counters and who
    -- decrements them. `last_completed_at` is distinct from `last_seen_at`
    -- above: one is the last task finished, the other is the last request of
    -- any kind, and the contributor list shows the first.
    tasks_completed   BIGINT NOT NULL DEFAULT 0 CHECK (tasks_completed >= 0),
    last_completed_at TIMESTAMPTZ
);

-- Serves the anonymous half of /api/workers, which merges both kinds of
-- identity in one ranking. Partial: an identity that has completed nothing is
-- not a contributor and is not listed.
CREATE INDEX anonymous_workers_contribution_idx
    ON anonymous_workers (tasks_completed DESC) WHERE tasks_completed > 0;

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

CREATE INDEX input_data_role_name_idx ON input_data (role, name);

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
    state          TEXT NOT NULL DEFAULT 'running'
                   CHECK (state IN ('running', 'staged', 'confirmed',
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

-- Derived files: the wordmaps and rack info tables the server builds a
-- reference copy of, and the hash a worker has to reproduce.
--
-- Neither file is ever shipped -- 179 MB and 1.9 GB for CSW24 -- so every
-- machine that needs one builds it from files it already has. What travels
-- instead is the SHA-256 the server's own pinned MAGPIE got from the same
-- inputs: a worker builds its own copy and uses it only if the bytes agree,
-- and declines the task otherwise. See MAGPIE_DEPENDENCY.md.
--
-- The key is the whole identity of the file rather than a surrogate, because
-- what makes two derived files the same file is that they were built from the
-- same inputs by the same builder. A wordmap depends on a .kwg and the letter
-- distribution it is built against; a rack info table depends on a .klv2 as
-- well, because its entries carry precomputed leave values.
--
-- `builder` is separate from the MAGPIE version on purpose. A CSW24 wordmap
-- built in December 2025 and one built nine months later differ in 72,852,152
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
    job_type   job_type NOT NULL,
    -- NULL until the job is first activated; set by the admin at activation
    -- time. Every active job's share of the fleet: the scheduler hands each
    -- claim to the active job furthest behind
    -- `(claims_issued - claims_baseline) / allocation`, and the active jobs
    -- may allocate at most 100% between them. There is
    -- no priority: a job that should get nothing for now is set to 0%, which
    -- is exactly what `inactive` means, and a job that should get everything
    -- is the only one above 0%.
    allocation INT CHECK (allocation BETWEEN 0 AND 100),
    -- Number of independent workers that must complete each task. Default 1 = single-claim behavior.
    redundancy INT NOT NULL DEFAULT 1 CHECK (redundancy >= 1),
    -- Jobs start inactive; admin activates with an allocation percentage.
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
    bingo_bonus   INT NOT NULL,                              -- -bb
    sim_cutoff    DOUBLE PRECISION NOT NULL CHECK (sim_cutoff >= 0 AND sim_cutoff <= 100),  -- -cutoff
    -- Minimum MAGPIE version workers must have to execute tasks for this job,
    -- as sortable parts. Semver in TEXT compares lexically, where '1.10.0' <
    -- '1.9.0' -- a bug that appears only once a minor version reaches double
    -- digits, i.e. long after it is written.
    --
    -- Not nullable: every job pins input data, and a client too old to
    -- understand expected_data contributes unverified rather than declining,
    -- so "no floor" is not a state worth being able to express. 0.1.0 is
    -- `birdtest-contribute`'s pre-release version: neither birdtest nor the
    -- branch is in production yet, so everything the protocol relies on --
    -- every result-changing setting stated on the request, input data and
    -- derived files checked against the hashes the job pins, the word info
    -- table switched off before every load, a seed on every task -- is in
    -- 0.1.0, and the version moves only when a release changes what a task
    -- computes. The default here is the same value as the server's
    -- MIN_MAGPIE_VERSION, which create_job writes explicitly; the two are kept
    -- equal so a row written any other way (a restore, a hand insert) does not
    -- floor a job below the server.
    min_magpie_major INT NOT NULL DEFAULT 0 CHECK (min_magpie_major >= 0),
    min_magpie_minor INT NOT NULL DEFAULT 1 CHECK (min_magpie_minor >= 0),
    min_magpie_patch INT NOT NULL DEFAULT 0 CHECK (min_magpie_patch >= 0),
    -- Every claim ever issued for this job, abandoned and declined ones
    -- included: the deficit the scheduler orders on. Kept as a counter rather
    -- than counted, because counting task_claims on every claim request costs
    -- time proportional to the job's whole history. Only ever incremented,
    -- except by a purge, which deletes the claims it counts.
    claims_issued   BIGINT NOT NULL DEFAULT 0 CHECK (claims_issued >= 0),
    -- Where this job's share is measured *from*. The scheduler orders on
    -- `(claims_issued - claims_baseline) / allocation`, and the baseline is
    -- reset -- on activation, on an allocation change, on a purge -- so that
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
    -- Progress totals the dashboard reads, maintained in the submit transaction
    -- rather than counted on read (PLAN.md, "What these reads cost"). Both are
    -- incremented
    -- once per task, on its FIRST accepted result, because that is the row the
    -- reads they replace selected: with redundancy > 1 the later claims of a
    -- task replay the same deterministic work, and summing all of them would
    -- multiply every total by the redundancy.
    --
    -- games_completed counts GAMES for both games and game_pairs; a pairs job's
    -- unit count is half of it, exactly as the read derived it. racks_analyzed
    -- counts distinct opening racks with an accepted analysis, which is a plain
    -- sum because each task covers its own disjoint slice of the rack space.
    --
    -- Neither is authoritative for anything that decides: SPRT still reads
    -- game_results, so a drifted counter shows a wrong number on a page and
    -- cannot stop a job early. A purge zeroes them; a partial restore
    -- recomputes them (RUNBOOK 2.3).
    games_completed BIGINT NOT NULL DEFAULT 0 CHECK (games_completed >= 0),
    racks_analyzed  BIGINT NOT NULL DEFAULT 0 CHECK (racks_analyzed >= 0),
    -- Tasks created, and tasks that reached `completed`. The job list shows
    -- both for every job on the page, and counting them meant two COUNT(*)s
    -- over `tasks` per job per page view -- linear in each job's whole history,
    -- on the site's index. Same reasoning and same caveats as the two counters
    -- above: nothing decides anything from them, a purge zeroes them, and a
    -- partial restore recomputes them (RUNBOOK 2.3).
    tasks_total     BIGINT NOT NULL DEFAULT 0 CHECK (tasks_total >= 0),
    tasks_completed BIGINT NOT NULL DEFAULT 0 CHECK (tasks_completed >= 0),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    activated_at    TIMESTAMPTZ,
    deactivated_at  TIMESTAMPTZ
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
--   player; 'score' = sort by raw score only. A simming player sorts its candidates too, before
--   simulating them, so every row states one. Both static and simming players are valid in
--   games/game_pairs jobs.
--
-- Simulation columns are all NULL for a static (no-sim) player.

CREATE TABLE player_configs (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name             TEXT NOT NULL UNIQUE,  -- human-readable label, e.g. "simmer-NWL23-4ply"
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
    movegen_margin         DOUBLE PRECISION NOT NULL, -- move-gen equity margin for 'equity' recording (-mmargin)
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
    total_racks       BIGINT NOT NULL CHECK (total_racks >= 0)
);

CREATE TABLE job_game_config (
    job_id              UUID PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
    -- lexicon, variant and letter distribution live on the job now: the first
    -- on the player configs, the other two on `jobs`.
    player1_config_id   UUID NOT NULL REFERENCES player_configs(id),
    player2_config_id   UUID NOT NULL REFERENCES player_configs(id),
    games_per_batch     INT NOT NULL DEFAULT 1,
    -- Two finish conditions: SPRT significance (evaluated after min_games) OR reaching max_games.
    min_games           INT NOT NULL,   -- SPRT is not evaluated until this many games are complete
    max_games           INT NOT NULL,   -- job auto-completes at this count regardless of SPRT
    -- SPRT parameters (H0: elo_diff = elo_low, H1: elo_diff = elo_high)
    sprt_alpha          DOUBLE PRECISION NOT NULL DEFAULT 0.05,
    sprt_beta           DOUBLE PRECISION NOT NULL DEFAULT 0.05,
    elo_low             DOUBLE PRECISION NOT NULL DEFAULT -10.0,
    elo_high            DOUBLE PRECISION NOT NULL DEFAULT 10.0,
    -- Keep the position analyses the worker produces while playing. A worker
    -- analyses a position every turn regardless; this decides whether those are
    -- recorded. Off by default: at ~22.5 turns a game it roughly doubles the
    -- rows a job produces.
    capture_positions   BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE job_game_pair_config (
    job_id              UUID PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
    -- lexicon, variant and letter distribution live on the job now: the first
    -- on the player configs, the other two on `jobs`.
    player1_config_id   UUID NOT NULL REFERENCES player_configs(id),
    player2_config_id   UUID NOT NULL REFERENCES player_configs(id),
    pairs_per_batch     INT NOT NULL DEFAULT 1,
    min_pairs           INT NOT NULL,
    max_pairs           INT NOT NULL,
    sprt_alpha          DOUBLE PRECISION NOT NULL DEFAULT 0.05,
    sprt_beta           DOUBLE PRECISION NOT NULL DEFAULT 0.05,
    elo_low             DOUBLE PRECISION NOT NULL DEFAULT -10.0,
    elo_high            DOUBLE PRECISION NOT NULL DEFAULT 10.0,
    -- Keep the position analyses the worker produces while playing. A worker
    -- analyses a position every turn regardless; this decides whether those are
    -- recorded. Off by default: at ~22.5 turns a game it roughly doubles the
    -- rows a job produces.
    capture_positions   BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE job_leave_config (
    job_id         UUID PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
    -- The one place a lexicon still sits on a job: leave generation has a
    -- single bot and no player_configs row to hold it. It needs no klv_id
    -- (every generation's leaves come from the server-built KLV artifact, and
    -- generation 1's is a zeroed one) and no winpct_id (the bot plays
    -- statically). Its complete data requirement is this plus the job's
    -- letterdist_id and layout_id.
    kwg_id         UUID NOT NULL REFERENCES input_data(id),
    -- Games each leave-gen task plays over its forced-rack subset.
    num_iterations INT NOT NULL,
    -- How many sequential generations this job runs before it is complete.
    generation_count  INT NOT NULL DEFAULT 1 CHECK (generation_count >= 1),
    -- Per-generation occurrence target every rack must reach before the generation closes.
    target_rack_count INT NOT NULL CHECK (target_rack_count >= 1),
    -- Size of the forced-rack subset handed to a single task.
    racks_per_task    INT NOT NULL CHECK (racks_per_task >= 1),
    -- Whether the leave-generating bot plays with a wordmap. Sent to the worker,
    -- which builds one from its .kwg if it does not already have it. A player
    -- setting like any other -- workers assume nothing about wordmaps.
    use_wordmap       BOOLEAN NOT NULL DEFAULT TRUE
);

-- Exports
--
-- A completed job's results, as one gzipped NDJSON object in the artifact
-- store. Only completed jobs can be exported, and that is what makes the
-- artifact worth having: a completed job's results are immutable, so an export
-- is built once and reused, where an export of an active job would be stale as
-- it was written.
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
    error         TEXT,
    requested_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    requested_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at  TIMESTAMPTZ
);

-- The newest ready export for a job, which is what a download resolves to.
CREATE INDEX job_exports_job_idx ON job_exports (job_id, requested_at DESC);

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
    -- Denormalized counters used by SKIP LOCKED selection; avoids per-candidate join/aggregate.
    accepted_count       INT NOT NULL DEFAULT 0,
    active_claim_count   INT NOT NULL DEFAULT 0,
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
-- planner had to read every available task of the job and sort it. A job with
-- redundancy above 1 leaves tasks available until their slots fill, so that is
-- not a short list.
CREATE INDEX tasks_queue_idx   ON tasks (job_id, created_at) WHERE state = 'available';
CREATE INDEX tasks_claimed_idx ON tasks (state) WHERE state = 'claimed';

-- Individual claims (one row per worker claim; up to redundancy concurrent/cumulative rows per task)
--
-- claimed_by_user_id carries no ON DELETE clause because a user row is never
-- deleted: account deletion anonymizes it in place (users.deleted_at, and a
-- tombstone username and email) and leaves these rows exactly where they are.
-- Removing them instead would take with them the captured in-game positions
-- keyed to those claims -- including the ones other redundant claims
-- deduplicated against, which nothing else holds -- and leave-generation
-- occurrences that were folded into per-rack totals and cannot be subtracted
-- back out. See routes::admin::delete_user.

-- 'declined' is distinct from 'abandoned': one is a worker saying "I cannot do
-- this", the other is a claim that lapsed. Only the first is diagnostic.
CREATE TYPE claim_state AS ENUM ('claimed', 'completed', 'abandoned', 'declined');

CREATE TABLE task_claims (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    task_id              UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    claim_token          UUID NOT NULL,
    state                claim_state NOT NULL DEFAULT 'claimed',
    claimed_by_user_id   UUID REFERENCES users(id),
    claimed_by_anon_uuid UUID REFERENCES anonymous_workers(uuid),
    claimed_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_heartbeat_at    TIMESTAMPTZ,
    completed_at         TIMESTAMPTZ,
    -- As reported at claim time. What the fleet is actually running, which is
    -- the evidence for raising a job's floor.
    magpie_version       TEXT,
    CONSTRAINT claim_has_single_owner CHECK (
        (claimed_by_user_id IS NOT NULL)::int + (claimed_by_anon_uuid IS NOT NULL)::int = 1
    )
);

-- Prevent a single identity from filling more than one live slot on the same
-- task. 'declined' must be excluded alongside 'abandoned': a worker that
-- declined a task for missing data and then fixed its data has to be able to
-- claim that task again.
CREATE UNIQUE INDEX task_claims_user_unique_idx
    ON task_claims (task_id, claimed_by_user_id)
    WHERE state NOT IN ('abandoned', 'declined');
CREATE UNIQUE INDEX task_claims_anon_unique_idx
    ON task_claims (task_id, claimed_by_anon_uuid)
    WHERE state NOT IN ('abandoned', 'declined');

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
    -- batch of a job may be short.
    rack_start        BIGINT NOT NULL CHECK (rack_start >= 0),
    rack_count        INT NOT NULL CHECK (rack_count >= 1),
    previous_play     TEXT,                  -- GCG-encoded previous move; required when inference is enabled; NULL for opening racks
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
    -- seed is also stored on the tasks row; duplicated here for convenience when reading the full request.
    seed              BIGINT NOT NULL,
    num_games         INT NOT NULL DEFAULT 1,
    player1_config_id UUID NOT NULL REFERENCES player_configs(id),
    player2_config_id UUID NOT NULL REFERENCES player_configs(id)
);

CREATE TABLE leave_requests (
    task_id             UUID PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
    lexicon             TEXT NOT NULL,
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
    use_wordmap         BOOLEAN NOT NULL   -- denormalized from job_leave_config.use_wordmap
);

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
-- out in the lap under way. A row exists exactly while a lap has racks left to
-- hand out: the task that takes the last of them deletes it.
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
-- Read and written only under that lock. A row that goes missing (a purge, a
-- partial restore) is a lap not started, which waits for what is in flight and
-- staged before it selects anything; nothing is handed out twice.
CREATE TABLE leave_selection_cursors (
    job_id      UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    generation  INT NOT NULL,
    cursor_rack TEXT NOT NULL,
    PRIMARY KEY (job_id, generation)
);

-- Task records (one per accepted claim; keyed by task_claim_id since redundancy > 1 yields multiple results per task)
-- task_id is denormalized here for efficient job-results queries without joining through task_claims.

-- One analysed position per row, whatever produced it.
--
-- Opening rack jobs write one per rack. Games and game-pairs jobs write one per
-- turn when `capture_positions` is on: a worker analyses a position on every
-- turn anyway, and keeping those makes a job a corpus of analysed positions as
-- well as an Elo measurement.
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
    task_id         UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
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
    )
);

-- Games are seeded and deterministic, so redundant claims replay identical
-- games and would capture identical positions. Keying on the task rather than
-- the claim makes the first accepted claim the one that lands and the rest
-- no-ops, so redundancy still verifies the *result* without multiplying the
-- corpus.
CREATE UNIQUE INDEX position_analysis_records_in_game_idx
    ON position_analysis_records (task_id, game_index, turn_number)
    WHERE game_index IS NOT NULL;

-- Opening racks keep their natural key: one analysis per rack per claim, so
-- redundant claims each record their own and can be compared.
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
    -- The simulated win percentage. NULL for a static player, which ranks on
    -- equity alone and simulates nothing.
    win_percentage  DOUBLE PRECISION,
    -- Mean win%+spread blend in [0, 1] (see the player config's
    -- utility_w_winpct/utility_w_spread/utility_spread_scale), sometimes used
    -- to rank moves instead of equity or raw win percentage. NULL for a
    -- static player, same as win_percentage.
    blended_utility DOUBLE PRECISION
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
    id               BIGSERIAL PRIMARY KEY,
    move_id          BIGINT NOT NULL REFERENCES position_analysis_moves(id) ON DELETE CASCADE,
    ply              SMALLINT NOT NULL,
    bingo_percentage DOUBLE PRECISION NOT NULL,
    average_score    DOUBLE PRECISION NOT NULL,
    -- The UNIQUE above is the index the cascade from moves uses: move_id is
    -- its leading column, so there is deliberately no second index on it.
    UNIQUE (move_id, ply)
);

-- Shared by games and game pairs: one row per accepted claim, holding the
-- aggregate MAGPIE's autoplay reports. Autoplay does not emit individual games
-- -- it reports counts and score moments for a batch, and in `-gp` mode also
-- the pentanomial: how many completed pairs ended in each of the five possible
-- pair outcomes. The pentanomial is what SPRT and the rating fits read; the
-- divergent summary alongside it is a diagnostic only.
--
-- With redundancy > 1 a task has several rows here, one per accepted claim,
-- and because games are seeded and deterministic they describe the *same*
-- games. Every aggregate that treats rows as observations (SPRT, progress,
-- ratings) therefore reads one row per task -- the first accepted -- or it
-- would count each game `redundancy` times.
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
    -- This -- not the divergent subset below -- is what SPRT and the ratings
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
             AND pent_1 + 2 * pent_2 + 3 * pent_3 + 4 * pent_4 = 2 * wins + ties)
    ),

    -- The divergent subset: pairs whose two games did not play identically.
    -- Kept as a *diagnostic* -- it says how often two configs actually differ,
    -- which is worth showing -- and deliberately not used as a statistical
    -- sample. NULL for `games` jobs.
    divergent_games   INT CHECK (divergent_games >= 0),
    divergent_wins    INT CHECK (divergent_wins >= 0),
    divergent_losses  INT CHECK (divergent_losses >= 0),
    divergent_ties    INT CHECK (divergent_ties >= 0),
    CONSTRAINT game_results_divergent_all_or_nothing CHECK (
        (divergent_games IS NULL AND divergent_wins IS NULL
             AND divergent_losses IS NULL AND divergent_ties IS NULL)
        OR (divergent_games IS NOT NULL AND divergent_wins IS NOT NULL
             AND divergent_losses IS NOT NULL AND divergent_ties IS NOT NULL
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
    -- transition against fewer results, or by a rebuild under a changed
    -- klv::build. Recording the hash is what turns that from invisible into a
    -- query; the ON CONFLICT DO NOTHING on insert means the row keeps the
    -- FIRST hash, so a later mismatch is evidence rather than an overwrite.
    sha256        TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
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
-- rating can never affect whether a job stops. SPRT stays on the job config
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
    name          TEXT NOT NULL UNIQUE,
    variant       TEXT NOT NULL,
    letterdist_id UUID NOT NULL REFERENCES input_data(id),
    layout_id     UUID NOT NULL REFERENCES input_data(id),
    -- The fixed point every other rating is measured against. Ratings are only
    -- identifiable up to an additive constant, so exactly one player config
    -- must be pinned; the static bot at 2000 is the convention.
    anchor_player_config_id UUID NOT NULL REFERENCES player_configs(id),
    anchor_rating DOUBLE PRECISION NOT NULL DEFAULT 2000,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (variant, letterdist_id, layout_id, name)
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
-- active job takes a run every two minutes, and past a month a day is the
-- resolution the history chart draws at anyway. The ratings and residuals
-- below go with their run.
CREATE TABLE rating_runs (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    pool_id       UUID NOT NULL REFERENCES rating_pools(id) ON DELETE CASCADE,
    computed_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Why this run happened: 'membership' (an admin added or removed a config),
    -- 'evidence' (new results arrived), or 'manual'.
    trigger       TEXT NOT NULL,
    method        TEXT NOT NULL DEFAULT 'bradley_terry_mm',
    -- Fit provenance. A run that did not converge is still stored and still
    -- displayed, flagged: hiding it would leave the page silently stale.
    iterations    INT NOT NULL,
    converged     BOOLEAN NOT NULL,
    -- How much evidence went in, so a run can be compared to its predecessor
    -- without re-reading game_results.
    pairs_used    BIGINT NOT NULL,
    jobs_used     INT NOT NULL
);

CREATE INDEX rating_runs_pool_idx ON rating_runs (pool_id, computed_at DESC);

-- The ratings themselves: one row per player config per run. This is the only
-- table in the schema that holds a rating.
CREATE TABLE player_config_ratings (
    run_id           UUID NOT NULL REFERENCES rating_runs(id) ON DELETE CASCADE,
    player_config_id UUID NOT NULL REFERENCES player_configs(id),
    rating           DOUBLE PRECISION NOT NULL,
    -- Approximate Elo standard error. Wide bars are the honest signal that a
    -- config has barely played, or has only played opponents far from its own
    -- strength; the page shows them next to the rating for that reason.
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

-- The residuals of one fit: for every head-to-head with games in it, the score
-- the fit's ratings predict against the score that happened. Stored with the
-- run rather than recomputed on each view of the pool, which rebuilt the
-- pool's evidence matrix -- a grouped scan over every paired result it counts
-- -- on every public page view. Stored, they also describe the evidence this
-- fit used, not evidence that has moved on since.
CREATE TABLE rating_run_residuals (
    run_id               UUID NOT NULL REFERENCES rating_runs(id) ON DELETE CASCADE,
    row_player_config_id UUID NOT NULL REFERENCES player_configs(id),
    col_player_config_id UUID NOT NULL REFERENCES player_configs(id),
    pairs                DOUBLE PRECISION NOT NULL,
    actual               DOUBLE PRECISION NOT NULL,  -- the row config's score rate
    predicted            DOUBLE PRECISION NOT NULL,
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
-- Completed claims by time. The ETA (`jobstats::estimate_eta`, on every
-- detail view and live push) and the job list's `stalled` flag both ask
-- "how many of this job's claims completed in the last hour / day", and
-- task_claims has no job column, so the alternative plan walks every task of
-- the job and every claim of each -- the job's whole history, for a question
-- about its last hour. Through this index the scan is bounded by the fleet's
-- recent completions instead, whatever the job's age.
CREATE INDEX        task_claims_completed_idx ON task_claims (completed_at DESC)
    WHERE state = 'completed';
CREATE INDEX        task_claims_user_idx      ON task_claims (claimed_by_user_id);
CREATE INDEX        task_claims_anon_idx      ON task_claims (claimed_by_anon_uuid);
-- (job_id, state), not job_id alone: the job list counts a job's tasks and its
-- completed tasks for every job on the page, and with state in the index both
-- are index-only rather than a heap visit per task.
CREATE INDEX        tasks_job_idx             ON tasks (job_id, state);
-- (task_id, submitted_at) rather than task_id alone: the per-task "first
-- accepted result" read that every aggregate uses orders on both.
CREATE INDEX        game_results_task_idx     ON game_results (task_id, submitted_at);
-- The public results feed for a games or game-pairs job, keyset-paginated like
-- the opening-rack one. `task_claim_id` is the primary key and so the
-- tiebreaker, since `game_results` has no serial.
CREATE INDEX        game_results_feed_idx
    ON game_results (job_id, submitted_at DESC, task_claim_id DESC);
CREATE INDEX        leave_records_task_idx    ON leave_records (task_id);
CREATE INDEX        position_records_task_idx ON position_analysis_records (task_id);
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

## Possible Future Improvements (from fishtest)

### Worker Client Features

- **Fleet mode** — a flag that makes the worker exit cleanly on error or empty queue, enabling orchestrators (systemd, Docker, CI) to manage its lifecycle.
- **Global artifact cache** — multiple workers on the same machine or network share downloaded dictionaries and bot binaries rather than each fetching independently.
- **Hardware-aware binary selection** — workers report CPU capabilities and download or compile the appropriate binary variant for their architecture.

### Configuration

- **Per-job-type (or per-job) heartbeat timeout** — the heartbeat timeout window is a single global constant for v1. A future improvement could make it configurable per job type or per individual job.
- **Automatic database password rotation** — the master password is set by hand and rotated by runbook, because every other secret is handled that way and the tasks read a fixed `DATABASE_URL` from SSM. Turning RDS-managed rotation back on means injecting `DB_PASSWORD` from the managed secret (the backend can already assemble its URL from `DB_*` parts), teaching `backup.sh` and `restore-drill.sh` the same parts, and an EventBridge rule that forces a new ECS deployment on each rotation — with a short window after each one where new connections fail until tasks restart. Having the process re-read the secret itself would close that window but adds a Secrets Manager code path the design deliberately avoids.

### Scaling

- **Primary/secondary server split** — one instance owns task scheduling and mutations; read-only instances serve the dashboard. Eliminates concurrent scheduling conflicts under high worker load. Note that several things assume a single instance today and would have to move first: staged imports and their reaper, the in-process rate limiters, and the per-process SSE broadcaster (`desired_count` is validated to 1 for that reason).
- **Debounced live stats** — done: the finish condition is checked on every eighth submission (plus whenever nothing is left in flight), and the SSE push is coalesced per job and spaced at least a second apart. What remains is moving the payload's own aggregates to a background refresh, which `compute`'s slow-query log line exists to decide on.

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
| RDS Postgres | Everything in the [Schema](#schema): users, API key hashes, jobs, player configs, tasks, claims, results, `input_data.content`, audit log | **System of record** |
| S3 artifact bucket | Per-generation KLVs under `leaves/{job_id}/generation-{n}.klv2` | **Derivable** |
| SSM Parameter Store | `/birdtest/DATABASE_URL` (which carries the hand-set RDS master password), `/birdtest/SESSION_SIGNING_KEY` | **Secret, unmanaged** |

Within Postgres the rows differ enormously in how replaceable they are, which is
what makes selective restore worth building rather than only whole-database
rollback:

- **Irreplaceable.** `game_results`, `position_analysis_records` / `_moves` /
  `_plies`, `leave_rack_progress` together with `leave_rack_staging` (the
  accepted leave results not yet merged into it), `leave_records`, `rating_pools`,
  `rating_pool_members`, `task_claims`, `audit_log`. This is donated compute. A
  contributor is not going to run the same 40,000 game pairs again because we
  lost them, and the SPRT state derived from them cannot be recomputed from
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
   result, rating and progress row for a job in one transaction, and `delete_job` /
   `delete_user` are similarly total. There is no confirmation dialogue in the API
   layer and no undo. This is the most likely way birdtest loses contributor work,
   and it is the scenario that most demands *selective* restore: the rest of the
   database has moved on and must not be rolled back.
2. **A bad migration.** Until release the schema is a single file edited in place,
   and the documented reset is `DROP SCHEMA public CASCADE`. That command against
   the wrong `DATABASE_URL` is a total loss with no application-level trace.
3. **Instance or AZ failure.** RDS is single-AZ by default; an instance failure is
   an outage and a restore.
4. **Region loss.** Unlikely, survivable only if backups already left the region.
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
  pg/2026-09-07T03:00:00Z/
    manifest.json
    dump/                     # pg_dump -Fd output, one file per table
  pg/2026-09-07T03:00:00Z.manifest.json   # duplicated at top level for cheap listing
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

`migration_checksums` and `backend_image` together answer "what code can read this
dump", which under a single mutable `0001_initial.sql` is otherwise unanswerable.
`table_row_counts` is what a restore is verified against, and what makes a
silently-truncated dump detectable without restoring it.

#### Encryption and secrets

The dump contains argon2 password hashes, API key hashes, email addresses and
unexpired reset-token hashes. It is encrypted with SSE-KMS using a customer-managed
key whose policy grants `Decrypt` only to the restore role, **not** to the backup
task — the task needs `GenerateDataKey` and `Encrypt` to write, and nothing more. A
compromised backup task can then create backups but not read old ones.

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
`leave_generation_artifacts.sha256` and an admin-triggered rebuild path.** The three
work together: replication handles object loss, the checksum turns "was this
artifact corrupted or overwritten" into a query, and the rebuild path is what a
restore uses when a DB restored to time *T* references keys that no longer exist.

Two details a rebuild has to respect:

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
   coarse; instead the backup task emits a `birdtest/backup SuccessTimestamp`
   custom metric and the alarm fires on `missing data` for > 36 hours. Catches both
   crashes and a schedule that silently stopped.
3. **In-app surface.** An admin page listing recent backups.

Layer 3 had a design choice of its own. Having the backend list the backup bucket
directly would require giving the task role `ListBucket` / `GetObject` on it,
weakening the isolation the separate bucket exists to create. Instead **the backup
task writes a `backups` row into Postgres when it finishes**: the backend reads its
own database and needs no new S3 permission, and the row is exactly the manifest. A
restored database also restores the backup history, which is confusing but harmless
if `restored_at` context is displayed. Failed runs insert a row with `ok = false` so
the admin page shows the failure rather than a gap. The table is insert-only and
nothing in the request path reads it.

A related, cheap safety feature belongs in the same phase and is built:
`purge_job`, `delete_job` and `delete_user` write the counts of what they are about
to destroy into `audit_log` *before* destroying it. Restoring is far easier when the
log says what was lost.

### Restore

| Scenario | Mechanism | Data loss |
|---|---|---|
| Instance/AZ failure | RDS PITR restore to new instance, repoint `DATABASE_URL` | ≤ 5 min |
| Bad migration / dropped schema | RDS PITR to just before the statement | ≤ 5 min |
| Mistaken purge/delete of one job or user | Restore latest dump into a **scratch** instance, extract, re-insert | Whatever arrived after the last dump, for those rows only |
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
instance. Confirm `deletion_protection` and `backup_retention_period` carried over:
a restored instance does **not** inherit automated-backup settings by default, and a
restore that leaves the new instance unbacked is a trap. The alternative shape —
restore and *swap identifiers* so the endpoint is unchanged — avoids touching SSM but
requires renaming the damaged instance first and is slower under pressure; prefer
repointing SSM.

**Selective restore** is the common case, and the database must not be rolled back
because everything else has moved on. Restore into a scratch database, extract the
affected rows in dependency order, re-insert into production inside one transaction
with `ON CONFLICT DO NOTHING` throughout so a partial re-run is safe, then repair
the denormalized counters — which is the part a naive row copy gets wrong.
`tasks.accepted_count` and `active_claim_count` must be recomputed from the restored
`task_claims`, and `tasks.state` / `completed_at` recomputed against the job's
redundancy. `purge_job` deletes tasks precisely so they regenerate cleanly; a
restore that puts claims back without their counters leaves the scheduler
dispatching work that is already done. Finally, recompute what is not a simple copy:
the job's SPRT verdict, and the rating pools (a refit, from data that is already
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
than the last schema edit restorable only with archaeology.

**Verifying a restore.** Do not declare one finished on "the page loads":

- `SELECT COUNT(*)` per major table, against the manifest's `table_row_counts` (for
  a dump restore) or pre-incident dashboard figures (for PITR).
- Referential sanity: no `leave_generation_artifacts` row whose key 404s through the
  worker artifact endpoint; no job whose `letterdist_id` or `layout_id` is missing
  from `input_data`; no `input_data` row with `role IN ('letterdist','layout')` and
  `content IS NULL`.
- Counter sanity: `tasks.accepted_count` and `active_claim_count` agree with
  `task_claims`; no `tasks.state = 'completed'` with insufficient accepted claims.
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
*Automated, monthly*: a scheduled task restores the latest dump, runs the row-count
and referential checks, writes a result row, and tears the database down. This is
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
  `password_hash` with a known throwaway argon2 hash, and truncates `api_keys`,
  `email_confirmations` and `password_reset_tokens`. Restoring production data
  locally without it is documented as something not to do.

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
that brings up `docker compose`, seeds a row in every table a result touches with
plain SQL, dumps, drops, restores and asserts the verification checks pass.
(`scripts/restore-roundtrip.sh` seeds directly rather than through a worker: what
it is testing is `pg_dump`/`pg_restore`, and going through the worker API would
add a client to the failure surface without adding a row shape.)

#### Where the implementation differed from the plan

- The **backup bucket is replicated cross-region too**, not just the artifact bucket.
  The 24-hour region-loss RPO is not met by replicating only derivable data, and the
  marginal cost of a second replication rule is a KMS key in the DR region.
- The **restore drill restores into a second database on the production instance**
  rather than provisioning one. It keeps the drill a shell script instead of an
  orchestration; the cost is transient storage, which `max_allocated_storage` (5×
  allocated) covers. `restore_drill_enabled = false` turns it off if that becomes
  tight.
- **`multi_az` is a variable defaulting to false**, not a change. It doubles the
  instance cost, and it is availability rather than backup — the call belongs to
  whoever pays for it.
- The **round-trip test is a script, not CI**: `.github/workflows/ci.yml` runs the
  per-pull-request tiers only, and the round trip belongs with the nightly jobs
  TESTING.md describes, which do not exist yet.
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
   work. Stopping first costs a few seconds with nothing serving, against an
   invariant that otherwise does not hold exactly when the code changes. The
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

**Docker, and nothing else** — for everything except a worker doing real
computation. The backend, frontend, Postgres and the S3 stand-in all run as
containers, so no Rust, Node, Python or Postgres install is required on the
host.

| Tool | Used for |
|---|---|
| Docker / Docker Compose | The entire stack |

The compose stack needs a MAGPIE build: the backend runs one for every derived
file and every leave-generation KLV, and refuses to start without it. Point
`MAGPIE_BIN` at a local checkout's `bin/magpie` (`make magpie
BUILD=portable_release`). The same checkout is what runs work against the
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

`docker compose down -v` also works but discards the MinIO bucket with it.

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
docker compose --profile fake-worker up  # end-to-end suite only: synthetic results
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

From there, use the now-admin account's session to create a player config and a job through `/admin/player-configs/new` and `/admin/jobs/new` (or the equivalent `POST /api/admin/...` calls directly), then activate the job with an allocation via `/api/admin/jobs/:id/activate`. Once a job is active, `magpie contribute` (step 5) will start claiming and completing real tasks against it, and the dashboard at `http://localhost:5173/jobs/:id` updates live via SSE — this is the fastest way to confirm a full change (backend, frontend, and worker together) actually works end to end.

### Running the checks

```bash
cd backend  && cargo test --lib --bins   # unit and contract tests; no database needed
cd backend  && TEST_DATABASE_URL=postgres://birdtest:birdtest@localhost:5432/birdtest \
               cargo test                # plus tiers 2-3 in backend/tests/
cd backend  && cargo clippy --all-targets
cd frontend && npm run check             # svelte-check against the TypeScript config
```

[TESTING.md](TESTING.md) is the full picture: six tiers, what each may touch,
the shared seed and fixture data, and how the development environment is the
end-to-end suite with its assertions and teardown removed.
