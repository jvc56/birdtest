# shellcheck shell=bash
#
# Shared by the operator scripts (scripts/deploy.sh, reset-prod-db.sh,
# rollback.sh, set-setting.sh and the rest; README.md, "Operator scripts"):
# the steps every one of them takes and none should take differently.
#
#   - settings and login: ~/.birdtest-env (BIRDTEST_ENV), AWS_PAGER off, and
#     `aws sts get-caller-identity`, which says to run `aws sso login` when
#     the login has expired;
#   - the stack: Terraform outputs from whatever backend `terraform -chdir=infra`
#     is initialized with (the S3 backend of LAUNCH_PLAN Step 5, or local
#     state), the region exported from them, and the workspace printed, with
#     anything but `default` refused unless BIRDTEST_WORKSPACE names it;
#   - prod.tfvars: fetched from the state bucket at the start, uploaded after
#     an apply, each with a diff, and the upload refused if someone else
#     changed the bucket's copy in between;
#   - the plan gate: `plan -out`, then `terraform show -json` scanned for a
#     database, bucket, network or key destroyed, replaced or forgotten (refused
#     unless BIRDTEST_ALLOW_DESTRUCTIVE_PLAN=yes, then a typed confirmation),
#     then a question before anything is applied;
#   - prompts read from /dev/tty (OPS_TTY in the tests), so nothing piped
#     into a script answers them;
#   - the two services, the backend's and the frontend's, and what each runs;
#     which of them a commit's changes reach (ops_changed_components);
#   - the rollout wait: each service's deployment settled, running the image
#     that was deployed (a circuit-breaker rollback is said out loud), then
#     both target groups healthy, past the AWS waiter's ten minutes;
#   - the release log, ~/birdtest-releases.log (BIRDTEST_RELEASE_LOG);
#   - the tool check, up front.
#
# Sourced, never run. The calling script sets `set -euo pipefail` and calls
# ops_init first.

if [[ -n "${OPS_LIB_LOADED:-}" ]]; then return 0; fi
OPS_LIB_LOADED=1

OPS_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
OPS_INFRA=${INFRA_DIR:-$OPS_ROOT/infra}
OPS_ENV_FILE=${BIRDTEST_ENV:-$HOME/.birdtest-env}
OPS_RELEASE_LOG=${BIRDTEST_RELEASE_LOG:-$HOME/birdtest-releases.log}
OPS_TFVARS=$OPS_INFRA/prod.tfvars
OPS_NAME=$(basename "$0")
# Every resource type whose loss is data loss or an outage: the database, a
# bucket (and its versioning, lock and replication settings), the network the
# database sits in, a KMS key the backups are encrypted with.
OPS_DANGEROUS_TYPES='^(aws_db_instance|aws_db_subnet_group|aws_s3_bucket.*|aws_vpc|aws_subnet|aws_kms_key)$'
export AWS_PAGER=""

ops_say() { printf '%s: %s\n' "$OPS_NAME" "$*" >&2; }
ops_die() { printf '%s: %s\n' "$OPS_NAME" "$*" >&2; exit 1; }

# A scratch directory, removed on exit, for plans, fetched copies and the like.
ops_init() {
  OPS_TMP=$(mktemp -d "${TMPDIR:-/tmp}/birdtest-ops.XXXXXX")
  trap 'rm -rf "$OPS_TMP"' EXIT
}

# ---------------------------------------------------------------------------
# Tools, settings, login
# ---------------------------------------------------------------------------

ops_need() {
  local missing="" t
  for t in "$@"; do
    command -v "$t" >/dev/null 2>&1 || missing="$missing $t"
  done
  [[ -z "$missing" ]] || ops_die "missing tools:$missing (README.md, \"Operator scripts\", lists them)"
}

ops_load_env() {
  [[ -r "$OPS_ENV_FILE" ]] \
    || ops_die "no $OPS_ENV_FILE: scripts/onboard-deployer.sh writes one (README.md, \"Operator scripts\")"
  # shellcheck source=/dev/null
  source "$OPS_ENV_FILE"
  export AWS_PAGER=""
}

# The settings named must be set (in ~/.birdtest-env).
ops_require() {
  local v
  for v in "$@"; do
    [[ -n "${!v:-}" ]] || ops_die "$v is not set in $OPS_ENV_FILE"
  done
}

ops_login() {
  local out
  if ! out=$(aws sts get-caller-identity --query '[Account, Arn]' --output text 2>&1); then
    printf '%s\n' "$out" >&2
    ops_die "not logged in to AWS: run  aws sso login${AWS_PROFILE:+ --profile $AWS_PROFILE}  and run this again"
  fi
  OPS_ACCOUNT=${out%%[[:space:]]*}
  ops_say "AWS: ${out#*[[:space:]]}"
  if [[ -n "${ACCOUNT:-}" && "$ACCOUNT" != "$OPS_ACCOUNT" ]]; then
    ops_die "logged in to account $OPS_ACCOUNT, but $OPS_ENV_FILE says ACCOUNT=$ACCOUNT"
  fi
}

# ---------------------------------------------------------------------------
# The stack, from Terraform's state
# ---------------------------------------------------------------------------

tf() { terraform -chdir="$OPS_INFRA" output "$@"; }

