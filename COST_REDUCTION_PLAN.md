# AWS cost reduction plan

**Goal:** take the monthly bill from about $148 to about $94 without removing a feature.
**Status:** in progress (started 2026-11-03). Phases 0–2 done; the change is on branch `cost-reduction-2026-11`.
**Evidence date:** 2026-11-02, from Cost Explorer, CloudWatch, ECS, RDS and a read-only `terraform plan`. An independent review found one blocker and two serious errors in the first draft; all its findings are addressed here. Phase 9 and the `api_5xx` alarm were added after that review.

## What you need to do

| When | Action | Avoidable? |
|---|---|---|
| Once, to start | Reply "go" | No: this approves one irreversible change (step 3), a production apply, and restarting randomracer |
| Before saying "go" | Switch this session to a mode that doesn't prompt for each command, or be ready to approve prompts | Yes, if you choose that mode: `scripts/deploy.sh`, `gh pr merge` and `aws budgets` are not on your allow list |
| Only if your session has expired when the run starts | Run `aws sso login` | No: it needs your browser, and the plan checks first so it asks at most once |

Everything else is automated: the code change, tests, PR, merge, deploy, verification, budget and anomaly-alert changes, and the randomracer swap.

**Expect these emails after the run, and ignore them:** an OK notice from each new alarm, and possibly a budget forecast alert while AWS's forecast catches up with the lower spending.

## Why the bill is this size

Before birdtest launched on Oct 5–6 the account cost about $15/month, all of it the randomracer server. October's $135.88 included a $16 domain charge, which repeats every October, and 26 days of birdtest. At the rate once backups stop growing, the bill is about $148/month.

| Item | Monthly | Evidence |
|---|---|---|
| Web task (1 vCPU, 2 GB) | $36 | CPU at or under 1% since Oct 12; lifetime peak 45.5%; memory peak 10.9% |
| Derived-file builder | $29 | About 8,000 runs in 4 weeks; 16 files built on 3 days; last build Oct 12 |
| RDS instance and storage | $16 | Already micro size |
| Load balancer | $16 | Required by the architecture |
| Public IPv4 | $15 | Load balancer and tasks; there is no NAT gateway |
| randomracer server | $15 | `t2.micro`, gp2 disk and a public IP; 233 MB of memory in use |
| Frontend task | $9 | Already the smallest Fargate size |
| RDS backup storage | $7 or more | $0.18/day on Oct 31 and rising as the 30-day window fills |
| Everything else | About $4 | Not worth changing |

## Changes

| # | Change | Saves per month | Trade-off |
|---|---|---|---|
| 1 | `derived_builder_schedule`: `rate(5 minutes)` to `rate(1 hour)` | About $27 | Builds can wait much longer; see "What hourly builds mean" |
| 2 | `task_cpu`: 1024 to 512, `task_memory` stays 2048 | About $15 | Heavy admin work runs up to twice as slowly; about 2 minutes of API downtime during the change |
| 3 | `db_backup_retention_days`: 30 to 7 | About $6 | Older point-in-time backups are deleted, which **cannot be undone** |
| 4 | New alarms: `birdtest-backend-cpu-high`, `birdtest-api-5xx` | $0 | None: these are alarms eight and nine, and the first ten are free |
| 5 | Budget limit: $200 to $125 | $0 | None |
| 6 | Anomaly alert: from "$100 and 40%" to "$20 or more" | $0 | Possibly the odd extra email |
| 7 | randomracer: `t2.micro` to `t3a.nano`, disk gp2 to gp3, new 1 GB swap file | About $5.60 | Half the memory; two site outages of a few minutes during the run |

**Expected result:** about $94/month, or about $3.10/day. All savings are estimates from October's rates, and backup savings could be slightly higher.

### What the changes mean in practice

**What hourly builds mean**

| Situation | Now | Hourly |
|---|---|---|
| New file for a job | Up to 5 min | Up to 1 hour |
| Transient failure, three attempts | About 20 min | About 2 hours |
| Builder killed during a build | Up to about 80 min | Up to about 2 hours |
| Restore, or a MAGPIE pin that changes the builder version | A few minutes | Jobs may not dispatch for up to 2 hours |

