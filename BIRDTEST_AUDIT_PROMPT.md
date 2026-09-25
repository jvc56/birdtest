# Agent Prompt: birdtest Repository Audit

You are auditing the birdtest repository (the current working repository), a crowdsourced word-game analysis platform modeled after Fishnet. The stack is an Axum + SQLx backend, a SvelteKit frontend (dark mode only, served by Nginx in production), Terraform-managed AWS infra (VPC, ALB, ECS Fargate, RDS Postgres, S3, SES, SSM, backups), and **MAGPIE itself as the production contributor client** — `magpie contribute` claims tasks, executes them locally, and submits results. `worker/fake_worker.py` is used **only** by the end-to-end test suite to get predictable contributions on cue without a C toolchain in the loop; it is not a production client and there is no "fake-worker mode" anywhere else, including local dev. If you find any code, docs, comments, or config that describe or treat the Python worker as a real production client, that is a mistake — correct it.

The MAGPIE repository is available locally at `~/MAGPIE`. Its `birdtest-contribute` branch is the pre-release version birdtest targets via `MIN_MAGPIE_VERSION` (check `MAGPIE_VERSION` on the branch and the backend's default rather than trusting a number written here).

`PLAN.md` is the design document (architecture, schema, API surface, rationale, the Worker Client protocol, input-data/capability negotiation, backups). `TESTING.md` states what is guaranteed and how it's checked. `RUNBOOK.md` is the recovery procedure. Read all three before starting, to understand intended behavior — but treat the actual code as the source of truth for current behavior, per objective 6 below.

**This may not be the first audit.** Before starting, check the repo for evidence of prior audits: earlier `audit/birdtest-*` branches, git history, and any existing `AUDIT_FINDINGS*.md` files (on `main`, a prior audit branch, or elsewhere). If prior audit artifacts exist:
- Review what was previously flagged as unresolved/pending feedback and check whether it has since been addressed elsewhere before re-flagging it.
- Do not silently re-litigate decisions a prior audit already made and resolved (code-updated or PLAN.md-updated) unless you find something genuinely new or the prior decision now looks wrong in light of new changes to the code.
- Determine the next available version number for the findings file (see the `AUDIT_FINDINGS` versioning rule below) based on the highest existing version found.
- Where relevant, note in a new entry that it revisits or builds on a finding from a prior audit.

## Scope of a pass

An audit may run as one pass or as a loop of passes (see the loop prompt, if one was given). Each pass is one of two kinds, and the loop prompt says which:

- **A full pass** covers every objective below across the whole repository. The first pass of an audit is always a full pass.
- **A follow-up pass** covers only:
  - everything changed since the previous pass (`git diff <previous pass's commit>` in birdtest, and the same range on MAGPIE's `birdtest-contribute`), reviewed against every objective;
  - one area not examined in the last few passes, named in the pass's plan (for example: leave generation, exports, input-data import, the rating sweep, the admin UI, a RUNBOOK section, IAM).

  A follow-up pass does not re-audit unchanged code outside its named area.

## Severity

Classify every finding, and act on it by class:

- **High:** wrong or corrupted results or data, data loss, a security hole, an outage or a stall of seconds or more under realistic load or on a public route, a race that corrupts state, a deployment blocker, or a recovery procedure that would fail or do damage as written.
- **Medium:** the same kinds of failure, but needing an unusual trigger or with bounded impact; a documented guarantee (PLAN.md, TESTING.md, README.md, RUNBOOK.md) that does not hold; a broken user or admin flow.
- **Low:** everything else that is a real improvement: small inefficiencies, cosmetic UI, wording, and hardening with no demonstrated failure.

Fix high and medium findings. Fix a low finding only when the fix is small, local and clearly safe. Otherwise record it as a Known Limits entry (objective 7) rather than fixing it. Style preferences are not findings.

## Verifying a fix

A fix is not done until it has been verified. Most defects found late in earlier audits were in the previous pass's own fixes, each verified too narrowly.

- **Show the failure first.** Before a fix, reproduce the failure: a test that fails, a measurement, or a run of the procedure. Afterwards, show the same check passing. A fix to code gets a test. A performance fix is measured at realistic scale, including adversarial inputs (old or forged cursors, heavy and light users, skewed statistics). A fix to a procedure is run: for a shell procedure, in `bash -i` with the real tools where possible and stubs where not, through its failure and re-run cases.
- **Adversarial check.** Before the pass is committed, a reviewer who did not write the fix is given only the fix and told to break it: its edge cases, its assumptions, its behaviour at scale, and the paths it did not touch that share its logic. A problem found here is fixed within the pass, not left for the next one.
- **Prefer a redesign over a third patch.** If a mechanism needs a third correction, step back: redesign it, or remove it.

## Procedures as scripts

A recovery or operations procedure longer than a few commands, or one with conditions, guards or loops, belongs in a script under `scripts/`, not in a Markdown code block to be pasted. Test the script in CI, with external tools (`aws`, `terraform`) stubbed where they cannot run. The RUNBOOK or README then says which script to run, with what arguments, and what it checks. When an audit finds a defect in a pasted procedure, move it into a script as part of the fix.

## Your objectives, in order

1. **Read `PLAN.md`, `TESTING.md`, and `RUNBOOK.md` first**, to understand the intended architecture and design (priority-tier weighted-random task scheduler, redundant task execution via `task_claims`, per-job ELO ratings and SPRT statistical testing, impossible-result plausibility checks and pentanomial cross-checking as worker-integrity mechanisms, admin role handling via `is_admin`, content-pinned input data, the derived-data builder for wordmaps/rack-info tables/leave-generation KLVs, the version-floor mechanism, and backup/restore procedures). Treat all three as *summaries* of intent, not as ground truth — the actual code is the source of truth for current behavior.

2. **Full-repo review.** Go through the entire codebase — backend, frontend, MAGPIE integration points, schema/migrations, Terraform, CI (`.github/workflows`), scripts, docs — and identify:
   - Bugs and logical mistakes
   - Inefficiencies (algorithmic, query, or resource-level)
   - Oversights (unhandled edge cases, missing validation, race conditions, error-handling gaps)
   - Footguns (things that work today but will break under load, concurrency, or misuse)
   - General improvements (code quality, structure, testing, observability, security)
   - Places where **`TESTING.md`'s stated guarantees don't actually hold**, or where **`RUNBOOK.md`'s recovery steps would not actually work** as written against the current code/infra — these are correctness bugs in the documentation-as-contract, not stylistic notes.

   **Give particular attention to race conditions across the entire codebase.** Task dispatching and receiving are a specific area of concern — carefully verify how tasks are claimed, assigned, marked in-progress, and completed, and check for issues like double-dispatch, lost updates, unprotected read-modify-write sequences on shared state, or assumptions that don't hold under concurrent workers. Don't limit this check to task dispatch/receipt alone — look for similar concurrency hazards anywhere else in the backend, database access patterns, or job lifecycle logic (e.g., redundant task execution via `task_claims`, job completion/aggregation logic, ELO/SPRT updates, the derived-data builder's interaction with in-flight jobs). Treat any race condition found as a bug under this objective.

   Fix these directly in the code rather than just reporting them, unless the fix is ambiguous or needs a design decision — in that case, flag it instead of guessing.

3. **Verify every MAGPIE argument that can affect a task's outcome is explicitly set by the server-dispatched task, not left to whatever the client/worker has locally.** Enumerate the full set of MAGPIE CLI arguments/flags/config options that can influence simulation or analysis results (seeds, depth, iteration counts, equity/strategy parameters, lexicon, board/rack config, time limits, or anything else that changes output). For each one, trace whether birdtest's task dispatch always sets it explicitly from server-side task data, or whether it's ever left unset/defaulted, allowing it to silently take whatever value is already configured on the worker's local MAGPIE instance (e.g., anything read from a contributor's own `contribute.txt` or `settings.txt` rather than the dispatched task). Any argument that can affect results but isn't guaranteed to be explicitly set per-task is a correctness bug: it means different workers could produce divergent, non-reproducible results for what's supposed to be the same task, which would corrupt job data (ELO, SPRT, plausibility, cross-checking all assume comparable results). Fix these directly by ensuring the task dispatch payload and the worker invocation cover every outcome-affecting argument explicitly. Arguments that are purely cosmetic/informational (e.g., verbosity, thread count, output formatting) don't need this treatment — only flag/fix those that can change the actual outcome of a task.

4. **Analyze the critical path for job and task processing, and move non-essential work off it.** The priority is completing jobs and tasks, and getting workers a new task as fast as possible after they finish one. For task dispatch, task receipt/completion, and the overall job lifecycle, trace exactly what work happens synchronously in that path versus what's purely for display, monitoring, or informational purposes (e.g., progress percentages, dashboards, the `/admin/derived-data` and `/admin/backups` views, logging/metrics that aren't needed to decide the next action, notifications, UI-facing aggregates). Anything that only feeds visibility/reporting and isn't required to correctly dispatch the next task, validate a submission, or advance job state should be moved off the critical path — made async, deferred, batched, or queued — even if that means progress displays or informational views lag slightly behind real time. Do not defer anything that affects correctness (e.g., data needed for `task_claims` redundancy checks, plausibility validation, or SPRT/ELO calculations that gate job completion) — only defer work that is purely observational. Fix these directly where safe; flag anything where moving work off the critical path could risk correctness or introduce a new race condition (cross-reference objective 2's concerns) rather than guessing.

5. **Analyze performance across birdtest and MAGPIE (where MAGPIE is relevant to birdtest).** Focus on large, unexpected performance pitfalls — things that could plausibly cost many seconds or minutes more than expected (e.g., unbounded/unindexed queries, N+1 query patterns, blocking calls in hot paths, unnecessary full-table scans, inefficient locking, pathological worst cases in task scheduling or matching logic, slow MAGPIE invocation patterns, the derived-data builder's ~2.4 GB memory / 1.9 GB file-write behavior interacting badly with anything). Do not spend effort chasing small, marginal gains (millisecond-level micro-optimizations) — that is out of scope for this pass. For any performance issues found, list them **in order of severity, from most severe to least severe**, along with their expected real-world impact (e.g., "could add ~30s per job under load X" or "could stall the scheduler for minutes under condition Y"). Fix clear, safe wins directly; flag anything that needs a design trade-off or further profiling rather than guessing.

6. **Analyze and optimize storage.** Review how birdtest stores data — schema design, table/column sizing, indexing strategy, and anything that could cause unbounded or unexpectedly fast growth (e.g., per-task or per-game result rows, raw MAGPIE output, logs, task history, `task_claims` records that are never pruned, leave-generation progress rows — noted in the docs as ~3.2M rows per full-rack job, copied again for every later generation). Specifically look for:
   - Data stored at a finer granularity than anything actually consumes
   - Missing or inappropriate indexes causing bloat or slow lookups, and superfluous indexes adding write overhead for no read benefit
   - Inefficient types/encodings for large or repetitive fields
   - Lack of any retention, archival, or pruning strategy for data that grows unbounded, where PLAN.md/TESTING.md or the system's needs imply old data doesn't need to stay fully live forever
   - Duplication of data across tables/rows that could be normalized, or over-normalization causing excessive joins on hot paths
   - Anything that could make the nightly `pg_dump` or the restore drill (`scripts/restore-drill.sh`, `scripts/restore-roundtrip.sh`) slower or more fragile as data grows
   As with performance, prioritize large/structural storage issues over marginal savings. Fix clear, safe wins directly (e.g., adding a missing index, tightening an oversized column type); flag anything involving a retention/deletion policy or schema migration with real trade-offs rather than deciding unilaterally, since deleting or restructuring historical data can be irreversible — especially given the single-migration-edited-in-place approach (`backend/migrations/0001_initial.sql`) still in effect pre-release.

7. **Reconcile code against PLAN.md — and produce a written record of every discrepancy.** Create a new, versioned markdown document (see the `AUDIT_FINDINGS` versioning rule below) that lists every place where the code and PLAN.md disagree. For each discrepancy, record:
   - What the code does
   - What PLAN.md says
   - The decision made: **code updated**, **PLAN.md updated**, or **left unresolved pending feedback**
   - The reasoning behind that decision
   The default bias is that the code wins and PLAN.md gets updated to match it, since PLAN.md is a summary document. Only update the code instead when it looks obviously wrong (a genuine bug, or a clear mismatch with what the rest of the system needs) and PLAN.md's description looks like the better/intended behavior. If a discrepancy is genuinely ambiguous — reasonable arguments on both sides, or a real design trade-off — do not guess: leave both code and PLAN.md untouched, document it in the findings file with the trade-offs laid out, and flag it as needing human input.

   **Keep PLAN.md's "Known Limits and Open Questions" section current, in its numbered format.** Every finding this audit leaves in place — a limit accepted on purpose, an option considered and not built, an item left unresolved pending feedback, or a smaller issue weighed and left — gets an entry there, numbered `KL-n`, with these five parts:
   - **Context:** where it lives and what it depends on.
   - **Problem:** what goes wrong, or what it costs.
   - **Options considered:** the alternatives weighed. If none were, say so rather than inventing some after the fact.
   - **Option implemented:** what the code does now. For an item still open, that is "none yet", naming the audit that opened it.
   - **Justification:** why that option, and what would make it worth revisiting.

   Numbers are permanent: a new entry takes the next unused number, whichever subsection it goes in. Re-check every existing entry against the current code. If this audit changes one, update its five parts in place. If this audit resolves one, keep its number, say in "Option implemented" what was done and in which audit, and do not delete it. Add the audit's findings file to the section's list of findings files. The findings file refers to these entries by number.

8. **Verify MAGPIE's `birdtest-contribute` branch supports birdtest.** In `~/MAGPIE`, check out the `birdtest-contribute` branch, which is expected to already contain everything birdtest needs to function as its production worker (including whatever the backend needs to build wordmaps, rack info tables, and leave-generation KLVs, and whatever `magpie contribute` needs for the dispatch protocol). Check this assumption directly — don't take it on faith. If you find gaps (missing functionality, an interface birdtest expects that MAGPIE doesn't provide, outdated behavior, version-floor mismatches, etc.), fix them **directly on the `birdtest-contribute` branch** — never on `main` or any other MAGPIE branch. Record every such change in the findings file as well, noting what was missing and why it was needed.

9. **Deployment/implementation blockers.** Hunt for anything that would prevent birdtest from being built, deployed, or run end-to-end in production with MAGPIE as the worker: missing environment/config handling, broken build steps, incomplete Docker/CI setup, missing migrations, unhandled dependency on the MAGPIE binary or `MAGPIE_ROOT`, missing secrets management, incomplete auth/CSRF handling, half-finished features, mismatches between `backend_image` and `derived_builder_image`'s pinned MAGPIE version, or anything that would make Terraform's required-but-defaultless variables (`acm_certificate_arn`, `alert_email`, `derived_builder_image`) easy to misconfigure at first deploy. Resolve these directly.

## Constraints and process

- **In the birdtest repo:** do not commit to `main`. Create a new branch off `main` (e.g. `audit/birdtest-<date>`, incrementing/disambiguating if a branch with that date already exists) and make all birdtest-side changes there — code fixes, PLAN.md updates, and the findings file.
- **`AUDIT_FINDINGS` versioning:** never overwrite or append to a prior audit's findings file. Each audit creates one new file — e.g. `AUDIT_FINDINGS_1.md`, `AUDIT_FINDINGS_2.md`, `AUDIT_FINDINGS_N.md` — using the next integer after the highest version already present anywhere in the repo's history (main and any audit branches). An audit that runs as a loop of passes keeps **one** file for the whole run, with a section per pass (its kind, scope, findings by severity, and what was fixed and verified), and a summary at the top that is kept current. Prior audits' findings files remain untouched as a historical record.
- **In `~/MAGPIE`:** make all changes directly on the existing `birdtest-contribute` branch, as described in objective 8. Do not create a new branch for this.
- State both the birdtest branch name and confirmation of `birdtest-contribute` usage in your final summary.
- Prioritize correctness and safety over stylistic preferences — don't rewrite working code just to match a different taste.
- Keep changes scoped and reviewable; avoid sweeping rewrites unless something is fundamentally broken.
- Run existing tests (`cargo test --lib --bins`, the `TEST_DATABASE_URL` integration suite, `npm run check` in `frontend/`) and add tests for anything you fix that lacked coverage, before considering an issue resolved. See "Verifying a fix" for what else a fix needs.
- Before considering an audit finished, check that PLAN.md's "Known Limits and Open Questions" section has a numbered entry, with all five parts, for every finding this audit left in place, and that entries it resolved say so (objective 7).
- The findings file for this audit is the authoritative record of every code-vs-plan decision made during this audit — it must be complete enough that a human reviewer could re-derive and second-guess each decision without re-reading the diffs. If this is a repeat audit, briefly note at the top of the file which prior audit(s)/version(s) it builds on, if identifiable.
- Produce a final summary organized by: (a) birdtest branch name created (and note if this is a repeat audit, referencing prior audit branches/findings versions found), (b) bugs/fixes made in birdtest — with race conditions called out as their own subsection, (c) MAGPIE argument coverage issues found and fixed, (d) critical-path/async changes made, (e) performance issues found — ordered most to least severe, with expected impact and what was fixed vs. flagged, (f) storage issues found and fixed vs. flagged, (g) Python-worker-as-production-client corrections made, (h) `birdtest-contribute` changes made and why, (i) the filename of this audit's findings file, a one-line count of code-wins / plan-wins / unresolved items, and the `KL-n` numbers added, changed or resolved in PLAN.md's "Known Limits and Open Questions", (j) deployment blockers found and resolved.
- **At the very end of your output, include a section titled "Issues and Recommended Solutions"** covering every issue surfaced across all objectives above that was fixed, flagged, or left unresolved (bugs, race conditions, MAGPIE argument gaps, critical-path changes, performance issues, storage issues, PLAN.md discrepancies, MAGPIE gaps, deployment blockers, TESTING.md/RUNBOOK.md inaccuracies — everything). For each issue, include: relevant context (file/location, how it was found), the problem itself, the possible solutions considered, and your recommendation — including what you actually did, if you acted, or why you flagged it instead. This section should be thorough enough to stand on its own as a full technical record, independent of the rest of the summary.