# Sets OPS_WORKSPACE, OPS_REGION (exported as AWS_REGION/AWS_DEFAULT_REGION),
# OPS_CLUSTER, OPS_SERVICE (the backend's), OPS_FRONTEND_SERVICE and
# OPS_VARFILES.
ops_stack() {
  OPS_WORKSPACE=$(terraform -chdir="$OPS_INFRA" workspace show) \
    || ops_die "terraform cannot read infra/: run  terraform -chdir=infra init  first"
  # RUNBOOK §5's copy lives in its own workspace, and a script run there by
  # mistake works on the copy -- or, with prod.tfvars alone, plans production's
  # settings onto it.
  if [[ "$OPS_WORKSPACE" != default && "$OPS_WORKSPACE" != "${BIRDTEST_WORKSPACE:-default}" ]]; then
    ops_die "the Terraform workspace is '$OPS_WORKSPACE', not default (RUNBOOK §5's copy?): run  terraform -chdir=infra workspace select default,  or set BIRDTEST_WORKSPACE=$OPS_WORKSPACE to act on it"
  fi
  OPS_REGION=$(tf -raw region 2>/dev/null) || OPS_REGION=""
  [[ -n "$OPS_REGION" ]] \
    || ops_die "no Terraform outputs: is infra/ initialized with the stack's backend (terraform -chdir=infra init), and the login current?"
  # Every AWS call in the stack's own region, not the CLI's default -- which,
  # during a region loss, is usually the region that was lost.
  export AWS_REGION=$OPS_REGION AWS_DEFAULT_REGION=$OPS_REGION
  OPS_CLUSTER=$(tf -raw cluster_name)
  # The backend's service, the cluster and the target groups share the stack's
  # name (infra/ecs.tf): by name, not by an output, so that a stack applied
  # before the frontend had a service of its own is named the same way.
  OPS_SERVICE=$OPS_CLUSTER
  OPS_FRONTEND_SERVICE=$OPS_CLUSTER-frontend
  OPS_VARFILES=(-var-file=prod.tfvars)
  if [[ "$OPS_WORKSPACE" != default ]]; then
    [[ -e "$OPS_INFRA/$OPS_WORKSPACE.tfvars" ]] \
      || ops_die "workspace $OPS_WORKSPACE has no infra/$OPS_WORKSPACE.tfvars to apply on top of prod.tfvars"
    OPS_VARFILES+=(-var-file="$OPS_WORKSPACE.tfvars")
  fi
  ops_say "workspace $OPS_WORKSPACE, region $OPS_REGION, cluster $OPS_CLUSTER"
}

# ---------------------------------------------------------------------------
# prod.tfvars, kept in the state bucket (LAUNCH_PLAN Step 9)
# ---------------------------------------------------------------------------

ops_tfvars_url() { printf 's3://%s/birdtest/prod.tfvars' "$STATE_BUCKET"; }

# Fetches the bucket's copy into infra/prod.tfvars. A local file that
# differs is shown, and replaced only when the operator agrees (it is kept
# beside it); the bucket's copy is the one the last deploy applied.
ops_tfvars_fetch() {
  [[ -n "${STATE_BUCKET:-}" && -n "${STATE_REGION:-}" ]] \
    || ops_die "STATE_BUCKET and STATE_REGION must be set in $OPS_ENV_FILE"
  OPS_TFVARS_BASE=$OPS_TMP/prod.tfvars.fetched
  aws s3 cp "$(ops_tfvars_url)" "$OPS_TFVARS_BASE" --region "$STATE_REGION" --only-show-errors \
    || ops_die "could not fetch $(ops_tfvars_url)"
  if [[ -e "$OPS_TFVARS" ]] && ! cmp -s "$OPS_TFVARS" "$OPS_TFVARS_BASE"; then
    ops_say "infra/prod.tfvars differs from the state bucket's copy:"
    diff -u --label "infra/prod.tfvars (this machine)" --label "$(ops_tfvars_url)" \
      "$OPS_TFVARS" "$OPS_TFVARS_BASE" >&2 || true
    ops_confirm "Use the bucket's copy (the one last applied)? The local file is kept beside it." \
      || ops_die "stopped: reconcile infra/prod.tfvars with the bucket's copy first"
    cp "$OPS_TFVARS" "$OPS_TFVARS.local-$(date -u +%Y%m%dT%H%M%SZ)"
  fi
  cp "$OPS_TFVARS_BASE" "$OPS_TFVARS"
}

# Uploads infra/prod.tfvars, showing what changes. Refused if the bucket's
# copy changed since ops_tfvars_fetch: two deployers at once, whose applies
# the state lock kept apart but whose settings it does not.
ops_tfvars_upload() {
  local now=$OPS_TMP/prod.tfvars.now
  aws s3 cp "$(ops_tfvars_url)" "$now" --region "$STATE_REGION" --only-show-errors \
    || ops_die "could not read $(ops_tfvars_url) back; upload infra/prod.tfvars by hand: aws s3 cp infra/prod.tfvars $(ops_tfvars_url) --region $STATE_REGION"
  if ! cmp -s "$now" "$OPS_TFVARS_BASE"; then
    diff -u --label "as fetched" --label "the bucket now" "$OPS_TFVARS_BASE" "$now" >&2 || true
    ops_die "the bucket's prod.tfvars changed while this ran (another deployer?): not uploaded. Merge the two by hand, then  aws s3 cp infra/prod.tfvars $(ops_tfvars_url) --region $STATE_REGION"
  fi
  if cmp -s "$now" "$OPS_TFVARS"; then
    ops_say "prod.tfvars unchanged; nothing to upload"
    return 0
  fi
  ops_say "uploading prod.tfvars:"
  diff -u --label "$(ops_tfvars_url)" --label "infra/prod.tfvars" "$now" "$OPS_TFVARS" >&2 || true
  aws s3 cp "$OPS_TFVARS" "$(ops_tfvars_url)" --region "$STATE_REGION" --only-show-errors \
    || ops_die "the upload failed; run it by hand: aws s3 cp infra/prod.tfvars $(ops_tfvars_url) --region $STATE_REGION"
  cp "$OPS_TFVARS" "$OPS_TFVARS_BASE"
}

