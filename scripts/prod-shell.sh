#!/usr/bin/env bash
# An interactive shell inside the VPC, with psql, pg_restore and DATABASE_URL:
# the ops task (infra/ops.tf) started with ECS Exec, for the RUNBOOK.md steps
# that are more than one batch of SQL -- a selective restore above all. The
# task stops by itself after SHELL_HOURS (default 4).
#
# Leaving the shell does not stop the task, and neither does this script
# unless asked: an ECS Exec session ends after twenty idle minutes, or when a
# laptop sleeps, and stopping the task then killed a restore running in it --
# hours of pg_restore, and the scratch copy on its disk. Run long commands
# under `setsid nohup ... > /tmp/<name>.log 2>&1 &` inside, and come back with
#
#   scripts/prod-shell.sh --attach <task-arn>
#
# Needs the AWS CLI with the Session Manager plugin, jq, and the Terraform
# state in infra/ (or INFRA_DIR). Inside, `apt-get update && apt-get install -y
# awscli` gives the shell the CLI for fetching a dump from the backups bucket.
set -euo pipefail

tf() { terraform -chdir="${INFRA_DIR:-infra}" output "$@"; }
# Every AWS call in the stack's own region, not the CLI's default -- which,
# during a region loss, is usually the region that was lost.
export AWS_REGION AWS_DEFAULT_REGION
AWS_REGION=$(tf -raw region)
AWS_DEFAULT_REGION=$AWS_REGION
cluster=$(tf -raw cluster_name)
# After a region-loss drill the workspace may still be the DR stack's.
echo "workspace $(terraform -chdir="${INFRA_DIR:-infra}" workspace show), region $AWS_REGION, cluster $cluster" >&2
task_definition=$(tf -raw ops_task_definition)
subnets=$(tf -json service_subnet_ids | jq -r 'join(",")')
security_group=$(tf -raw service_security_group_id)
seconds=$(( ${SHELL_HOURS:-4} * 3600 ))

attach() {
  aws ecs execute-command --cluster "$cluster" --task "$1" --container ops \
    --interactive --command /bin/bash || true
  echo >&2
  echo "the task goes on running until it stops itself (SHELL_HOURS). Again:" >&2
  echo "  scripts/prod-shell.sh --attach $1" >&2
  read -r -p "stop it now? [y/N] " answer </dev/tty || answer=n
  if [[ "$answer" == [yY]* ]]; then
    aws ecs stop-task --cluster "$cluster" --task "$1" >/dev/null
    echo "stopped" >&2
  fi
}

if [[ "${1:-}" == --attach ]]; then
  [[ -n "${2:-}" ]] || { echo "usage: $0 --attach <task-arn>" >&2; exit 2; }
  attach "$2"
  exit 0
fi

# ECS Exec needs the Session Manager plugin; without it the task would start
# and nothing could open a shell in it.
command -v session-manager-plugin >/dev/null \
  || { echo "install the AWS CLI's Session Manager plugin first" >&2; exit 1; }

overrides=$(jq -n --arg command "sleep $seconds" \
  '{containerOverrides: [{name: "ops", command: [$command]}]}')
started=$(aws ecs run-task --cluster "$cluster" --task-definition "$task_definition" \
  --launch-type FARGATE --enable-execute-command \
  --network-configuration "awsvpcConfiguration={subnets=[$subnets],securityGroups=[$security_group],assignPublicIp=ENABLED}" \
  --overrides "$overrides" --output json)
task_arn=$(jq -r '.tasks[0].taskArn // empty' <<<"$started")
if [[ -z "$task_arn" ]]; then
  echo "the task did not start:" >&2
  jq '.failures' <<<"$started" >&2
  exit 1
fi
# Stopped if this script fails before the shell opens; after that, only when
# asked (see above).
trap 'aws ecs stop-task --cluster "$cluster" --task "$task_arn" >/dev/null' EXIT
echo "starting $task_arn" >&2
aws ecs wait tasks-running --cluster "$cluster" --tasks "$task_arn"
# The exec agent comes up a little after the task reports running.
for _ in $(seq 60); do
  agent=$(aws ecs describe-tasks --cluster "$cluster" --tasks "$task_arn" \
    --query "tasks[0].containers[0].managedAgents[?name=='ExecuteCommandAgent'].lastStatus | [0]" \
    --output text)
  [[ "$agent" == RUNNING ]] && break
  sleep 5
done
# Still under the trap: a task nobody can get into is stopped, not left to
# sleep out SHELL_HOURS holding DATABASE_URL.
[[ "${agent:-}" == RUNNING ]] || { echo "the task's ECS Exec agent never started" >&2; exit 1; }
trap - EXIT
attach "$task_arn"