A run builds at most eight files. The new `scripts/build-derived-now.sh` starts a run on demand, but it still honours the wait after a failed build.

**Half the web task's CPU**
- Normal traffic is safe: 20,000–66,000 requests a day used 1% CPU or less.
- The risk is in bursts production hasn't had recently: no KLV conversion, export or import has run in the last 14 days. The 45.5% lifetime peak becomes about 91%.
- The admin "rebuild artifacts" request reaches the load balancer's timeout past about twenty generations today, and that likely becomes about ten.

**7-day recovery window**
- Nightly dumps (365 days, replicated) still cover older restores, with up to a day of data lost.
- Corruption found after 7 days falls back to a dump.
- `reset-prod-db.sh` deletes the dumps too, so after a reset the only way back is the 7-day window.

**Why these values**
- Memory stays at 2 GB because `PLAN.md` KL-44 puts the worst-case burst near that level, and 1 GB saves only about $3 more.
- The budget is $125 so its 85% alert sits at $106.25. Normal months of $93–$97 and this November stay quiet. October's domain renewal month, at about $112, will trigger that alert, which is expected.
- Step 2 also replaces the backend task on the new Fargate platform version, so its Nov 8 retirement gap happens at a time we choose.

## Rules for the run

- **Stop rule:** if any check fails before the deploy, stop, change nothing further and report it. Do not work around a check.
- **No image rebuilds:** do not edit anything under `backend/`, `frontend/` or `docker/`, or `Cargo.*` or `.dockerignore`. Any of those makes `deploy.sh` rebuild images.
- **No task definition edits:** do not edit `scripts/backup.sh`, `scripts/restore-drill.sh` or `scripts/restore-job.sh`, even for a matching comment. They are read into task definitions, so an edit fails the Phase 4 check. The "five minutes" in `restore-drill.sh:21` is about RDS autoscaling and is correct.
- **No `--skip-ci-check`**, and no `BIRDTEST_ALLOW_DESTRUCTIVE_PLAN`.
- **Never run `scripts/rollback.sh` in this plan.** It moves images only, and this release changes none.
- **Scope:** on randomracer, do only what Phase 9 lists. Do not touch the frontend, RDS class or storage, or the open draft PR #31.
- **Working directory:** `S=$(mktemp -d)` once at the start; use it for all temporary files and delete it at the end.

## Execution

### Phase 0: Preflight (read-only)

All of these must pass. If the AWS session has expired, ask for `aws sso login` once, then continue.

1. `aws sts get-caller-identity --profile birdtest` shows account `824858782304`; `gh auth status` passes.
2. On `main`, no tracked changes, and `HEAD` equals `origin/main`.
3. Both ECS services have one running task and one `COMPLETED` deployment, and the live backend image tag equals the one in `prod.tfvars`.
4. `infra/prod.tfvars` matches `s3://birdtest-tfstate-824858782304/birdtest/prod.tfvars` byte for byte. If it differs, `deploy.sh` asks an extra question that the unattended run cannot answer.
5. `terraform -chdir=infra plan -lock=false -input=false -detailed-exitcode -var-file=prod.tfvars` exits 0, meaning no drift.

### Phase 1: Make the change on branch `cost-reduction-2026-11`

**Terraform**
- `infra/derived.tf`: default to `rate(1 hour)`. Update the variable description and the comments at lines 10, 17, 187–188 and 298.
- `infra/variables.tf`: `task_cpu` default 512; `db_backup_retention_days` default 7.
- `infra/rds.tf`: rewrite the "Thirty days of point-in-time recovery" comment.
- `infra/ecs.tf`: add these two alarms:

