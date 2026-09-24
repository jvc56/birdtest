# AUDIT_FINDINGS_7 — the eleventh audit (2026-09-24)

Branch `audit/birdtest-2026-09-24`, off `main` at `5d35525`. MAGPIE changes are
on `birdtest-contribute` at `68a74611` (not yet pushed; `docker/Dockerfile`
pins it, so it must be pushed before an image is built).

**Builds on** the ten earlier audits: `AUDIT_FINDINGS.md` and
`AUDIT_FINDINGS_1.md` … `AUDIT_FINDINGS_6.md` (branches
`audit/birdtest-2026-09-11` … `audit/birdtest-2026-09-17-pass2`), whose records
were folded into PLAN.md and deleted at `d5ed19a`. Their accepted limits are
PLAN.md's [Known Limits and Open Questions](PLAN.md#known-limits-and-open-questions);
nothing below re-litigates one of them except where it says so.

**Count: 27 code wins (code changed to match the plan, or both changed where
the plan's claim was false), 9 plan wins (PLAN.md, TESTING.md, RUNBOOK.md or
README.md changed to match the code), 4 unresolved pending feedback.**

How the audit was run: five parallel read-only reviews (dispatch and
concurrency; statistics, ratings, auth and the frontend; MAGPIE argument
coverage and the `birdtest-contribute` branch; schema, storage and
performance; infrastructure, CI and the documents-as-contract), each finding
then re-verified against the code by hand before anything was changed. Two
reviewer claims did not survive verification and are recorded as such.

---

## 1. Code versus PLAN.md: every discrepancy and its decision

Each entry: what the code did, what the plan (or a contract document) said, the
decision, and why.

### 1.1 A leave task played the previous generation's KLV (MAGPIE) — **code updated**

- **Code:** `config_contribute_leave_generation` writes the fetched KLV to a
  fixed name, `<lexicon>_birdtest_previous`, and loads it by that name.
  `players_data_set` keeps any data of the requested name already in memory
  (`get_index_of_existing_data` matches on name only). From the second leave
  task in a process the name was already loaded, so the task played the
  previous task's KLV; `autoplay`'s end-of-run `players_data_reload` re-read the
  old file only before it was overwritten.
- **PLAN.md** ("Every setting that can change a result"): "a leave task gives
  both [seats] the fetched KLV."
- **Decision:** code updated. The plan states the only correct behaviour. Every
  worker's first task after each generation boundary merged a generation's
  worth of statistics played with stale leaves, undetectably. The same cache
  also defeated the digest checks for a wordmap or rack info table rebuilt
  mid-run and for a lexicon updated mid-run.
- **Fix:** `config_contribute_evict_changed_data` (MAGPIE `config.c`) records
  each loaded KWG/KLV/WMP/RIT's file identity (`get_file_identity`: size,
  inode, mtime and ctime to the nanosecond, moved from `contribute.c` to
  `io_util.c`) after every contribute load and, before the next, evicts
  (`players_data_evict`) any whose file changed or which this path did not load
  itself. Test: `test_a_rewritten_klv_is_read_again` (fails without the fix,
  confirmed by disabling it).

### 1.2 A `429` slept for hours (MAGPIE) — **code updated**

- **Code:** `chttp.c` declared `CURLINFO_RETRY_AFTER = 6291508`, which is
  `CURLINFO_OFF_T + 52` — `CURLINFO_CONNECT_TIME_T`, the connect time in
  microseconds. `http_client.c` slept that many seconds on a `429`, up to five
  times, and the heartbeat's single-shot request used the same loop, so
  `heartbeat_stop` (and the submission) hung behind it.
- **PLAN.md** (retry table): "429 — sleep `Retry-After` (default 1s) and
  retry, up to 5 times."
- **Decision:** code updated. Constant corrected to `+ 57` (6291513, checked
  against every other constant in the file); waits clamped to 60 s
  (`http_client_rate_limit_wait_seconds`, tested); the single-shot request no
  longer retries a `429`. PLAN's table updated to say all three.

### 1.3 Result-changing MAGPIE commits did not move the version — **code and PLAN.md updated**

