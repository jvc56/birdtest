#!/usr/bin/env bash
# An interactive shell inside the VPC, with psql, pg_restore and DATABASE_URL:
# the ops task (infra/ops.tf) started with ECS Exec, for the RUNBOOK.md steps
# that are more than one batch of SQL -- a selective restore above all. The
# task stops by itself after SHELL_HOURS (default 4); leaving the shell does
# not stop it, so this stops it on exit.
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
aws ecs execute-command --cluster "$cluster" --task "$task_arn" --container ops \
  --interactive --command /bin/bash