```hcl
# Sustained load only: a long export can trip it, which is a reason to look,
# not by itself a reason to raise task_cpu.
resource "aws_cloudwatch_metric_alarm" "backend_cpu_high" {
  count               = var.desired_count > 0 ? 1 : 0
  alarm_name          = "${local.name}-backend-cpu-high"
  alarm_description   = "birdtest's backend has been over 85% CPU for fifteen minutes: look at task_cpu"
  namespace           = "AWS/ECS"
  metric_name         = "CPUUtilization"
  dimensions          = { ClusterName = aws_ecs_cluster.main.name, ServiceName = aws_ecs_service.backend.name }
  statistic           = "Average"
  period              = 300
  evaluation_periods  = 3
  threshold           = 85
  comparison_operator = "GreaterThanOrEqualToThreshold"
  alarm_actions       = [aws_sns_topic.alerts.arn]
  ok_actions          = [aws_sns_topic.alerts.arn]
  tags                = local.tags
}

# Fifteen minutes so that a deploy's two-minute gap never trips it. No errors
# means no datapoints, which is healthy here.
resource "aws_cloudwatch_metric_alarm" "api_5xx" {
  count               = var.desired_count > 0 ? 1 : 0
  alarm_name          = "${local.name}-api-5xx"
  alarm_description   = "birdtest's load balancer has answered 5xx for fifteen minutes"
  namespace           = "AWS/ApplicationELB"
  metric_name         = "HTTPCode_ELB_5XX_Count"
  dimensions          = { LoadBalancer = aws_lb.main.arn_suffix }
  statistic           = "Sum"
  period              = 300
  evaluation_periods  = 3
  threshold           = 5
  comparison_operator = "GreaterThanOrEqualToThreshold"
  treat_missing_data  = "notBreaching"
  alarm_actions       = [aws_sns_topic.alerts.arn]
  ok_actions          = [aws_sns_topic.alerts.arn]
  tags                = local.tags
}
```

**Reset warning (must change together)**
- `scripts/lib/ops.sh:754`: change the fallback to `pitr=${pitr:-7}`. It feeds the warning an operator reads before an irreversible reset.
- `scripts/ops-scripts-check.sh:267`: change the pinned text to `until they expire, 7 days from now`.

**New script: `scripts/build-derived-now.sh`**
- Use `ops_init`, the service subnet and security group outputs, and the task family `$OPS_CLUSTER-derived-builder`.
- Follow the `aws ecs run-task` call in `scripts/check-alerts.sh`, without its `--overrides`.
- Wait for the task to stop in a loop that can outlast ECS's 10-minute waiter, then exit with the container's `exitCode`.
- Add it to the `bash -n` list at `scripts/ops-scripts-check.sh:67` and to that file's header, with coverage using the fakes.

**Docs.** Search for all of these, including phrases split across lines: `five minutes`, `five-minute`, `few minutes`, `about 20 minutes`, `1 vCPU`, `30 days`, `30-day`, `(30)`, `Thirty`.

| File | Lines known to be stale |
|---|---|
| `README.md` | 561, 578, 826, 840, 1110, 1136, 1158, 1173; add `build-derived-now.sh` to the operator scripts |
| `RUNBOOK.md` | 1195, 1214–1216, 2037; add `build-derived-now.sh` to the restore and MAGPIE-pin steps, and note that it honours the failure wait |
| `PLAN.md` | 4715, 6072, 8766, 10542, 10650, 11012, 11103, "Backups and Restore"; update KL-19 for the halved limit |
| `JOURNEYS.md` | 80, 1314 |
| `UPDATES_PLAN.md` | 111 |
| `scripts/lib/ops.sh` | 652 |
| `scripts/reset-prod-db.sh` | 19, 32 |

**Deferred to the next backend or frontend release**, because editing them would rebuild images: `docker/Dockerfile:151`, `backend/src/bin/build-derived.rs:5` and `:14`, `backend/src/ratings.rs:521`, `backend/src/auth/api_key.rs:54`, and `frontend/src/routes/admin/derived-data/+page.svelte:97`. List these in the PR description.

**This plan:** include it in the PR, marked as in progress.

### Phase 2: Test locally