- **Code:** `MAGPIE_VERSION` was `0.1.0` from 2026-09-16 on, across `dfc79f1a`
  (a capturing static player played its worst move — results change) and the
  two fixes above. `MIN_MAGPIE_VERSION` defaulted to `0.1.0` everywhere.
- **PLAN.md:** "the version moves only when a release changes what a task
  computes, and the floor moves with it."
- **Decision:** code updated to honour the rule: `MAGPIE_VERSION` 0.1.1; every
  default of the floor (config, the `min_magpie_patch` column default, compose,
  both env examples, Terraform, the tier-5/6 harnesses) 0.1.1; contract
  fixtures (both repos) and two tests that encoded 0.1.0. PLAN.md's
  "pre-release version" paragraph rewritten: the version moves with every
  result-changing change, not only at a release, since the floor is the only
  lever against a known-bad build.

### 1.4 The HTTP timeout bounded the whole exchange (MAGPIE) — **code updated**

- **Code:** `CURLOPT_TIMEOUT` 120 s — upload, server processing and download
  together — against results the server accepts up to 64 MiB. Below ~4.5 Mb/s
  of uplink the largest could not be sent; each of 20 retries re-uploaded it.
- **PLAN.md:** described MAGPIE's "120-second request timeout" as the bound the
  ALB's 300 s idle timeout had to exceed; nothing about large results.
- **Decision:** code updated: 120 s connect timeout plus stall detection
  (`CURLOPT_LOW_SPEED_LIMIT` 1 B/s over 120 s) under a one-hour ceiling;
  `CURLOPT_POSTREDIR` so a redirected `POST` stays a `POST`. PLAN.md updated
  ("A request fails on a stall, not on its length").

### 1.5 Request names were not all validated (MAGPIE) — **code updated**

- **Code:** only the top-level lexicon, variant, distribution and layout were
  checked. Player lexicon/leaves/win% model, `rit_name` (which the worker may
  *build*, 1.9 GB) and `expected_data` file names went straight into paths.
- **Plan / contract:** MAGPIE's own comment: "Every field of a task request is
  untrusted: it becomes file paths."
- **Decision:** code updated: `data_filepaths_is_safe_name` (one or two runs of
  `[A-Za-z0-9_-]` joined by a single `.`), applied to every player object's
  names in `config_contribute_validate_common` and to `expected_data` names
  (an unsafe one is treated as a missing file, never resolved or hashed).
  Tested in `test_a_request_must_state_its_distribution_and_layout`.

### 1.6 Purging a completed job stranded it — **code updated**

- **Code:** `purge_job` left `status` alone; `activate_job` refuses completed
  jobs; so a purged completed job was empty and could never run again.
- **PLAN.md / UI:** a purge "regenerates [tasks] from the start of its space at
  the next claim"; the admin page said "the job starts over."
- **Decision:** code updated: a purge sets a completed job to `inactive` (and
  clears the stored SPRT verdict, 1.13). Active/inactive jobs keep their state.

### 1.7 A purge or delete stalled the fleet — **code updated** (race)

- **Code:** `purge_job`/`delete_job` lock every open claim (`lock_open_claims`)
  for the whole of their deletes. The heartbeat (`UPDATE … WHERE claim_token`),
  submission and decline (`FOR UPDATE OF c`) and the reclaim statement all
  waited on those locks with no timeout, each holding one of twenty pool
  connections. MAGPIE heartbeats every 30 s, so a purge with ~20 live workers
  exhausted the pool within one cycle; every claim, submission and admin
  request on the server then failed at the 30 s acquire timeout until the purge
  committed. Two reclaims locking overlapping claims in different orders could
  also deadlock.
- **PLAN.md:** described the lock order as preventing a *purge* deadlock, and
  nothing about the rest of the fleet.
- **Decision:** code updated: the heartbeat updates `WHERE id = (SELECT … FOR
  UPDATE SKIP LOCKED)`; reclamation selects lapsed claims `FOR UPDATE OF c SKIP
  LOCKED` in a CTE and updates by id; submission and decline bound their claim
  lookup with `SET LOCAL lock_timeout = '5s'` (reset after), and `55P03` now
  maps to `503` with `Retry-After: 5` (MAGPIE retries 5xx; its retry after the
  purge gets `accepted: false`). PLAN.md's lifecycle and Workflow step 3
  updated. The purge's own length is item 3.2 (unresolved).

