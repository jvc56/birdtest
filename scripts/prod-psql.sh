#!/usr/bin/env bash
# Run SQL files against the production database, in the ops task, the way
# RUNBOOK.md's multi-step procedures need: psql variables (-v job=<id>, read
# as :'job'), ON_ERROR_STOP, and no forced transaction -- each file says
# BEGIN and COMMIT where it means them, and a file whose loops commit as
# they go can run. scripts/prod-sql.sh stays the tool for one batch of
# statements in one transaction.
#
#   scripts/prod-psql.sh [-v NAME=VALUE]... [--keep HOURS] FILE.sql...
#   scripts/prod-psql.sh --task TASK_ARN [-v NAME=VALUE]... FILE.sql...
#   scripts/prod-psql.sh --follow TASK_ARN [RUN]
#   scripts/prod-psql.sh --keep HOURS
#
# The files run in order, in one psql session (`psql -X -v ON_ERROR_STOP=1
# -v NAME=VALUE... -f FILE...`), which stops at the first error.
#
# By default a new ops task starts with the files in its environment
# (compressed: about 8 KB of overrides in all, which ECS caps), runs them as
# its own command and stops. The task is the detached part: closing this
# terminal, or the laptop sleeping, does not stop the SQL, and
#   scripts/prod-psql.sh --follow <task-arn>
# picks up its output again. What psql prints is read back from CloudWatch
# as it arrives -- so never select personal columns (addresses, hashes): the
# log keeps them for thirty days.
#
# --keep HOURS leaves the task running for that long after the files finish
# (or, with no files, just starts one), with ECS Exec on, so that /tmp --
# files a step wrote with \copy or \o -- outlives the step: a later run with
# --task <arn> uses the same task, and scripts/prod-shell.sh --attach <arn>
# opens a shell in it.
#
# --task TASK_ARN runs the files in a task that is already running: one that
# --keep started, or a scripts/prod-shell.sh shell's, where RUNBOOK §2.1's
# scratch copy lives (its /tmp/restore.env is read first, so the SQL sees
# SCRATCH_URL through \getenv). The files go over ECS Exec in pieces (it
# needs the Session Manager plugin), are checked against their SHA-256, and
# run detached in the task, their output joining the task's log.
#
# A file containing the line `-- prod-psql: writes` is confirmed before it
# runs; one containing `-- prod-psql: needs NAME...` is refused without those
# variables. Needs the AWS CLI, jq, and the Terraform state in infra/ (or
# INFRA_DIR).
set -euo pipefail
# shellcheck source=scripts/lib/ops.sh
source "$(dirname "$0")/lib/ops.sh"

usage() { sed -n '9,12p' "$0" >&2; exit 2; }