1. `terraform fmt -check $(git ls-files 'infra/*.tf' 'infra/*.tftest.hcl')`, plus the new alarms' file. Not `-recursive`: that flags the git-ignored `prod.tfvars`, and formatting it would break Phase 0 step 4.
2. `terraform -chdir=infra validate`
3. `terraform -chdir=infra test`
4. `scripts/ops-scripts-check.sh`
5. `scripts/runbook-check.sh RUNBOOK.md README.md`

### Phase 3: Merge

1. Commit, push and open a PR. Confirm `git diff --name-only origin/main` lists nothing under `backend/`, `frontend/` or `docker/`, and none of `Cargo.*`, `.dockerignore`, `scripts/backup.sh`, `scripts/restore-drill.sh` or `scripts/restore-job.sh`.
2. Wait for CI (about 12 minutes). Only `ci.yml` must pass.
3. `gh pr merge --merge --delete-branch`, then check out `main`, pull, and wait for `ci.yml` to go green on the merge commit. `deploy.sh` requires that.

### Phase 4: Check the exact plan

Save a plan and refuse to continue unless it contains exactly this:

| Resource | Action | Allowed change |
|---|---|---|
| `aws_cloudwatch_metric_alarm.backend_cpu_high[0]` | create | New alarm |
| `aws_cloudwatch_metric_alarm.api_5xx[0]` | create | New alarm |
| `aws_ecs_task_definition.backend` | replace | `cpu` 1024 to 512; normalized `container_definitions` |
| `aws_ecs_service.backend` | update | `task_definition` only |
| `aws_scheduler_schedule.derived_builder` | update | `schedule_expression` only |
| `aws_db_instance.main` | update | `backup_retention_period` only |

The review tested the first draft's edits in a scratch copy: `validate` passed, `terraform test` passed 58 of 58, and the plan matched its allowlist. The `api_5xx` alarm was added after that review, so this will be its first test.

```bash
terraform -chdir=infra plan -lock=false -input=false -var-file=prod.tfvars -out="$S/check.tfplan"
terraform -chdir=infra show -json "$S/check.tfplan" \
  | jq -r '.resource_changes[] | select(.change.actions != ["no-op"]) | "\(.address) \(.change.actions | join("/"))"'
```

For `aws_db_instance.main`, compare `.change.before` with `.change.after` and confirm only `backup_retention_period` differs. Delete the saved plan afterwards, because it may contain secrets.

### Phase 5: Deploy

**Timing:** do not start between 02:45 and 03:30 UTC (nightly dump) or between 06:45 and 08:30 UTC (RDS backup window). If the run reaches this point inside either window, wait for it to end.

`deploy.sh` reads its one confirmation from `OPS_TTY`, so the answer comes from a file. Phase 0 step 4 ensures there is only one prompt. If a second appears, it reads end-of-file and the deploy stops safely. The destructive-plan refusal still applies.

```bash
printf 'y\n' > "$S/answers"
OPS_TTY="$S/answers" scripts/deploy.sh
```

Run it in the background. It waits until both services are healthy. Expect about 2 minutes of API 503s, with pages still loading.

**`deploy.sh` reporting success doesn't prove the new CPU is live.** It compares images, and this release changes none, so a circuit-breaker rollback to 1024 would still say "deployed". Phase 6 is the real check.

### Phase 6: Verify

| Check | Pass condition |
|---|---|
| Web task | Running task uses **512** CPU and 2048 memory; target group healthy |
| Site | `https://birdtest.org/health` and `https://birdtest.org/` return 200 |
| Schedule | `aws scheduler get-schedule` shows `rate(1 hour)` |
| Recovery window | `aws rds describe-db-instances` shows retention 7 and status `available` |
| Alarms | Both new alarms exist and neither is in `ALARM` |
| On-demand build | `scripts/build-derived-now.sh` exits 0 |
| Nightly dump | `birdtest-backup` schedule is `ENABLED` and `birdtest-backup-stale` is `OK` |

### Phase 7: Update cost alerts

