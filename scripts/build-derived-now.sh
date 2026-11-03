#!/usr/bin/env bash
# Start the derived-file builder now (infra/derived.tf), rather than waiting
# for its hourly schedule, and wait for it to finish.
#
#   scripts/build-derived-now.sh
#
# The same task the schedule starts -- the family's latest revision, in the
# service's subnets and security group -- so it builds exactly what a
# scheduled run would: what the `derived_data` queue holds, at most eight
# files a run. It honours the wait after a failed build (5, then 15 minutes):
# a row still inside its wait is not taken, and a run started then finds
# nothing to do. A scheduled run that overlaps it is safe; the queue's lease
# keeps the two off the same row.
#
# Exits with the builder's exit code. 0 means the run finished, not that every
# build succeeded: a build's own failure is recorded on its row, which the
# admin "Derived data" page shows. Waits without a cap, since a rack info
# table can outlast ECS's ten-minute waiter; interrupting this script does
# not stop the task.
#
# Works on whatever stack infra/'s workspace is (BIRDTEST_WORKSPACE=dr for
# RUNBOOK §5's copy). Needs aws, terraform, jq, the settings in
# ~/.birdtest-env and infra/ initialized with the stack's backend.
set -euo pipefail
# shellcheck source=scripts/lib/ops.sh
source "$(dirname "$0")/lib/ops.sh"

[[ $# == 0 ]] || { sed -n '5p' "$0" >&2; exit 2; }

ops_init
ops_need aws terraform jq
ops_load_env
ops_login
ops_stack

family=$OPS_CLUSTER-derived-builder
subnets=$(tf -json service_subnet_ids | jq -r 'join(",")')
log_group=$(tf -raw log_group_name)
task=$(aws ecs run-task --cluster "$OPS_CLUSTER" --task-definition "$family" \
  --launch-type FARGATE \
  --network-configuration "awsvpcConfiguration={subnets=[$subnets],securityGroups=[$(tf -raw service_security_group_id)],assignPublicIp=ENABLED}" \
  --query 'tasks[0].taskArn' --output text)
[[ -n "$task" && "$task" != None ]] || ops_die "the builder task did not start"
ops_say "started $family: $task"

# Polled without a cap, as scripts/prod-sql.sh does and for the same reasons:
# `aws ecs wait tasks-stopped` gives up after ten minutes. A task ECS does not
# describe reads `None` -- just after run-task, or long after it stopped --
# and counts as stopped once it has been seen, or after five minutes of it. A
# failed call reads empty and is asked again.
seen=false
unseen=0
while :; do
  sleep 10
  status=$(aws ecs describe-tasks --cluster "$OPS_CLUSTER" --tasks "$task" \
    --query 'tasks[0].lastStatus' --output text 2>/dev/null) || status=""
  [[ "$status" == STOPPED ]] && break
  if [[ "$status" == None ]]; then
    unseen=$((unseen + 1))
    { $seen || (( unseen >= 30 )); } && break
  elif [[ -n "$status" ]]; then
    seen=true
    unseen=0
  fi
done

# The builder's log, a few seconds after the stop so its last lines are in.
sleep 10
stream=derived-builder/derived-builder/${task##*/}
token=""
while :; do
  page=$(aws logs get-log-events --log-group-name "$log_group" --log-stream-name "$stream" \
    --start-from-head ${token:+--next-token "$token"} --output json 2>/dev/null) || {
    ops_say "no log from the task (it may never have started its container)"
    break
  }
  jq -r '.events[].message' <<<"$page"
  next=$(jq -r '.nextForwardToken' <<<"$page")
  [[ "$next" == "$token" ]] && break
  token=$next
done

exit_code=$(aws ecs describe-tasks --cluster "$OPS_CLUSTER" --tasks "$task" \
  --query 'tasks[0].containers[0].exitCode' --output text)
if [[ "$exit_code" == 0 ]]; then
  ops_say "the builder finished (exit 0); the admin \"Derived data\" page shows each row's state"
  exit 0
fi
if [[ "$exit_code" == None ]]; then
  ops_say "the task stopped without an exit code:"
  aws ecs describe-tasks --cluster "$OPS_CLUSTER" --tasks "$task" \
    --query 'tasks[0].[stoppedReason, containers[0].reason]' --output text >&2 || true
  exit 1
fi
ops_say "the builder exited $exit_code"
[[ "$exit_code" =~ ^[0-9]+$ ]] && exit "$exit_code"
exit 1