# A variable's value in a tfvars file, without its quotes: one-line
# assignments only (the ones these scripts write and read).
ops_tfvar() {
  local key=$1 file=${2:-$OPS_TFVARS}
  awk -v k="$key" '
    {
      line = $0
      if (line !~ "^[ \t]*" k "[ \t]*=") next
      sub("^[ \t]*" k "[ \t]*=[ \t]*", "", line)
      sub(/[ \t]+#.*$/, "", line); sub(/[ \t]+$/, "", line)
      if (line ~ /^".*"$/) line = substr(line, 2, length(line) - 2)
      v = line; found = 1
    }
    END { if (found) print v }
  ' "$file"
}

# Sets KEY to the HCL value VALUE (already quoted if a string) in a tfvars
# file: the line replaced if there is one, appended otherwise. A key that is
# not a variable of infra/ is refused (a typo would be ignored by Terraform
# with only a warning), and so is one whose value spans lines.
ops_tfvars_set() {
  local key=$1 value=$2 file=${3:-$OPS_TFVARS}
  [[ "$key" =~ ^[a-z_][a-z0-9_]*$ ]] || ops_die "not a variable name: $key"
  grep -q "^variable \"$key\"" "$OPS_INFRA"/variables.tf \
    || ops_die "infra/variables.tf has no variable $key"
  [[ "$value" != *$'\n'* ]] || ops_die "a value on one line only"
  local current
  current=$(awk -v k="$key" '$0 ~ "^[ \t]*" k "[ \t]*=" { sub("^[^=]*=[ \t]*", ""); sub(/[ \t]+$/, ""); print }' "$file")
  case "$current" in
    "<<"* | "["*[!\]] | "[" | "{"*[!\}] | "{")
      ops_die "$key's value in $(basename "$file") spans lines: edit it by hand" ;;
  esac
  [[ "$(printf '%s\n' "$current" | grep -c .)" -le 1 ]] \
    || ops_die "$key is set more than once in $(basename "$file"): edit it by hand"
  if grep -Eq "^[[:space:]]*${key}[[:space:]]*=" "$file"; then
    # The value through the environment: `awk -v` would read its backslashes
    # as escapes.
    OPS_VALUE=$value awk -v k="$key" '
      $0 ~ "^[ \t]*" k "[ \t]*=" { print k " = " ENVIRON["OPS_VALUE"]; next } { print }
    ' "$file" > "$file.new" && mv "$file.new" "$file"
  else
    [[ ! -s "$file" || "$(tail -c1 "$file")" == "" ]] || printf '\n' >> "$file"
    printf '%s = %s\n' "$key" "$value" >> "$file"
  fi
}

# A value as HCL: numbers, booleans, null, lists, maps and quoted strings as
# given; anything else becomes a quoted string. One that HCL would read
# otherwise than it looks -- a quote, a backslash, a `${` or `%{` template --
# must be given already quoted, as HCL.
ops_hcl_value() {
  local v=$1 number='^-?[0-9]+(\.[0-9]+)?$'
  case $v in
    true | false | null | \"*\" | \[*\] | \{*\})
      printf '%s' "$v"; return 0 ;;
  esac
  if [[ "$v" =~ $number ]]; then
    printf '%s' "$v"
    return 0
  fi
  case $v in
    *[\"\\]* | *"\${"* | *"%{"*)
      ops_die "give this value as HCL, quoted and escaped: $v" ;;
  esac
  printf '"%s"' "$v"
}

# The three image variables and the MAGPIE floor, for the release log: as
# they are now, and the two images as they were when prod.tfvars was fetched
# (previous_*), so that the release a line replaced can be put back from that
# line alone (ops_release_images) -- the backend and the frontend need not be
# at one tag.
ops_release_fields() {
  printf 'backend_image=%s derived_builder_image=%s frontend_image=%s min_magpie_version=%s' \
    "$(ops_tfvar backend_image)" "$(ops_tfvar derived_builder_image)" \
    "$(ops_tfvar frontend_image)" "$(ops_tfvar min_magpie_version)"
  printf ' previous_backend_image=%s previous_frontend_image=%s' \
    "$(ops_tfvar backend_image "$OPS_TFVARS_BASE")" "$(ops_tfvar frontend_image "$OPS_TFVARS_BASE")"
}

# The image variables each service runs: the derived-data builder goes with
# the backend, at its tag (it is the backend image with another entrypoint,
# and Terraform refuses the two at different tags).
ops_service_images() {
  case $1 in
    backend) printf '%s\n' backend_image derived_builder_image ;;
    frontend) printf '%s\n' frontend_image ;;
    *) ops_die "not a service: $1" ;;
  esac
}

# Points the images of each SERVICE named (backend, frontend; both if none is)
# at TAG, in the repositories they already name.
ops_retag() {
  local tag=$1 svc v repos=""
  shift
  (($#)) || set -- backend frontend
  [[ "$tag" =~ ^[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}$ ]] || ops_die "not an image tag: $tag"
  for svc in "$@"; do
    case $svc in
      backend) repos+="${repos:+|}backend|derived-builder" ;;
      frontend) repos+="${repos:+|}frontend" ;;
      *) ops_die "not a service: $svc" ;;
    esac
  done
  sed -E "s#(birdtest-($repos)):[^\"]+\"#\\1:$tag\"#" "$OPS_TFVARS" > "$OPS_TFVARS.new"
  mv "$OPS_TFVARS.new" "$OPS_TFVARS"
  for svc in "$@"; do
    for v in $(ops_service_images "$svc"); do
      [[ "$(ops_tfvar "$v")" == *":$tag" ]] || ops_die "$v in prod.tfvars did not take the tag $tag: check its line"
    done
  done
}

ops_image_tag() { local img=$1; printf '%s' "${img##*:}"; }

# ---------------------------------------------------------------------------
# Prompts, from the terminal
# ---------------------------------------------------------------------------

ops_read() {
  if [[ -z "${OPS_TTY_OPEN:-}" ]]; then
    exec 9<"${OPS_TTY:-/dev/tty}" || ops_die "no terminal to ask on"
    OPS_TTY_OPEN=1
  fi
  IFS= read -r "$1" <&9
}