1. **Budget:** read the budget, keep only `BudgetName`, `BudgetLimit`, `TimeUnit`, `TimePeriod`, `BudgetType`, and its filter and metrics fields, set the limit to `125`, and write it back with `aws budgets update-budget`. Confirm the three notifications and the subscriber are unchanged. If AWS rejects the update, report it and continue, because nothing depends on it.
2. **Anomaly alert:** run `aws ce update-anomaly-subscription` on `Default-Services-Subscription` with `{"Dimensions":{"Key":"ANOMALY_TOTAL_IMPACT_ABSOLUTE","Values":["20"],"MatchOptions":["GREATER_THAN_OR_EQUAL"]}}`. The current "$100 and 40%" rule would rarely fire on a $100 bill.

### Phase 8: Find what is using database disk (read-only)

Free database disk fell from 18 GB to 9.3 GB in four weeks. Autoscaling could start in about 16 days, and each step adds about $1/month that never shrinks. CloudWatch already rules out WAL and replication slots.

Run one `SELECT`-only file through `scripts/prod-psql.sh`: total database size, the 15 largest tables with their indexes, their `n_dead_tup`, and `temp_bytes` from `pg_stat_database`. Select sizes and names only, never personal columns, because output is kept in the logs for 30 days. Report the result; do not change anything.

### Phase 9: Swap randomracer to a cheaper server

This phase is independent of birdtest: a failure here doesn't undo anything above, and a birdtest failure before the deploy skips it. Region `us-east-2`, instance `i-0b5aa1f845bac3f77`, volume `vol-0547c921fd59d6a5e`, SSH as `ubuntu` with `~/randomracer-keypair1.pem`. The Elastic IP `3.13.229.56` stays attached.

**Timing:** do not start between 05:45 and 07:15 UTC, because the site's scheduled update runs at about 06:24 UTC over SSH.

The first restart in 755 days also boots a newer kernel (`5.15.0-1084-aws`) that has never run. The two restarts below keep that separate from the hardware change.

1. **Check:** `https://randomracer.com/` and `/cgi-bin/cgi_wrapper.pl` return 200, and `/lib/modules/5.15.0-1084-aws/kernel/drivers/net/ethernet/amazon/ena/` exists. Stop this phase if either fails.
2. **Snapshot:** `aws ec2 create-snapshot` on the volume, then `aws ec2 wait snapshot-completed`.
3. **Disk:** `aws ec2 modify-volume --volume-type gp3`. No restart is needed.
4. **Swap file:** create a 1 GB `/swapfile` with mode 600, run `mkswap` and `swapon`, and add it to `/etc/fstab` only if it isn't there. Confirm with `swapon --show`.
5. **First restart, same server:** `sudo reboot`, then confirm `uname -r` shows `5.15.0-1084-aws`, `swapon --show` lists the file, and both URLs return 200.
6. **Second restart, new server:** stop the instance and wait, `aws ec2 modify-instance-attribute --instance-type Value=t3a.nano`, start it, and `aws ec2 wait instance-status-ok`. Set CPU credits to `standard` if they aren't, so a busy spell slows the server instead of adding charges.
7. **Verify:**

| Check | Pass condition |
|---|---|
| Server | `t3a.nano`, running, same IP, `standard` credits |
| Disk | gp3 and the volume's modification has no error |
| Site | Both URLs return 200 |
| Memory | `free -m` shows about 450 MB total, 1 GB swap, and more than 100 MB available |
| Services | `apache2` and `ssh` active; `systemctl --failed` lists nothing new |

8. **Clean up:** delete the snapshot once every check passes, so it adds no cost and needs no follow-up.

The scheduled update can't be tested in the run. It connects to the same IP with the same host keys, so nothing here should affect it.

### Phase 10: Report

1. Mark this plan done, recording the outcome and any deviations, in a docs-only PR. Docs deploy nothing.
2. Save a project memory with the new values and the reason for them.
3. Tell you the result.

## Rollback

`scripts/rollback.sh` is not part of any row: see "Rules for the run".