vars=() files=() keep="" task="" follow="" follow_run=""
while (($#)); do
  case $1 in
    -v) [[ $# -ge 2 && "$2" == *=* ]] || usage; vars+=("$2"); shift ;;
    -v*=*) vars+=("${1#-v}") ;;
    --keep) [[ $# -ge 2 && "$2" =~ ^[1-9][0-9]?$ ]] || usage; keep=$2; shift ;;
    --task) [[ $# -ge 2 ]] || usage; task=$2; shift ;;
    --follow) [[ $# -ge 2 ]] || usage; follow=$2; follow_run=${3:-}; break ;;
    -h | --help) sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//' >&2; exit 0 ;;
    -*) usage ;;
    *) files+=("$1") ;;
  esac
  shift
done
if [[ -n "$task" && -n "$keep" ]]; then usage; fi
if [[ -z "$follow" && -z "$keep" ]] && ((${#files[@]} == 0)); then usage; fi

# Checked here, before anything starts.
for kv in ${vars[@]+"${vars[@]}"}; do
  [[ "${kv%%=*}" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]] || ops_die "not a psql variable name: ${kv%%=*}"
  [[ "${kv%%=*}" != ON_ERROR_STOP ]] || ops_die "ON_ERROR_STOP is always on"
done
writes=0
for f in ${files[@]+"${files[@]}"}; do
  [[ -f "$f" && -r "$f" ]] || ops_die "no such file: $f"
  # shellcheck disable=SC2013 # words, not lines: `needs a b c`
  for need in $(sed -n 's/^-- prod-psql: needs //p' "$f"); do
    found=0
    for kv in ${vars[@]+"${vars[@]}"}; do [[ "${kv%%=*}" == "$need" ]] && found=1; done
    ((found)) || ops_die "$f needs -v $need=<value>"
  done
  grep -qx -- '-- prod-psql: writes' "$f" && writes=1
done

ops_init
ops_need aws terraform jq
if [[ -n "$task" ]]; then ops_need session-manager-plugin; fi
ops_stack
task_definition=$(tf -raw ops_task_definition)
log_group=$(tf -raw log_group_name)

# Prints a run's output from the task's log as it arrives: the lines between
# its start and exit markers (RUN empty: every line), until the exit marker
# or the task stops. Returns psql's exit status.
follow_run() {
  local arn=$1 run=$2 stream token="" next page line rc="" started=0 status stopped=0 unseen=0
  stream="ops/ops/${arn##*/}"
  [[ -n "$run" ]] || started=1
  while :; do
    page=$(aws logs get-log-events --log-group-name "$log_group" --log-stream-name "$stream" \
      --start-from-head ${token:+--next-token "$token"} --output json 2>/dev/null) || page=""
    next=$token
    if [[ -n "$page" ]]; then
      while IFS= read -r line; do
        if [[ -n "$run" && "$line" == "__birdtest_run $run start" ]]; then started=1; continue; fi
        if [[ -n "$run" && "$line" == "__birdtest_run $run exit "* ]]; then rc=${line##* }; break; fi
        ((started)) && printf '%s\n' "$line"
      done < <(jq -r '.events[].message' <<<"$page")
      next=$(jq -r '.nextForwardToken // empty' <<<"$page")
    fi
    if [[ -n "$rc" ]]; then
      [[ "$rc" == 0 ]] && return 0
      [[ "$rc" =~ ^[0-9]+$ ]] || { ops_say "the run did not start in the task ($rc)"; return 1; }
      ops_say "psql exited $rc"
      return 1
    fi
    if [[ -n "$next" && "$next" != "$token" ]]; then token=$next; continue; fi
    ((stopped)) && break
    status=$(aws ecs describe-tasks --cluster "$OPS_CLUSTER" --tasks "$arn" \
      --query 'tasks[0].lastStatus' --output text 2>/dev/null) || status=""
    # A task ECS no longer describes reads None: just after run-task, or an
    # hour after it stopped. Read the log once more either way, a few seconds
    # on: CloudWatch takes that long to take in the last lines.
    if [[ "$status" == STOPPED ]]; then stopped=1
    elif [[ "$status" == None ]]; then unseen=$((unseen + 1)); ((unseen < 30)) || stopped=1
    else unseen=0
    fi
    sleep 10
  done
  if [[ -z "$run" ]]; then return 0; fi
  ops_say "the task stopped without the run's end in its log:"
  aws ecs describe-tasks --cluster "$OPS_CLUSTER" --tasks "$arn" \
    --query 'tasks[0].[stoppedReason, containers[0].exitCode]' --output text >&2 || true
  return 1
}

if [[ -n "$follow" ]]; then
  follow_run "$follow" "$follow_run"
  exit
fi

run="run-$(date -u +%Y%m%dT%H%M%SZ)-$RANDOM"
subnets=$(tf -json service_subnet_ids | jq -r 'join(",")')
security_group=$(tf -raw service_security_group_id)

payload=""
if ((${#files[@]})); then
  ops_say "files: ${files[*]}"
  ((${#vars[@]} == 0)) || ops_say "variables: ${vars[*]}"
  if ((writes)); then
    ops_confirm "These write to the production database. Run them?" || ops_die "nothing run"
  fi
  # The payload: the files, the variables (a file each, so that no value is
  # ever parsed by a shell) and the script that runs them in the task.
  mkdir -p "$OPS_TMP/p/sql" "$OPS_TMP/p/vars"
  i=0
  for f in "${files[@]}"; do
    i=$((i + 1))
    cp "$f" "$OPS_TMP/p/sql/$(printf '%02d' "$i")-$(basename "$f" | tr -c 'A-Za-z0-9._\n-' _)"
  done
  for kv in ${vars[@]+"${vars[@]}"}; do
    printf '%s' "${kv#*=}" > "$OPS_TMP/p/vars/${kv%%=*}"
  done
  printf '%s' "$run" > "$OPS_TMP/p/RUN"
  cat > "$OPS_TMP/p/run.sh" <<'RUN'
# Inside the ops task (the postgres image): psql over the files, in order.
cd "$(dirname "$0")" || exit 1
run=$(cat RUN)
echo "__birdtest_run $run start"
# A shell's scratch copy (RUNBOOK §2.1) is named in /tmp/restore.env.
if [ -r /tmp/restore.env ]; then . /tmp/restore.env; export SCRATCH_URL; fi
args=(-X -v ON_ERROR_STOP=1)
for v in vars/*; do
  [ -e "$v" ] || continue
  value=$(cat "$v"; printf x)
  args+=(-v "${v#vars/}=${value%x}")
done
for f in sql/*; do args+=(-f "$f"); done
psql "$DATABASE_URL" "${args[@]}"
rc=$?
echo "__birdtest_run $run exit $rc"
exit "$rc"
RUN
  payload=$(tar -czf - -C "$OPS_TMP/p" . | base64 | tr -d '\n')
fi

if [[ -z "$task" ]]; then
  # A new task, the files in its environment.
  if [[ -n "$payload" ]]; then
    command="mkdir -p /tmp/ops/$run && cd /tmp/ops/$run && printf %s \"\$BIRDTEST_PAYLOAD\" | base64 -d | tar -xzf - && bash run.sh; rc=\$?"
  else
    command="mkdir -p /tmp/ops; echo started; rc=0"
  fi
  if [[ -n "$keep" ]]; then command="$command; sleep $((keep * 3600))"; fi
  command="$command; exit \$rc"
  overrides=$(jq -nc --arg payload "$payload" --arg command "$command" '{
    containerOverrides: [{
      name: "ops",
      environment: (if $payload == "" then [] else [{ name: "BIRDTEST_PAYLOAD", value: $payload }] end),
      command: [$command]
    }]
  }')
  if ((${#overrides} > 8000)); then
    ops_die "the files are too large for a task's overrides (${#overrides} of about 8,000 bytes): start a task with --keep HOURS, then run them in it with --task <arn>"
  fi
  started=$(aws ecs run-task --cluster "$OPS_CLUSTER" --task-definition "$task_definition" \
    --launch-type FARGATE ${keep:+--enable-execute-command} \
    --network-configuration "awsvpcConfiguration={subnets=[$subnets],securityGroups=[$security_group],assignPublicIp=ENABLED}" \
    --overrides "$overrides" --output json)
  task=$(jq -r '.tasks[0].taskArn // empty' <<<"$started")
  if [[ -z "$task" ]]; then
    jq '.failures' <<<"$started" >&2
    ops_die "the task did not start"
  fi
  ops_say "task $task"
  [[ -z "$keep" ]] || ops_say "it runs for $keep hours: --task $task for the next files, scripts/prod-shell.sh --attach $task for a shell"
else
  # An existing task, over ECS Exec: the payload in pieces, then a starter
  # that checks it whole and runs it detached, into the task's own log.
  ops_say "task $task (ECS Exec)"
  exec_in() {
    aws ecs execute-command --cluster "$OPS_CLUSTER" --task "$task" --container ops \
      --interactive --command "$1" > "$OPS_TMP/exec.out" 2>&1 \
      || { cat "$OPS_TMP/exec.out" >&2; ops_die "ECS Exec into $task failed (is it running, with ECS Exec on?)"; }
  }
  sum=$(printf '%s' "$payload" | base64 -d 2>/dev/null | { sha256sum 2>/dev/null || shasum -a 256; } | cut -c1-64)
  exec_in "/bin/bash -c 'mkdir -p /tmp/ops && : > /tmp/ops/$run.b64'"
  rest=$payload
  while [[ -n "$rest" ]]; do
    exec_in "/bin/bash -c 'printf %s ${rest:0:4000} >> /tmp/ops/$run.b64'"
    rest=${rest:4000}
  done
  starter="trap '' HUP
cd /tmp/ops || exit 1
if [ \"\$(base64 -d $run.b64 | sha256sum | cut -c1-64)\" != $sum ]; then
  echo '__birdtest_run $run exit corrupt' > /proc/1/fd/1; exit 1
fi
if ! { mkdir $run && base64 -d $run.b64 | tar -xzf - -C $run; }; then
  echo '__birdtest_run $run exit unpacking' > /proc/1/fd/1; exit 1
fi
setsid nohup bash $run/run.sh > /proc/1/fd/1 2>&1 < /dev/null &
sleep 1"
  exec_in "/bin/bash -c 'printf %s $(printf '%s' "$starter" | base64 | tr -d '\n') | base64 -d | bash'"
fi

if [[ -n "$payload" ]]; then
  ops_say "following $run (Ctrl-C stops only this; the SQL runs on. Again: scripts/prod-psql.sh --follow $task $run)"
  follow_run "$task" "$run"
fi