ops_confirm() {
  local answer=""
  printf '%s [y/N] ' "$1" >&2
  ops_read answer || answer=""
  [[ -z "${OPS_TTY:-}" ]] || printf '%s\n' "$answer" >&2
  [[ "$answer" == [yY] || "$answer" == [yY][eE][sS] ]]
}

# Refuses unless the operator types EXPECTED exactly.
ops_confirm_typed() {
  local expected=$1 what=$2 answer=""
  printf 'Type %s to %s: ' "$expected" "$what" >&2
  ops_read answer || answer=""
  [[ -z "${OPS_TTY:-}" ]] || printf '%s\n' "$answer" >&2
  [[ "$answer" == "$expected" ]] || ops_die "not confirmed (typed '$answer'): nothing done"
}

# ---------------------------------------------------------------------------
# The plan gate
# ---------------------------------------------------------------------------

# Plans to OPS_PLAN and asks before anything is applied; sets OPS_PLAN_EMPTY
# when there is nothing to apply, and OPS_PLAN_JSON to the plan as JSON.
ops_plan_gate() {
  OPS_PLAN=$OPS_TMP/birdtest.tfplan
  OPS_PLAN_JSON=$OPS_TMP/birdtest.tfplan.json
  OPS_PLAN_EMPTY=""
  local rc=0
  terraform -chdir="$OPS_INFRA" plan -input=false -detailed-exitcode "${OPS_VARFILES[@]}" \
    -out="$OPS_PLAN" || rc=$?
  case $rc in
    0) OPS_PLAN_EMPTY=1; echo '{}' > "$OPS_PLAN_JSON"; ops_say "the plan has no changes"; return 0 ;;
    2) ;;
    *) ops_die "terraform plan failed: nothing applied" ;;
  esac
  terraform -chdir="$OPS_INFRA" show -json "$OPS_PLAN" > "$OPS_PLAN_JSON" \
    || ops_die "terraform show -json failed: nothing applied"
  local dangerous
  dangerous=$(jq -r --arg re "$OPS_DANGEROUS_TYPES" '
    .resource_changes[]?
    | select(.type | test($re))
    | select(any(.change.actions[]; . == "delete" or . == "forget"))
    | "  \(.address): \(.change.actions | join(" then "))"' "$OPS_PLAN_JSON") \
    || ops_die "could not read the plan's JSON: nothing applied"
  if [[ -n "$dangerous" ]]; then
    ops_say "the plan destroys, replaces or forgets:"
    printf '%s\n' "$dangerous" >&2
    if [[ "${BIRDTEST_ALLOW_DESTRUCTIVE_PLAN:-}" != yes ]]; then
      ops_die "refused: stop and ask before applying this (LAUNCH_PLAN Part 3, README \"Deploying\"). Nothing was applied. Once it is understood, BIRDTEST_ALLOW_DESTRUCTIVE_PLAN=yes lets it through with a typed confirmation."
    fi
    ops_confirm_typed destroy "apply it anyway"
  fi
  ops_say "plan: $(jq -r '[.resource_changes[]?.change.actions | join("/")] | group_by(.) | map("\(length) \(.[0])") | join(", ")' "$OPS_PLAN_JSON")"
  ops_confirm "Apply this plan?" || ops_die "not applied: nothing changed"
}

# Whether the gated plan changes either service or its task definition.
ops_plan_touches_service() {
  jq -e '[.resource_changes[]?
          | select(.address | test("^aws_ecs_(service|task_definition)\\.(backend|frontend)$"))
          | select(.change.actions != ["no-op"])] | length > 0' "$OPS_PLAN_JSON" >/dev/null
}

ops_apply_plan() {
  terraform -chdir="$OPS_INFRA" apply -input=false "$OPS_PLAN"
}

# ---------------------------------------------------------------------------
# The services: the backend's (OPS_SERVICE) and the frontend's
# ---------------------------------------------------------------------------

# The backend's: the one a reset, a restore or a password rotation stops,
# starts or restarts. The frontend's holds no state and keeps serving pages.
ops_desired_count() {
  aws ecs describe-services --cluster "$OPS_CLUSTER" --services "$OPS_SERVICE" \
    --query 'services[0].desiredCount' --output text
}

# Stops the backend's task and waits until none runs. Terraform's state still
# says one: the next apply would start it again (deploy.sh starts it itself,
# after applying, since a saved plan made at one task does not set the count).
ops_stop_service() {
  ops_say "stopping the backend's service (desired count 0); pages still load, and the -backend-down alarm fires in ten minutes and clears once it is back"
  aws ecs update-service --cluster "$OPS_CLUSTER" --service "$OPS_SERVICE" --desired-count 0 \
    --query 'service.desiredCount' --output text >/dev/null
  local i running
  for i in $(seq 90); do
    running=$(aws ecs describe-services --cluster "$OPS_CLUSTER" --services "$OPS_SERVICE" \
      --query 'services[0].runningCount' --output text 2>/dev/null) || running=""
    [[ "$running" == 0 ]] && { ops_say "no backend task running"; return 0; }
    [[ $i == 1 ]] || ops_say "waiting for the backend's task to stop ($running running)"
    sleep 10
  done
  ops_die "the backend's service still runs a task after fifteen minutes: look at it before going on (it stays at desired count 0)"
}

ops_start_service() {
  ops_say "starting the backend's service (desired count 1)"
  aws ecs update-service --cluster "$OPS_CLUSTER" --service "$OPS_SERVICE" --desired-count 1 \
    --query 'service.desiredCount' --output text >/dev/null
}