| Problem | Response |
|---|---|
| Fails before the deploy | Stop and report; production is unchanged |
| Phase 6 shows CPU still 1024, or a `-deploy-failed` mail arrives | Stop and report. The site is healthy on the old task, so no urgent action is needed |
| Site unhealthy on 512 | `OPS_TTY="$S/answers" scripts/deploy.sh --set task_cpu=1024` |
| Web task too slow, or an alarm fires later | Set `task_cpu` back to 1024 through the same PR and deploy path |
| Builds too slow to arrive | Set the schedule to `rate(30 minutes)`, which adds about $2.50/month |
| Recovery window regretted | Set it back to 30; backups already deleted do not return |
| randomracer doesn't come back on `t3a.nano` | Stop it, set `t2.micro`, start it, verify, keep the snapshot and report |
| randomracer doesn't boot the new kernel | Attach its disk to a temporary helper instance, set GRUB to boot `5.15.0-1070-aws`, reattach and start. If that fails, keep the snapshot and report: the site stays down until you decide |
| randomracer short of memory later | Change it to `t3a.micro`, which adds about $3.40/month and a short restart |

## After the run

There is no scheduled follow-up, because a cloud agent cannot use your SSO login and a session timer ends with the session. AWS does the watching and emails `joshuacastellano7@gmail.com`:

- **Sustained CPU:** `birdtest-backend-cpu-high`.
- **Failing requests:** `birdtest-api-5xx`.
- **Spending off target:** budget alerts at $106.25 (actual), $125 (actual) and $125 (forecast).
- **Sudden cost spike:** the anomaly alert, at $20.

Nothing watches a growing build queue or randomracer. Short CPU bursts are also not alarmed unless they cause errors.

## Not done: what you could still give up

Nothing in this section is part of the run. Pick any of these and it gets its own plan and review. All figures are estimates, measured from the $94 this plan reaches.

**Dropping an app feature usually saves nothing.** Birdtest pays for always-on infrastructure, not for use: the web task costs the same at 1% CPU as at 100%. A feature only saves money if removing it lets something be switched off.

**Where the remaining $94 goes**

| Item | Monthly |
|---|---|
| Web task (0.5 vCPU, 2 GB) | $21 |
| Load balancer | $16.50 |
| RDS instance and storage | $16 |
| Public IPv4, birdtest | $15 |
| randomracer server | $9.50 |
| Frontend task | $9 |
| Derived-file builder | $2.50 |
| Everything else, including RDS backups | About $5 |

**Usage, last 7 days:** 18,000–45,000 API requests a day and 560–880 page requests a day, arriving around the clock with no quiet period. The site has no natural time to switch off.

### Options that give something up

| Option | Saves per month | What you lose | Effort |
|---|---|---|---|
| Shut down randomracer | **About $9.50** | randomracer.com goes offline; keeping a disk snapshot would cost about $0.50 of that | Small |
| Put the frontend back inside the web task | **About $13** | Pages can no longer deploy on their own: every page change restarts the API, and pages go down too during each ~2-minute restart | Large: undoes a 988-line split from Oct 9 |
| Pause the site while no jobs are running | **About $1.70 per day paused** | The site, sign-in and workers are all offline until it's resumed; RDS can stay stopped for 7 days at most, and the "down" alarms need silencing | Medium; someone must pause and resume it |
| Run the site 12 hours a day | **About $19** ($25 with RDS stopped too) | The site and workers are offline half of every day; jobs take about twice as long | Medium |
| Web task at 0.25 vCPU, 2 GB | **About $7.40** | Heavy admin work up to 4× slower than today; the 0.45 vCPU peak no longer fits, so conversions could affect the live API | Small |
| Web task at 0.25 vCPU, 1 GB | **About $10.60** | As above, and the documented worst-case memory burst would crash the task | Small |
| Run builds only by hand | $2.50 | Jobs needing a new file wait until someone runs `build-derived-now.sh`, which brings back a manual step | Small |
| Drop the cross-region backup copy | About $1 | No protection if us-east-1 is lost | Small |
| Shorten the recovery window to 1 day | About $1 | Nightly dumps become the only way back past a day | Small |