### 1.8 The bounded dispatch-lock wait still held the pool — **code updated** (critical path)

- **Code:** a claim for a job whose dispatch lock was held waited up to 2 s
  (`DISPATCH_LOCK_WAIT_MS`) *on a pool connection*. A leave universe's seeding
  holds that lock for tens of seconds, and a job handing out nothing falls
  behind its share and heads every candidate list — so a fleet of idle workers
  polling every 5 s held the pool on it.
- **PLAN.md:** "The bound turns that into those workers being told to look
  elsewhere" — true of the answer, not of the cost.
- **Decision:** code updated: `jobs::DispatchHolds`, an in-process set (the
  service is validated to one instance) that seeding, purge and delete mark
  while they hold the lock; `try_claim_from_job` skips a marked job before
  taking a connection. PLAN.md updated.

### 1.9 Expired exports were still served — **code updated**

- **Code:** `exports::newest_ready` had no age limit; the bucket's lifecycle
  rule deletes `exports/` after 30 days. A completed job's admin stream then
  `303`-redirected to a deleted object every time (never falling back to the
  scan), and the admin page offered a dead download.
- **PLAN.md:** "they expire from the bucket after 30 days."
- **Decision:** code updated: `EXPORT_LIFETIME_DAYS = 29`, filtered in
  `newest_ready`; the admin detail reports `expired`; the Terraform rule and the
  constant cross-reference each other. PLAN.md updated.

### 1.10 Catalan captured positions were refused — **code updated**

- **Code:** `check_rack` counted characters; MAGPIE writes multi-character
  letters bracketed (`ld_ml_to_hl`: `[L·L]`, `[NY]`, `[QU]`), so a full
  Catalan rack with one was 10–11 "tiles" and every capture batch of a Catalan
  games job (re-enabled by `0ac7a57`) was a 400.
- **PLAN.md:** "a rack with eight tiles" is impossible — tiles, not characters.
- **Decision:** code updated: `plausibility::rack_tiles` counts a bracketed
  group as one tile and refuses unclosed/empty brackets; used for captured
  positions, opening racks and leave occurrences.

### 1.11 Per-ply statistics were unbounded and untruncated — **code updated**

- **Code:** `check_moves` ignored `plies`; `insert_position_analyses` stored
  every ply reported, though moves were truncated to `num_plays_recorded`.
  `num_moves = -1` also passed (`as usize`).
- **PLAN.md:** "Per-ply statistics pair the same way: `num_plies_recorded`
  against `plies`."
- **Decision:** code updated: plies validated (numbered from 0 ascending, bingo
  % in [0,100], finite non-negative average score) and stored only below
  `num_plies_recorded` (the larger of the two players' for a captured
  position); negative `num_moves` refused.

### 1.12 Impossible pentanomials passed — **code updated; PLAN.md extended**

- **Code:** checked bounds, pair count and half-points; `[0,1,0,1,0]` with
  W2 L2 T0 passed although buckets 1 and 3 each need a draw.
- **PLAN.md:** the cross-checks were described as pair count and half-points.
- **Decision:** code updated with the missing identity (ties = p1 + p3 + an
  even number ≤ 2·p2; wins and losses then follow from the half-point check);
  PLAN.md's "What a submission has to satisfy" gains the rule.

### 1.13 The SPRT verdict that completed a job was not recorded — **code updated**

- **Code:** `complete_unless_purged` set `completed` only; the page recomputed
  SPRT from every accepted result, including those in flight at completion,
  so a job that passed at LLR 2.96 could read "running — LLR 2.80" once they
  landed, with no record of the decision.
- **PLAN.md (Known Limits):** late results are "harmless to SPRT (already
  decided)" — but nothing held the decision.
- **Decision:** code updated: `jobs.sprt_decided_status/_llr/_units`, written
  by the finish check with the completion, cleared by a purge, returned as
  `games.decided` and shown as the result on both job pages with the live LLR
  beside it. Tested (`I-STATS-9b`).

### 1.14 The rating sweep missed membership-only changes; fits were stamped out of order — **code updated**