# The image of CONTAINER in SERVICE's primary deployment; fails when there is
# no such service (the frontend's, on a stack applied before it had one).
ops_running_image() {
  local service=$1 container=$2 td
  td=$(aws ecs describe-services --cluster "$OPS_CLUSTER" --services "$service" \
    --query "services[0].deployments[?status=='PRIMARY'].taskDefinition | [0]" --output text) || return 1
  [[ -n "$td" && "$td" != None && "$td" != null ]] || return 1
  aws ecs describe-task-definition --task-definition "$td" \
    --query "taskDefinition.containerDefinitions[?name=='$container'].image | [0]" --output text
}

ops_running_backend_image() { ops_running_image "$OPS_SERVICE" backend; }
ops_running_frontend_image() { ops_running_image "$OPS_FRONTEND_SERVICE" frontend; }

# Waits for SERVICE's deployment to settle: a single deployment, COMPLETED.
# A circuit-breaker rollback settles too, on the old image.
ops_wait_settled() {
  local service=$1 i out lines state
  for i in $(seq 240); do
    out=$(aws ecs describe-services --cluster "$OPS_CLUSTER" --services "$service" \
      --query 'services[0].deployments[].[status, rolloutState]' --output text 2>/dev/null) || out=""
    # No such service (the frontend's, before the apply that makes it).
    if [[ "$out" == None ]]; then
      ops_say "no service $service: nothing to wait for"
      return 0
    fi
    lines=$(printf '%s\n' "$out" | grep -c . || true)
    state=$(printf '%s\n' "$out" | awk 'NR == 1 { print $2 }')
    if [[ "$lines" == 1 && "$state" == COMPLETED ]]; then return 0; fi
    if [[ $i == 240 ]]; then
      ops_die "$service's deployment has not settled after an hour: see  aws ecs describe-services --cluster $OPS_CLUSTER --services $service"
    fi
    sleep 15
  done
}

# Waits for both services' deployments to settle, then checks each runs what
# was deployed -- BACKEND and FRONTEND, the images prod.tfvars names; an
# empty one is not checked -- then waits for both target groups. A
# circuit-breaker rollback is said here, with what to do.
ops_wait_release() {
  local expected_backend=$1 expected_frontend=${2:-} service container expected running
  ops_say "waiting for the deployments to settle (the backend's startup is allowed ten minutes; three failed starts roll a service back)"
  # ECS is eventually consistent: asked at once, it can still describe only
  # the old deployment, settled, which would read as a rollback.
  sleep 20
  for service in "$OPS_SERVICE" "$OPS_FRONTEND_SERVICE"; do
    ops_wait_settled "$service"
  done
  for service in "$OPS_SERVICE" "$OPS_FRONTEND_SERVICE"; do
    if [[ "$service" == "$OPS_SERVICE" ]]; then
      container=backend expected=$expected_backend
    else
      container=frontend expected=$expected_frontend
    fi
    [[ -n "$expected" ]] || continue
    running=$(ops_running_image "$service" "$container") || running=""
    if [[ "$running" != "$expected" ]]; then
      aws ecs describe-services --cluster "$OPS_CLUSTER" --services "$service" \
        --query 'services[0].events[:8].[createdAt, message]' --output text >&2 || true
      ops_die "ECS rolled the release back: $service runs $running, not $expected. Terraform still names the new release, so before any other apply run scripts/rollback.sh (RUNBOOK.md, \"Rolling back a deploy\")"
    fi
  done
  ops_wait_healthy
}

# Both target groups healthy. The AWS waiter gives up after ten minutes, as
# long as startup is allowed, so it is asked three times.
ops_wait_healthy() {
  local tg arn round ok
  for tg in backend frontend; do
    arn=$(aws elbv2 describe-target-groups --names "$OPS_CLUSTER-$tg" \
      --query 'TargetGroups[0].TargetGroupArn' --output text) \
      || ops_die "no target group $OPS_CLUSTER-$tg"
    ok=""
    for round in 1 2 3; do
      if aws elbv2 wait target-in-service --target-group-arn "$arn"; then ok=1; break; fi
      ops_say "$tg is not healthy yet (wait $round of 3)"
    done
    if [[ -z "$ok" ]]; then
      aws elbv2 describe-target-health --target-group-arn "$arn" --output table >&2 || true
      ops_die "$tg never became healthy: RUNBOOK.md, \"Rolling back a deploy\""
    fi
    ops_say "$tg healthy"
  done
}

# ---------------------------------------------------------------------------
# The release log
# ---------------------------------------------------------------------------

ops_log_release() {
  printf '%s %s\n' "$(date -u +%FT%TZ)" "$*" >> "$OPS_RELEASE_LOG"
}