### Options that lose no features

| Option | Saves per month | Catch |
|---|---|---|
| **Recommended next:** start builds automatically when a job needs them, keeping a daily schedule as a safety net | About $2.30, and builds start in minutes instead of up to 1–2 hours | Backend change and image rebuild, so it ships after this run. Must cover job creation, worker claims and the Retry button, and the builder must stay running through retry waits |
| 1-year Compute Savings Plan, no upfront | About $6 | Commits about $24/month for a year |
| Frontend on Fargate Spot | About $6 | Brief page gap on interruption; needs cluster capacity providers; assumes the usual 70% discount |
| RDS reserved instance, no upfront | About $4 | One-year commitment |
| Run the web and frontend tasks on one ECS-managed EC2 server (`t3.small`) | About $17 | Largest instance swap, but a large change: 2 GB shared by both tasks, so the worst-case burst could crash them; different task networking; you patch the host; a host failure takes down both |
| Frontend on ARM Fargate | About $1.80 | Image build and task changes; nginx only, so low risk |
| Move RDS to `db.t4g.micro` | About $1.50 | Instance change and a few minutes of downtime; nothing else changes |

**No cheaper version available:** RDS gp3 storage costs the same as gp2 here ($0.115/GB). A network load balancer has the same hourly charge as the current one. The frontend is already Fargate's smallest size.

**Web task and builder on ARM Fargate (about $4.70, 20% off) is not recommended.** The builder publishes the reference hashes contributors' files must match, and must carry the backend's exact MAGPIE. Moving MAGPIE from x86 to ARM could change floating-point results and so the hashes, which would make workers decline tasks. MAGPIE's x86-only build target would also need overriding.

### Replacing the architecture

Birdtest's current design can't go much below about $60, because RDS, the load balancer and the public IPs cost about $47 together whatever else changes.

| Option | Saves per month | What you lose | Effort |
|---|---|---|---|
| Run the site and Postgres on one small server | **About $40–55** ($50–65 if randomracer moves there too) | Managed RDS and point-in-time recovery; automatic failover of tasks; you own OS patching and Postgres; every deploy and restart takes the whole site down; the deploy, backup and restore tooling must be rewritten | Very large |

The range is 2–4 GB EC2 pricing with the builder left on Fargate. The 2 GB server gives the higher saving but is the riskier choice, because the database and the worst-case web burst would share that memory. Lightsail or a non-AWS provider might be cheaper, but neither has been priced or checked here.

### What each level would cost

| Choices, each added to the row above | Monthly bill |
|---|---|
| This plan | **About $94** |
| Shut down randomracer | About $85 |
| Put the frontend inside the web task | About $72 |
| Web task at 0.25 vCPU, 2 GB | About $65 |
| Run builds only by hand | About $62 |
| *Or instead:* one server, randomracer kept | About $40–55 |
| *Or instead:* one server, randomracer shut down | About $30–45 |

### Rejected

| Option | Saves per month | Why not |
|---|---|---|
| Move the frontend to S3 and CloudFront | About $13 | Same saving as merging it into the web task and keeps separate deploys, but puts a new layer in front of worker traffic and live updates |
| Replace the load balancer with API Gateway | About $22 | Its 10 MB and 30-second limits would break large worker results (up to 64 MiB) and live dashboards |
| Fargate Spot for the backend | About $15 | An interruption takes down the only API instance |
| Web task at 0.5 vCPU, 1 GB | About $3 | Risks a crash in a documented worst case for very little |

## Noticed, not part of this plan

- The nightly CI suite has failed on at least the last seven nights, in the `e2e (head)` job's tier-6 step. `deploy.sh` does not depend on it.
- randomracer has SSH open to the whole internet and runs Ubuntu 20.04, which is past standard support.
- The `infra/prod.tfvars` comment saying `db.t4g.micro` isn't offered for PostgreSQL in us-east-1 is out of date: it is orderable on 16.13.