- **Code:** membership routes commit, then refit separately; the sweep compared
  only `pairs_used`, so a dropped refit after adding/removing a config with no
  pairs was never repaired. `rating_runs.computed_at` defaulted to the
  transaction's start, before the pool lock.
- **PLAN.md:** the lock exists so "the newest run" is the latest fit.
- **Decision:** code updated: the sweep compares the last run's rated members
  as well; `computed_at = clock_timestamp()` under the lock. Tested
  (`I-RATE-9b`). PLAN.md updated.

### 1.15 The non-transitivity banner fired on noise — **code updated**

- **Code:** three residuals ≥ 5 points raised it whatever the pairs behind them.
- **PLAN.md:** "says so when the residuals are large enough that the ranking
  should not be read as one."
- **Decision:** code updated: a residual counts only at |z| ≥ 3 against
  `sqrt(p(1−p)/pairs)` (p clamped to [0.01, 0.99]). Tested (`F-CHART-6`).

### 1.16 An unconfirmed account was a permanent dead end — **code updated**

- **Code:** no resend route; reset needs a confirmed address; re-registration
  of the address took the taken-email branch and told the owner to sign in or
  reset — both impossible. Anyone could squat an address.
- **PLAN.md:** the taken-email notice "points them at login and password reset."
- **Decision:** code updated: registration deletes an unconfirmed, non-admin
  account naming the same username or address whose confirmation has expired;
  the notice for a still-waiting account points at the confirmation link; the
  notice is rate limited per address (skipped, not refused, so the response
  stays identical). Tested (`A-AUTH-4b`). PLAN.md updated.

### 1.17 The login limiter let one address lock any account out — **code updated; PLAN.md updated**

- **Code / PLAN.md:** 10 attempts a minute per username from anywhere, checked
  before verifying — one wrong guess every six seconds kept an admin (whose name
  `GET /api/users` publishes) out.
- **Decision:** a genuine trade-off (distributed guessing vs lockout) with a
  clearly better point: per IP 10/min, per (username, IP) 10/min, per username
  100/min. Code and PLAN.md updated; A-BOUND-2 redefined and retested.

### 1.18 The restore drill restored into production — **code and PLAN.md updated**

- **Code:** `restore-drill.sh` created a database on the production instance.
- **PLAN.md:** justified by "`max_allocated_storage` (5×) covers" it — which
  RDS autoscaling does not reliably do (acts only after 5 min under 10% free,
  one step, six-hour cooldown, never shrinks).
- **Decision:** code updated: the drill starts its own Postgres in the task
  (`DRILL_TARGET=local`, the default; `server` keeps the old mode); the drill
  task no longer holds `DATABASE_URL`. Verified with
  `scripts/backup-drill-check.sh`. PLAN.md's rationale replaced.

### 1.19 Nginx could not start on Fargate — **code updated** (deployment blocker)

- **Code:** `proxy_pass http://backend:8080` resolves at load; on Fargate
  containers share `localhost` and `backend` resolves to nothing, so the
  essential frontend container exited and the task never started.
- **PLAN.md:** "Two containers share a single ECS task definition."
- **Decision:** code updated: `docker/default.conf.template` with
  `${BACKEND_UPSTREAM}` (image default `backend:8080`, ECS sets
  `127.0.0.1:8080`, `NGINX_ENVSUBST_FILTER` confines substitution to it).
  Security headers added on the SPA location.

### 1.20 RUNBOOK's SQL could reach nothing — **infra, scripts and RUNBOOK updated**

- **Code:** RDS is private and admits only the service SG; no bastion, no ECS
  Exec, no psql in the backend image. README made the first admin with
  `docker compose exec … psql`; RUNBOOK ran `psql "$DATABASE_URL"` throughout.
- **Decision:** infra updated (`infra/ops.tf`: an ops task definition — postgres
  image, `DATABASE_URL`, read access to backups, ECS Exec) with
  `scripts/prod-sql.sh` and `scripts/prod-shell.sh`; README and RUNBOOK say
  where SQL runs. Not exercised (needs AWS).

### 1.21 RUNBOOK §1 (PITR) could not run as written — **RUNBOOK and infra updated**