# Tags the log names, newest first, each once: this file's lines
# (`deployed tag=... previous=...`, `rolled-back tag=...`) and the ones
# LAUNCH_PLAN's by-hand steps wrote (`deployed <tag>`, and the image lines
# `grep _image prod.tfvars` appended).
ops_logged_tags() {
  [[ -r "$OPS_RELEASE_LOG" ]] || return 0
  awk '
    # Per line, the older tag (previous=) before the newer one, so that read
    # backwards the newest comes first.
    {
      prev = ""; cur = ""
      for (i = 1; i <= NF; i++) {
        f = $i
        if (f ~ /^previous=/) { sub(/^previous=/, "", f); prev = f }
        else if (f ~ /^tag=/) { sub(/^tag=/, "", f); cur = f }
        else if (f == "deployed" && i + 1 == NF && $NF ~ /^[0-9a-f]+$/) cur = $NF
        else if (f ~ /birdtest-backend:/ && cur == "") { sub(/.*birdtest-backend:/, "", f); gsub(/"/, "", f); cur = f }
      }
      if (prev != "") tags[++n] = prev
      if (cur != "") tags[++n] = cur
    }
    END { for (i = n; i >= 1; i--) if (!(tags[i] in seen)) { seen[tags[i]] = 1; print tags[i] } }
  ' "$OPS_RELEASE_LOG"
}

# The min_magpie_version a logged release ran with, if the log says.
ops_logged_floor() {
  local tag=$1
  [[ -r "$OPS_RELEASE_LOG" ]] || return 0
  { grep -E "(^| )tag=$tag( |$)" "$OPS_RELEASE_LOG" || true; } | tail -1 \
    | sed -n -E 's/.*min_magpie_version=([^ ]*).*/\1/p'
}

# The backend's and the frontend's tags of a logged release, as "BACKEND
# FRONTEND": since a deploy rebuilds only what changed, release TAG may run
# a backend from an older commit. From the last line that deployed or rolled
# back to TAG (its image fields), else the last that replaced it
# (previous=TAG, with its previous_* fields), else -- a line from before the
# split, or none -- both at TAG.
ops_release_images() {
  local tag=$1 line b="" f=""
  if [[ -r "$OPS_RELEASE_LOG" ]]; then
    line=$({ grep -E "(^| )tag=$tag( |$)" "$OPS_RELEASE_LOG" || true; } \
      | { grep -E ' backend_image=[^ ]*:' || true; } | tail -1)
    if [[ -n "$line" ]]; then
      b=$(sed -n -E 's/.* backend_image=[^ ]*:([^ :]+)( .*|$)/\1/p' <<<"$line")
      f=$(sed -n -E 's/.* frontend_image=[^ ]*:([^ :]+)( .*|$)/\1/p' <<<"$line")
    else
      line=$({ grep -E "(^| )previous=$tag( |$)" "$OPS_RELEASE_LOG" || true; } \
        | { grep -E ' previous_backend_image=[^ ]*:' || true; } | tail -1)
      if [[ -n "$line" ]]; then
        b=$(sed -n -E 's/.* previous_backend_image=[^ ]*:([^ :]+)( .*|$)/\1/p' <<<"$line")
        f=$(sed -n -E 's/.* previous_frontend_image=[^ ]*:([^ :]+)( .*|$)/\1/p' <<<"$line")
      fi
    fi
  fi
  printf '%s %s' "${b:-$tag}" "${f:-$tag}"
}

# ---------------------------------------------------------------------------
# Schema changes (UPDATES_PLAN "Decided: schema changes keep editing 0001")
# ---------------------------------------------------------------------------

# Prints the migrations that FROM has and TO changed, renamed or removed:
# a database migrated by FROM's backend refuses TO's (sqlx checksums every
# applied migration). Added files are not listed. Returns 0 when there are
# some, 1 when there are none, 2 when FROM or TO is not a commit here.
ops_migrations_changed() {
  local from=$1 to=$2 changed
  git -C "$OPS_ROOT" cat-file -e "$from^{commit}" 2>/dev/null || return 2
  git -C "$OPS_ROOT" cat-file -e "$to^{commit}" 2>/dev/null || return 2
  changed=$(git -C "$OPS_ROOT" diff --name-status "$from" "$to" -- backend/migrations/) || return 2
  changed=$(printf '%s\n' "$changed" | awk 'NF && $1 !~ /^A/ { print "  " $0 }')
  [[ -n "$changed" ]] || return 1
  printf '%s\n' "$changed"
}

# ---------------------------------------------------------------------------
# What a commit's changes reach (deploy.sh)
# ---------------------------------------------------------------------------

# The parts of the stack the paths on stdin (repository-relative, one a line)
# reach, each printed once: `backend` (its image, and with it the derived-data
# builder's, which is the backend image with another entrypoint: backend/,
# docker/ -- the Dockerfile and its MAGPIE pin -- the root .dockerignore, and
# any Cargo file), `frontend` (frontend/, its own build context) and `infra`
# (infra/, and the scripts Terraform reads into task definitions with
# file()). Anything else -- docs, e2e/, worker/, the other scripts -- reaches
# nothing that is deployed.
ops_path_components() {
  awk '
    function hit(c) { if (!(c in seen)) { seen[c] = 1; print c } }
    /^frontend\//                                        { hit("frontend"); next }
    /^backend\// || /^docker\// || /^\.dockerignore$/    { hit("backend"); next }
    /(^|\/)Cargo\.[^\/]*$/                               { hit("backend"); next }
    /^infra\//                                           { hit("infra"); next }
    /^scripts\/(backup|restore-drill|restore-job)\.sh$/  { hit("infra"); next }
  '
}

# The parts of the stack (ops_path_components) that commit TO changes since
# FROM. Returns 2 when FROM or TO is not a commit here.
ops_changed_components() {
  local from=$1 to=$2 paths
  git -C "$OPS_ROOT" cat-file -e "$from^{commit}" 2>/dev/null || return 2
  git -C "$OPS_ROOT" cat-file -e "$to^{commit}" 2>/dev/null || return 2
  paths=$(git -C "$OPS_ROOT" diff --name-only --no-renames "$from" "$to") || return 2
  printf '%s\n' "$paths" | ops_path_components
}

# Of two tags, the one whose commit descends from the other's: the release
# the two together make up, for the release log's previous=. The first when
# neither is a commit here, or neither descends from the other.
ops_newer_tag() {
  local a=$1 b=$2
  if [[ "$a" != "$b" ]] && git -C "$OPS_ROOT" merge-base --is-ancestor "$a" "$b" 2>/dev/null; then
    printf '%s' "$b"
  else
    printf '%s' "$a"
  fi
}

# ---------------------------------------------------------------------------
# The database reset (reset-prod-db.sh, deploy.sh --reset-db)
# ---------------------------------------------------------------------------

# The site's hostname, from public_url; SITE from the settings must agree.
ops_site_host() {
  local url host
  url=$(ops_tfvar public_url)
  host=${url#https://}
  [[ -n "$host" && "$host" != "$url" ]] || ops_die "prod.tfvars has no https:// public_url"
  if [[ -n "${SITE:-}" && "$SITE" != "$host" ]]; then
    ops_die "SITE=$SITE in $OPS_ENV_FILE, but prod.tfvars' public_url is $url"
  fi
  printf '%s' "$host"
}

OPS_RESET_SQL="
-- Nothing else may hold the schema: the service is stopped, but a derived-data
-- build (every five minutes) or the 03:00 backup may be connected. Only this
-- role's sessions: RDS's own are not this database's.
SET client_min_messages = warning;
SET lock_timeout = '1min';
SELECT count(pg_terminate_backend(pid)) AS other_sessions_ended
  FROM pg_stat_activity
 WHERE datname = current_database() AND usename = current_user AND pid <> pg_backend_pid();
DROP SCHEMA public CASCADE;
CREATE SCHEMA public;
SELECT count(*) AS tables_left FROM pg_tables WHERE schemaname = 'public';
"

# ---------------------------------------------------------------------------
# The nightly dumps, which a reset deletes
# ---------------------------------------------------------------------------

# Sets OPS_DUMP_BUCKETS to the backups bucket and its DR replica, and
# OPS_DUMP_REGIONS to each one's region (the replica's is dr_region, which
# Terraform has no output for: S3 says).
ops_dump_buckets() {
  local b region
  OPS_DUMP_BUCKETS=("$(tf -raw backups_bucket)" "$(tf -raw backups_dr_bucket)")
  OPS_DUMP_REGIONS=()
  for b in "${OPS_DUMP_BUCKETS[@]}"; do
    [[ -n "$b" ]] || ops_die "no backups_bucket or backups_dr_bucket output from Terraform"
    region=$(aws s3api get-bucket-location --bucket "$b" --query LocationConstraint --output text) \
      || ops_die "could not read the region of $b (s3:GetBucketLocation)"
    # us-east-1 has no location constraint, and reads as None.
    [[ -n "$region" && "$region" != None && "$region" != null ]] || region=us-east-1
    OPS_DUMP_REGIONS+=("$region")
  done
}

# What pg/ holds in BUCKET, as "<dumps> <object versions> <delete markers>":
# a dump counted once by its top-level manifest, pg/<stamp>.manifest.json,
# which scripts/backup.sh writes last, whatever versions of it there are.
# Every page of the listing (the CLI follows them).
ops_dump_census() {
  local bucket=$1 region=$2 listing=$OPS_TMP/dump-census.json
  aws s3api list-object-versions --bucket "$bucket" --region "$region" --prefix pg/ \
    --output json > "$listing" \
    || ops_die "could not list the dumps in $bucket (s3:ListBucketVersions)"
  jq -rs '(.[0] // {}) as $r
    | ([($r.Versions // [])[].Key | select(test("^pg/[^/]+\\.manifest\\.json$"))] | unique | length) as $dumps
    | "\($dumps) \(($r.Versions // []) | length) \(($r.DeleteMarkers // []) | length)"' "$listing"
}

# Deletes every object version and delete marker under pg/ in BUCKET: a page
# of at most 1000 at a time (one ListObjectVersions response, and
# DeleteObjects' own limit), then the listing again from the start, until it
# is empty -- what was deleted is gone from it, so no continuation marker is
# kept. By version, so the bytes go and not just the current object: a plain
# delete would only add a delete marker. Each request bypasses the governance
# Object Lock both buckets hold (a replica carries its source's retention),
# which s3:BypassGovernanceRetention allows; and each bucket is done itself,
# since a delete by version is never replicated.
ops_delete_dumps() {
  local bucket=$1 region=$2 page=$OPS_TMP/dump-page.json batch=$OPS_TMP/dump-batch.json
  local n out errors total=0 last="" sum
  while :; do
    aws s3api list-object-versions --bucket "$bucket" --region "$region" --prefix pg/ \
      --no-paginate --output json > "$page" \
      || ops_die "could not list the dumps in $bucket; $total deleted so far"
    jq -s '{Objects: [(.[0] // {}) | (.Versions // [])[], (.DeleteMarkers // [])[]
                      | {Key, VersionId}], Quiet: true}' "$page" > "$batch"
    n=$(jq '.Objects | length' "$batch")
    ((n > 0)) || break
    ((n <= 1000)) || ops_die "a listing of $bucket returned $n versions, more than one delete takes"
    # The same page twice would be a loop that never ends.
    sum=$(cksum < "$batch")
    [[ "$sum" != "$last" ]] || ops_die "the same versions in $bucket came back after they were deleted"
    last=$sum
    out=$(aws s3api delete-objects --bucket "$bucket" --region "$region" \
      --delete "file://$batch" --bypass-governance-retention --output json) \
      || ops_die "deleting dumps in $bucket failed; $total deleted so far"
    errors=$(printf '%s' "$out" | jq -rs '(.[0].Errors // [])[] | "  \(.Key) \(.VersionId // ""): \(.Code) \(.Message // "")"')
    if [[ -n "$errors" ]]; then
      printf '%s\n' "$errors" | head -5 >&2
      ops_die "$(printf '%s\n' "$errors" | grep -c .) versions in $bucket were not deleted (above, the first five); $total deleted so far"
    fi
    total=$((total + n))
  done
  ops_say "deleted $total object versions and delete markers under pg/ in $bucket"
}

# Asks for the hostname, stops the backend's service, empties the schema and
# deletes the nightly dumps in both buckets. The backend is left stopped: the
# caller starts it (it applies 0001 to the empty schema as it starts). The
# frontend's service is not touched: pages load, and say the API is away.
ops_reset_database() {
  local host i census dumps=0 lines="" pitr
  host=$(ops_site_host)
  ops_dump_buckets
  for i in "${!OPS_DUMP_BUCKETS[@]}"; do
    census=$(ops_dump_census "${OPS_DUMP_BUCKETS[$i]}" "${OPS_DUMP_REGIONS[$i]}")
    read -r -a census <<<"$census"
    [[ ${#census[@]} == 3 ]] || ops_die "could not count the dumps in ${OPS_DUMP_BUCKETS[$i]}"
    dumps=$((dumps + census[0]))
    lines+=$(printf '\n    %s (%s): %s dumps, %s object versions, %s delete markers' \
      "${OPS_DUMP_BUCKETS[$i]}" "${OPS_DUMP_REGIONS[$i]}" "${census[0]}" "${census[1]}" "${census[2]}")
  done
  pitr=$(ops_tfvar db_backup_retention_days)
  pitr=${pitr:-30}
  cat >&2 <<EOF

  RESETTING THE PRODUCTION DATABASE of $host
  Every account, admin flag, API key, job, result and the imported input
  data are deleted: the schema is dropped and the backend makes it again.
  The nightly dumps are deleted too, every version under pg/, in the backups
  bucket and its DR replica:$lines
  Not RDS's automated backups: a point-in-time restore to before the reset
  (RUNBOOK §2) stays possible until they expire, $pitr days from now
  (db_backup_retention_days).

EOF
  ops_confirm_typed "$host" \
    "reset its database and delete the dumps in ${OPS_DUMP_BUCKETS[0]} and ${OPS_DUMP_BUCKETS[1]} ($dumps in all)"
  ops_stop_service
  ops_say "dropping and recreating the public schema (through the ops task)"
  INFRA_DIR=$OPS_INFRA "$OPS_ROOT/scripts/prod-sql.sh" "$OPS_RESET_SQL" \
    || ops_die "the reset failed (nothing was dropped if it says so above), and no dump was deleted. The backend is stopped: run this again, or start it with  aws ecs update-service --cluster $OPS_CLUSTER --service $OPS_SERVICE --desired-count 1"
  # After the drop, not before: it ended any 03:00 backup's session, so no
  # dump of the old database is written after these are gone.
  for i in "${!OPS_DUMP_BUCKETS[@]}"; do
    ( ops_delete_dumps "${OPS_DUMP_BUCKETS[$i]}" "${OPS_DUMP_REGIONS[$i]}" ) \
      || ops_die "the database is reset, but dumps remain in ${OPS_DUMP_BUCKETS[$i]} (above). The backend is stopped: run the same command again, which resets the empty database again and deletes the rest"
  done
  ops_log_release "reset-db host=$host dumps_deleted=$dumps"
}

# ---------------------------------------------------------------------------
# Shipping a change to prod.tfvars (deploy.sh, rollback.sh, set-setting.sh)
# ---------------------------------------------------------------------------

# With infra/prod.tfvars already edited: the plan gate; with OPS_RESET=1 the
# database reset, once the plan is approved and before the new task starts;
# the apply; the service started again after a reset (a plan saved while it
# ran one task does not set the count back); the upload; the release log
# line LOG_LINE; and the wait for the release to be healthy, each service
# running the image prod.tfvars now names.
ops_ship() {
  local log_line=$1 desired
  ops_plan_gate
  if [[ "${OPS_RESET:-0}" == 1 ]]; then
    ops_reset_database
  fi
  if [[ -z "$OPS_PLAN_EMPTY" ]]; then
    if ! ops_apply_plan; then
      if [[ "${OPS_RESET:-0}" == 1 ]]; then
        ops_say "the database has been reset and the backend's service is stopped."
      fi
      ops_die "terraform apply failed. prod.tfvars was not uploaded (infra/prod.tfvars has the change): fix the cause and run this again"
    fi
  fi
  if [[ "${OPS_RESET:-0}" == 1 ]]; then
    ops_start_service
  fi
  ops_tfvars_upload
  ops_log_release "$log_line"
  desired=$(ops_desired_count) || desired=""
  if [[ "$desired" == 0 ]]; then
    ops_say "the backend's service is at desired count 0: nothing to wait for"
  elif [[ "${OPS_RESET:-0}" == 1 ]] || { [[ -z "$OPS_PLAN_EMPTY" ]] && ops_plan_touches_service; }; then
    ops_wait_release "$(ops_tfvar backend_image)" "$(ops_tfvar frontend_image)"
    if command -v curl >/dev/null 2>&1; then
      local host
      host=$(ops_site_host)
      if curl -fsS --max-time 20 "https://$host/health" >/dev/null; then
        ops_say "https://$host/health answers"
      else
        ops_say "WARNING: https://$host/health did not answer: look at the site"
      fi
    fi
  fi
}

ops_reset_checklist() {
  local host=$1 pin version=""
  pin=$(sed -n 's/^ARG MAGPIE_COMMIT=//p' "$OPS_ROOT/docker/Dockerfile" | head -1)
  if [[ -n "$pin" && -d "${MAGPIE_DIR:-$HOME/MAGPIE}/.git" ]]; then
    version=$(git -C "${MAGPIE_DIR:-$HOME/MAGPIE}" show "$pin:download_data.sh" 2>/dev/null \
      | sed -n -E 's/^DATA_VERSION="?([^"]*)"?.*/\1/p' | head -1) || version=""
  fi
  version=${version:-"<DATA_VERSION in download_data.sh at the MAGPIE pin>"}
  cat >&2 <<EOF

The database is empty. Now:
  1. Register again at https://$host, every admin.
  2. Confirm and promote each one:  scripts/confirm-user.sh --admin <username>
     (it confirms the address by hand while SES is in the sandbox; sign out
     and in again afterwards).
  3. Admin -> Input data: version $version (20260925 when this
     was written), ref main, Fetch and diff, confirm.
  4. Recreate the player configs and jobs.
  Any contributor's stored identity is gone too: a contribute.txt uuid line
  or an API key from before the reset is refused (401) until made again.
EOF
}
