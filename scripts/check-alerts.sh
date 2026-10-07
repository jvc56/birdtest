#!/usr/bin/env bash
# Check that the alarms reach you (README.md, "Check that the alarms reach
# you"): after the first apply, once the SNS subscription is confirmed, and
# after any change to the alerts topic.
#
#   scripts/check-alerts.sh
#
# Three mails should arrive at alert_email, and checking the inbox is yours:
#   1. the backup-staleness alarm, set to OK and then to ALARM by hand (it
#      goes back to its real state at its next evaluation, perhaps with an OK
#      mail);
#   2. a backup task started with a command that fails, whose failure the
#      -backup-failed rule turns into a mail -- the rule's TriggeredRules
#      metric is checked here, and FailedInvocations must be 0;
# and, without a mail, the -deploy-failed rule's pattern is tested against a
# failed deployment of the live service (ECS's own events cannot be sent by
# hand), and the database storage event subscription must be active.
#
# Works on whatever stack infra/'s workspace is (BIRDTEST_WORKSPACE=dr for
# RUNBOOK §5's copy, whose names carry its suffix). Needs aws, terraform, jq,
# python3, the settings in ~/.birdtest-env and infra/ initialized.
set -euo pipefail
# shellcheck source=scripts/lib/ops.sh
source "$(dirname "$0")/lib/ops.sh"

[[ $# == 0 ]] || { sed -n '6p' "$0" >&2; exit 2; }

ops_init
ops_need aws terraform jq python3
ops_load_env
ops_login
ops_stack
name=$OPS_CLUSTER   # birdtest, or birdtest-dr: the stack's name and suffix
failed=0
fail() { ops_say "FAILED: $*"; failed=1; }

# 1. The staleness alarm. To OK first: a fresh stack's is already in ALARM
# (no backup has run), and setting the state it is in sends nothing.
aws cloudwatch set-alarm-state --alarm-name "$name-backup-stale" \
  --state-value OK --state-reason "testing the alert path (scripts/check-alerts.sh)"
aws cloudwatch set-alarm-state --alarm-name "$name-backup-stale" \
  --state-value ALARM --state-reason "testing the alert path (scripts/check-alerts.sh)"
ops_say "set $name-backup-stale to ALARM: a mail should arrive"

status=$(aws rds describe-event-subscriptions --subscription-name "$name-db-storage" \
  --query 'EventSubscriptionsList[0].Status' --output text 2>/dev/null) || status=""
if [[ "$status" == active ]]; then
  ops_say "the database storage event subscription is active"
else
  fail "the $name-db-storage event subscription is '${status:-missing}', not active"
fi

# 2. A backup that fails. The task's entry point is `bash -c`, so the
# override is the whole script, one string.
started=$(date -u +%s)
subnets=$(tf -json service_subnet_ids | jq -r 'join(",")')
task=$(aws ecs run-task --cluster "$OPS_CLUSTER" --task-definition "$(tf -raw backup_task_definition)" \
  --launch-type FARGATE \
  --network-configuration "awsvpcConfiguration={subnets=[$subnets],securityGroups=[$(tf -raw service_security_group_id)],assignPublicIp=ENABLED}" \
  --overrides '{"containerOverrides":[{"name":"backup","command":["exit 1"]}]}' \
  --query 'tasks[0].taskArn' --output text)
[[ -n "$task" && "$task" != None ]] || ops_die "the backup task did not start"
ops_say "started a backup task that fails: $task"

iso() { python3 -c 'import sys, datetime; print(datetime.datetime.fromtimestamp(int(sys.argv[1]), datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"))' "$1"; }
metric() {  # metric name -> the rule's sum since the task started
  aws cloudwatch get-metric-statistics --namespace AWS/Events --metric-name "$1" \
    --dimensions "Name=RuleName,Value=$name-backup-failed" \
    --start-time "$(iso $((started - 300)))" --end-time "$(iso $(($(date -u +%s) + 60)))" \
    --period 60 --statistics Sum --query 'sum(Datapoints[].Sum)' --output text 2>/dev/null
}
triggered=""
for _ in $(seq 40); do   # the task stops in a minute or two; the metric lags
  sleep 15
  t=$(metric TriggeredRules) || t=""
  if [[ "$t" =~ ^[1-9] ]]; then triggered=$t; break; fi
done
if [[ -n "$triggered" ]]; then
  ops_say "$name-backup-failed fired: a mail should arrive"
  f=$(metric FailedInvocations) || f=""
  [[ -z "$f" || "$f" == None || "$f" =~ ^0(\.0)?$ ]] \
    || fail "$name-backup-failed's FailedInvocations is $f: the topic refused the event (its policy?)"
else
  fail "$name-backup-failed did not fire within ten minutes (TriggeredRules is 0)"
fi

# 3. The deploy-failed rule against a failed deployment of the live service.
service=$(aws ecs describe-services --cluster "$OPS_CLUSTER" --services "$OPS_SERVICE" \
  --query 'services[0].serviceArn' --output text)
match=$(aws events test-event-pattern \
  --event-pattern "$(aws events describe-rule --name "$name-deploy-failed" --query EventPattern --output text)" \
  --event "$(jq -nc --arg s "$service" --arg r "$OPS_REGION" '{id: "1", account: "123456789012",
     source: "aws.ecs", time: "2026-01-01T00:00:00Z", region: $r, resources: [$s],
     "detail-type": "ECS Deployment State Change", detail: {eventName: "SERVICE_DEPLOYMENT_FAILED"}}')" \
  --query Result --output text) || match=""
if [[ "$match" == True || "$match" == true ]]; then
  ops_say "$name-deploy-failed matches a failed deployment of $OPS_SERVICE"
else
  fail "$name-deploy-failed does not match a failed deployment of $OPS_SERVICE"
fi

if ((failed)); then
  ops_die "some checks failed (above)"
fi
ops_say "done. Two mails should now be in alert_email's inbox: $name-backup-stale ALARM, and the backup failure. If one is missing, find out why now."