- Missing `db_security_group_id` output (the fallback passed `sg-XXXX`); no
  parameter group (the default group loses the WAL settings); the restored
  instance left outside Terraform, so a later apply would create an empty
  `birdtest`. **Decision:** outputs added; the source's parameter group read
  and passed; the procedure renames damaged→`birdtest-damaged-*` and
  restored→`birdtest`, then `terraform apply` restores Multi-AZ, backup window
  and tag copying.

### 1.22 RUNBOOK §2 (selective restore) silently lost the restored rows — **RUNBOOK updated**

- A purge leaves the job active; it regenerates seeds and generation-1 rows the
  restore needs, and `ON CONFLICT DO NOTHING` then drops the restored ones.
  **Decision:** new §2.0 (deactivate; delete what was regenerated); §2.1's
  scratch restore runs in the ops shell; the plies sequence line removed
  (1.27); §2.4's activate call given its body and CSRF requirement.

### 1.23 RUNBOOK §5 (region loss) could not run — **infra and RUNBOOK updated**

- `azs` defaulted to us-east-1; bucket names (global) and IAM role names
  (account-wide) collided with the lost region's and the DR replicas; the ACM
  certificate is regional; local state described the lost region.
  **Decision:** `name_suffix` variable (default empty, validated) feeding
  `local.name`; §5 rewritten (own workspace, azs, suffix, a live `dr_region`, a
  regional certificate, SES production access again).

### 1.24 The task role lacked S3 actions the code calls — **infra updated**

- `delete_object` (purging a job's exports) and `abort_multipart_upload`
  (failed streams) were AccessDenied, logged as warnings, and invisible locally
  (MinIO root credentials). **Decision:** `s3:DeleteObject` on `exports/*` only;
  `s3:AbortMultipartUpload` on the bucket.

### 1.25 Placeholder mail and URL variables — **infra and README updated**

- `public_url`, `ses_domain`, `mail_from_address` defaulted to
  `birdtest.example`; SES sandbox was undocumented. **Decision:** required and
  validated (https origin; no `.example`); README "Deploying" covers SES
  production access, image build and push, and the MAGPIE push-before-build
  order. `terraform validate` was not run (Terraform is not installed here);
  CI's `terraform validate` will check the syntax.

### 1.26 A wrong `derived_builder_image` ran a web server — **infra updated**

- The builder task set no `command`, so the backend image's `CMD` (the server,
  which never exits) ran every five minutes and its startup reapers failed the
  live server's exports. **Decision:** `command = ["build-derived"]` (a wrong
  image now fails at once) and a non-empty validation.

### 1.27 Storage: an unread surrogate key, a redundant foreign key, an unused index — **code updated**

- `position_analysis_plies.id` (BIGSERIAL PK beside `UNIQUE (move_id, ply)`)
  was read by nothing — ~30 B/row, 2–5 GB per simming opening-rack job; now
  `PRIMARY KEY (move_id, ply)`. `position_analysis_records.task_id`'s foreign
  key and `position_records_task_idx` served only a cascade the claim and job
  cascades already cover — dropped (column kept for the dedup index).
  `tasks_claimed_idx`, which PLAN's Known Limits said to drop "the next time
  the index list is touched", dropped. PLAN.md's schema copy regenerated from
  the migration (verified identical).

### 1.28 Plan wins (documents changed to match the code)

1. **TESTING.md** said CI runs `cargo test --locked`; it runs `cargo nextest
   run --locked --no-fail-fast` and `cargo test --doc`. Updated.
2. **TESTING.md** said nothing in CI runs the opt-in MAGPIE tests; the nightly
   does. Updated (and the nightly list).
3. **`config.rs`** said RDS manages and rotates the password in Secrets
   Manager; `rds.tf` sets it by hand into SSM. Comment corrected.
4. **README** said Docker is the only host dependency; the backend needs a
   mounted MAGPIE build. Corrected.
5. **`scripts/dev.py`** suggested `make magpie` (ASan dev build the container
   cannot run); now `BUILD=portable_release` and the glibc constraint.
6. **PLAN.md, retry and heartbeat sections** now describe what MAGPIE does with
   a `429` (1.2).
7. **PLAN.md, "What a submission has to satisfy"** now describes decoding
   straight from `RawValue` (3.3 below).
