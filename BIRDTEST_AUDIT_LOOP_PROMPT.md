# Agent Prompt: birdtest Audit Loop

Run the audit in `BIRDTEST_AUDIT_PROMPT.md` as a short loop of passes. Stop as soon as a pass finds nothing serious, or when the budget runs out. Do not modify `BIRDTEST_AUDIT_PROMPT.md` or this file.

## Before the first pass

- Read `BIRDTEST_AUDIT_PROMPT.md` in full. Its rules apply to every pass, in particular:
  - branch rules;
  - MAGPIE changes only on `birdtest-contribute`;
  - one findings file for the whole run;
  - the severity classes;
  - "Verifying a fix";
  - the Known Limits format.
- Create the run's birdtest branch off `main` and the run's findings file (the next `AUDIT_FINDINGS_N.md`).
- Read PLAN.md's "Known Limits and Open Questions". Its `KL-n` entries are the carried record: do not re-flag them unless the code has changed under them.

## Each pass

1. **Plan.** Pass 1 is a **full pass**. Every later pass is a **follow-up pass**: the diff since the previous pass's commit (birdtest and `birdtest-contribute`), plus one area not examined recently. Name that area, and don't pick the same area twice in a run. Write the plan at the top of the pass's section in the findings file.
2. **Review.** Use parallel reviewers, each told to report only, never edit.
   - **A full pass:** five reviewers.
     - dispatch, races and public-route cost;
     - auth, statistics and the frontend;
     - storage, performance, RUNBOOK and README;
     - deployment, CI and security;
     - MAGPIE and objectives 3 and 8.
   - **A follow-up pass:** one reviewer per part of the diff (backend, frontend, docs and procedures, infra, MAGPIE; skip parts the diff doesn't touch), plus one for the named area.
   - **What every reviewer is told:**
     - the severity classes;
     - to reproduce each finding before reporting it;
     - to list low findings apart, in a few lines;
     - not to re-flag `KL-n` entries or decisions recorded as made.
3. **Fix.**
   - Fix every high and medium finding.
   - Fix a low one only if it is small, local and clearly safe. Every other low finding becomes a `KL-n` entry.
   - If a mechanism is on its third correction, redesign it instead of patching it again.
4. **Verify.** Follow "Verifying a fix". The failure is shown first, then the fix is shown to work, and a reviewer who did not write the fix is given only the fix and told to break it. Anything that reviewer finds is fixed within this pass.
5. **Test.**
   - Every pass: the suites that cover what changed, plus `cargo clippy --all-targets -- -D warnings` and the full backend suite.
   - If MAGPIE changed: MAGPIE's full default test table, and `format.py` and `find_circ_deps.py` on a clean archive.
   - If the MAGPIE pin moved: rebuild the release binary and run tier 6.
   - If a procedure or script changed: run it through its failure and re-run cases.
6. **Record and commit.**
   - Add the pass's section to the findings file: kind, scope, findings by severity, what was fixed and how it was verified.
   - Update the `KL-n` entries.
   - Commit birdtest on the run's branch. Commit MAGPIE on `birdtest-contribute`, and update `docker/Dockerfile`'s pin.
7. **Report.** Give a pass report of a few lines: counts by severity, what was fixed, what became `KL-n`, and whether the loop continues.

## When to stop

- **A clean follow-up pass**, meaning no high or medium finding, including none from its adversarial checks: run one **confirmation full pass**. If that is clean too, the loop is done. If it finds high or medium findings, fix and verify them within the confirmation pass, then stop.
- **The budget: at most four passes, including the confirmation.** If pass 4 still finds high or medium findings, fix what can be fixed and verified within it, then stop. List what remains as open in the findings file and in the final output. Don't start a fifth pass unless the user asks for one.
- **Low findings never keep the loop going.**
- **If you are interrupted:** resume from the last committed pass. The findings file's pass sections say where the run stands.

## Finishing

1. **A final green run**, all passing, recorded in the findings file:
   - `cargo clippy --all-targets -- -D warnings`;
   - the full backend suite with `TEST_DATABASE_URL` (tier 6's opt-in tests included when a MAGPIE build is available);
   - `npm run check` and `npm test` in `frontend/`;
   - `terraform fmt -check` and `terraform validate`;
   - `scripts/restore-roundtrip.sh` and `scripts/backup-drill-check.sh`;
   - MAGPIE's test table and tier 6, if MAGPIE changed during the run.
2. **The final output:**
   - a list of every change across all passes;
   - the summary sections (a)–(j) that `BIRDTEST_AUDIT_PROMPT.md` asks for;
   - its "Issues and Recommended Solutions" section;
   - the actions left for a human (for example, pushing `birdtest-contribute`, or building images for tier 5).

## Environment

- Cap build parallelism (`CARGO_BUILD_JOBS=2`, `make -j3`, `nice`): an uncapped build has frozen this machine.
- Never run cppcheck or clang-tidy on MAGPIE.
- Ask before building Docker images.
- Use throwaway Postgres and MinIO containers, and remove them with `docker rm -f -v`. Port 5432 belongs to another project.
- Do not push to any remote.
- Exclude `BIRDTEST_AUDIT_PROMPT.md` and this file from commits.
