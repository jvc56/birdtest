#!/usr/bin/env bash
# Run SQL against the production database, from inside the VPC.
#
#   scripts/prod-sql.sh "UPDATE users SET is_admin = true WHERE username = 'alice'"
#   scripts/prod-sql.sh < fix.sql
#
# The database is not publicly accessible and its security group admits only
# the service's, so nothing on an operator's machine can reach it -- no psql
# from a laptop, no bastion. This runs the ops task (infra/ops.tf): the
# postgres image (so a psql matching the server), DATABASE_URL from SSM, and
# the service's subnets and security group. Its command is psql reading the
# SQL given here, and what psql prints is read back from CloudWatch Logs.
# For anything interactive, scripts/prod-shell.sh opens a shell in the same
# task instead. Needs the AWS CLI, jq and the
# Terraform state in infra/ (or INFRA_DIR).
#
# Statements run with ON_ERROR_STOP, in one transaction (--single-transaction):
# anything that fails rolls the whole script back. The SQL travels as an
# environment override, which ECS caps at about 8 KB in total.
set -euo pipefail

sql=${1:-$(cat)}
[[ -n "$sql" ]] || { echo "usage: $0 'SQL' (or SQL on stdin)" >&2; exit 2; }

tf() { terraform -chdir="${INFRA_DIR:-infra}" output "$@"; }
cluster=$(tf -raw cluster_name)
# After a region-loss drill the workspace may still be the DR stack's.
echo "workspace $(terraform -chdir="${INFRA_DIR:-infra}" workspace show), cluster $cluster" >&2
task_definition=$(tf -raw ops_task_definition)
subnets=$(tf -json service_subnet_ids | jq -r 'join(",")')
security_group=$(tf -raw service_security_group_id)
log_group=$(tf -raw log_group_name)

overrides=$(jq -n --arg sql "$sql" '{
  containerOverrides: [{
    name: "ops",
    environment: [{ name: "BIRDTEST_SQL", value: $sql }],
    command: ["printf %s \"$BIRDTEST_SQL\" | psql \"$DATABASE_URL\" -X -v ON_ERROR_STOP=1 --single-transaction -f -"]
  }]
}')

started=$(aws ecs run-task --cluster "$cluster" --task-definition "$task_definition" \
  --launch-type FARGATE \
  --network-configuration "awsvpcConfiguration={subnets=[$subnets],securityGroups=[$security_group],assignPublicIp=ENABLED}" \
  --overrides "$overrides" --output json)
task_arn=$(jq -r '.tasks[0].taskArn // empty' <<<"$started")
if [[ -z "$task_arn" ]]; then
  echo "the task did not start:" >&2
  jq '.failures' <<<"$started" >&2
  exit 1
fi
echo "running $task_arn" >&2
aws ecs wait tasks-stopped --cluster "$cluster" --tasks "$task_arn"

# All of psql's output, page by page: one call returns at most 10,000 events
# or 1 MB. A task that never started its container has no stream at all, and
# its stopped reason says why.
task_id=${task_arn##*/}
token=""
while :; do
  page=$(aws logs get-log-events --log-group-name "$log_group" \
    --log-stream-name "ops/ops/$task_id" --start-from-head \
    ${token:+--next-token "$token"} --output json 2>/dev/null) || {
    echo "no output from the task:" >&2
    aws ecs describe-tasks --cluster "$cluster" --tasks "$task_arn" \
      --query 'tasks[0].[stoppedReason, containers[0].reason]' --output text >&2
    exit 1
  }
  jq -r '.events[].message' <<<"$page"
  next=$(jq -r '.nextForwardToken' <<<"$page")
  [[ "$next" == "$token" ]] && break
  token=$next
done

exit_code=$(aws ecs describe-tasks --cluster "$cluster" --tasks "$task_arn" \
  --query 'tasks[0].containers[0].exitCode' --output text)
[[ "$exit_code" == "0" ]] || { echo "psql exited $exit_code" >&2; exit 1; }