8. **PLAN.md, Known Limits, "Milliseconds deliberately left"** now describes
   the job row updated once, last (3.4 below).
9. **PLAN.md, Known Limits** gains the finish check's placement as a decided
   limit (item 4.1), and the rating-pool edit/delete gap.

---

## 2. Objective 3 — every MAGPIE setting that can change a task's outcome

Traced through `backend/src/jobs/{handler,mod,racks,leave_gen}.rs` and MAGPIE's
`config.c`/`contribute.c`/`autoplay.c`. PLAN.md's table ("Every setting that
can change a result") was confirmed row by row; one row was wrong (1.1) and one
row added (lexical data already in memory).

| Setting | Status |
|---|---|
| Seed (all job types) | Pinned per task; opening rack `seed + i` |
| Batch size / rack range / forced racks | Pinned |
| Lexicon, leaves per player | Pinned, digest-verified; **now also re-read when the file changed (1.1)** |
| Leave-gen KLV | Fetched per task; **was stale from the second task (fixed, 1.1)** |
| Letter distribution, layout, variant | Required, digest-verified |
| Bingo bonus, sim cutoff | Required |
| Recorder, sort, plies, plays, recorded counts, movegen margin | Required, reset first |
| Iterations, min iterations, stop %, time limit, threshold, sampling, inference + margin, utility weights | Required of simmers, reset first |
| Win% model | Pinned, digest-verified |
| Wordmap, rack info table (and its name) | Pinned by built hash; **names now validated (1.5)** |
| Word info table | Forced off |
| Multi-threading mode, small plays, heat map, print interval, leavegen cap | Reset to constants |
| PlayChooser, overtime, endgame/PEG | PlayChooser forced off; the rest unreachable. The unused `ENDGAME_*`/`PRE_ENDGAME_*`/`PLAY_CHOOSER_TIME_SECS` keys (which looked pinned and were not) removed |
| Game pairs | Pinned for games; harmless leftover in leave tasks |
| Challenge rules/bonus | Unused by autoplay |
| First player / per-game seeds | From the seed PRNG |
| Threads | Contributor's own — accepted limit, not re-litigated |
| `contribute.txt` keys | None besides threads affect results |

---

## 3. Objectives 4 and 5 — critical path and performance

### 3.1 Performance issues, most severe first

1. **A purge/delete stalled every request on the server** (minutes, whole
   fleet) — fixed (1.7, 1.8); the purge's own length remains (4.2).
2. **Dispatch-lock waits held the pool during seeding** (~2 s × claim rate of
   held connections; ~50 idle workers saturate 20) — fixed (1.8).
3. **Result bodies parsed into `serde_json::Value`** (10–19× the body in
   memory; a 64 MiB result ≈ 0.6–1 GB on a 2 GB task; a few at once → OOM,
   a 7–8 minute outage) — fixed: `ResultBody.result` is `Box<RawValue>`,
   decoded once into the typed response on the blocking pool (~2× the body).
4. **The restore drill loaded production** (hours of CPU/I-O credits, storage
   ratchet, possible STORAGE_FULL) — fixed (1.18).
5. **The submit path took the job row early** (claims for the job queued on
   the whole submit transaction) — fixed: `store_result` returns a
   `ProgressDelta`; one `UPDATE jobs` is the transaction's last statement.
6. **The nightly backup's `count(*)` is a second full read** (low) — flagged
   in Known Limits, not changed (it defines what the manifest certifies).

### 3.2 Critical-path changes

- Moved off the waits: heartbeat never waits on a claim lock; reclaim skips
  locked claims; claims skip jobs held by seeding/purge/delete without a
  connection; the job-row counter update moved to the end of the submit
  transaction.
- Considered and **not** moved: the finish check (it decides whether the job
  keeps dispatching; the saving is milliseconds; moving it needs its own
  coalescing) — recorded in Known Limits. The live stats push was already
  spawned and coalesced. Contributor counters stay inline (drift on a crash).

---

## 4. Unresolved, pending human feedback

### 4.1 A retried claim whose response was lost is a second claim

`scheduler` commits a claim before the response is written; MAGPIE retries a
claim without limit, so a dropped response (connection reset, ALB `502`) hands
the worker a second task while the first holds its slot until the heartbeat
timeout — the same idle as a dead worker at a leave lap's end or a games job's
cap. **Options:** (a) a per-logical-claim `request_id` from MAGPIE, stored on
`task_claims` under a unique partial index, with a retry answered the existing
claim's assignment (rebuilt with `load_request`); (b) leave it. **Trade-off:**
(a) is a wire change on both sides for a rare trigger whose cost is bounded by
the heartbeat timeout. Recorded in Known Limits as open.

### 4.2 A purge or delete of a large job is one request, one transaction

With 1.7/1.8 it no longer stalls anything else, but a full simming
opening-rack job's cascades are tens of millions of rows: minutes on the
production instance, past the ALB's 300 s idle timeout, after which the request
is dropped and everything rolls back — it cannot finish through the API.
**Options:** (a) commit the state change at once and delete bottom-up in
batches on a spawned task with a status row, like exports; (b) list-partition
the result tables by job so purge is `TRUNCATE` and delete is `DROP`; (c) leave
it until such a job exists and measure first on the synthetic dataset.
**Recommendation:** (c) then (a). Recorded in Known Limits as open.

### 4.3 Retention of a completed job's scheduling history

`tasks`, `task_claims` (abandoned and declined included) and request rows —
20–40 MB of `forced_racks` per leave generation — are kept for good; and
confirmed/failed `input_data_import_rows` are never deleted. Whether scheduling
history (as opposed to results) must stay live is a policy decision, and
deleting it is irreversible. Recorded in Known Limits.

### 4.4 Rating pools cannot be edited or deleted

Creation is now validated (1.28 and `A-RATE-3b`), but a pool made by mistake is
permanent and pins its anchor config. An update and a delete route are the fix
if it matters; it is admin-only and cosmetic. Recorded in Known Limits.

---

## 5. Objective 8 — `birdtest-contribute`

All changes are on `birdtest-contribute` (commit `68a74611`, on top of
`47b57aad`), none elsewhere:

| Change | Why birdtest needed it |
|---|---|
| Evict changed lexical data before each contribute load (`config.c`, `players_data_evict`, `get_file_identity`) | 1.1 — leave generation played stale leaves |
| `CURLINFO_RETRY_AFTER` fixed; 429 waits clamped; heartbeat does not wait out a 429 | 1.2 |
| Stall timeout instead of a whole-exchange timeout; `CURLOPT_POSTREDIR` | 1.4 |
| `data_filepaths_is_safe_name` on every request name | 1.5 |
| `MAGPIE_VERSION` 0.1.1 | 1.3 — so the floor can refuse the builds above |
| Unused endgame/play-chooser keys removed | They looked like pinned settings |
| Contract fixtures carry the new floor | birdtest's fixtures changed |

Verified: the protocol's field names match the backend structs (claim,
assignment, `expected_data`, decline, heartbeat, result); `magpie builders`
prints what `magpie.rs` parses; the conversion commands the backend invokes
match MAGPIE's parser; `docker/Dockerfile` pins the new HEAD in both `ARG`s.
MAGPIE tests run: `contribute`, `config`, `players`, `builderhash`,
`autoplay`, `klv`, `rit`, `wmp`, `cmdapi`, `command` — all pass.

---

## 6. Other findings

- **Python worker as production client:** none found. Every mention
  (Dockerfile, both compose files, README, `dev.py`, `fake_worker.py`,
  TESTING, RUNBOOK) describes it as an e2e test double.
- **Pagination overflow** (`?page=4e17` wrapped to a negative `OFFSET`, a 500) —
  fixed with `saturating_mul`, unit-tested.
- **Reviewer claims that did not survive verification:** (1) "raw database
  errors are echoed in 5xx bodies" — `AppError::into_response` already
  replaces them with "internal error"; (2) nothing else.
- **Security headers** (none were set) — added to the SPA location.
- **Not run in this pass:** tier 5 (`e2e/run.sh`) builds the Docker images,
  which the user's standing instruction says to ask before doing; Terraform
  validation (not installed). Run: backend (467 tests incl. opt-in MAGPIE),
  clippy, frontend check and Vitest, `backup-drill-check.sh`,
  `restore-roundtrip.sh`, tier 6.
