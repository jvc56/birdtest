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
#   - the rollout wait: the service's deployment settled, running the image
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
# OPS_CLUSTER, OPS_SERVICE and OPS_VARFILES.
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
  # The service, its cluster and its target groups share the stack's name.
  OPS_SERVICE=$OPS_CLUSTER
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

# The three image variables and the MAGPIE floor, for the release log.
ops_release_fields() {
  printf 'backend_image=%s derived_builder_image=%s frontend_image=%s min_magpie_version=%s' \
    "$(ops_tfvar backend_image)" "$(ops_tfvar derived_builder_image)" \
    "$(ops_tfvar frontend_image)" "$(ops_tfvar min_magpie_version)"
}

# Points the three images at TAG, in the repositories they already name.
ops_retag() {
  local tag=$1
  [[ "$tag" =~ ^[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}$ ]] || ops_die "not an image tag: $tag"
  sed -E "s#(birdtest-(backend|derived-builder|frontend)):[^\"]+\"#\\1:$tag\"#" "$OPS_TFVARS" > "$OPS_TFVARS.new"
  mv "$OPS_TFVARS.new" "$OPS_TFVARS"
  local v
  for v in backend_image derived_builder_image frontend_image; do
    [[ "$(ops_tfvar "$v")" == *":$tag" ]] || ops_die "$v in prod.tfvars did not take the tag $tag: check its line"
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

# Whether the gated plan changes the web service or its task definition.
ops_plan_touches_service() {
  jq -e '[.resource_changes[]?
          | select(.address == "aws_ecs_service.main" or .address == "aws_ecs_task_definition.main")
          | select(.change.actions != ["no-op"])] | length > 0' "$OPS_PLAN_JSON" >/dev/null
}

ops_apply_plan() {
  terraform -chdir="$OPS_INFRA" apply -input=false "$OPS_PLAN"
}

# ---------------------------------------------------------------------------
# The service
# ---------------------------------------------------------------------------

ops_desired_count() {
  aws ecs describe-services --cluster "$OPS_CLUSTER" --services "$OPS_SERVICE" \
    --query 'services[0].desiredCount' --output text
}

# Stops the web task and waits until none runs. Terraform's state still says
# one: the next apply would start it again (deploy.sh starts it itself, after
# applying, since a saved plan made at one task does not set the count).
ops_stop_service() {
  ops_say "stopping the service (desired count 0); the -down alarms fire in ten minutes and clear once it is back"
  aws ecs update-service --cluster "$OPS_CLUSTER" --service "$OPS_SERVICE" --desired-count 0 \
    --query 'service.desiredCount' --output text >/dev/null
  local i running
  for i in $(seq 90); do
    running=$(aws ecs describe-services --cluster "$OPS_CLUSTER" --services "$OPS_SERVICE" \
      --query 'services[0].runningCount' --output text 2>/dev/null) || running=""
    [[ "$running" == 0 ]] && { ops_say "no task running"; return 0; }
    [[ $i == 1 ]] || ops_say "waiting for the task to stop ($running running)"
    sleep 10
  done
  ops_die "the service still runs a task after fifteen minutes: look at it before going on (the service stays at desired count 0)"
}

ops_start_service() {
  ops_say "starting the service (desired count 1)"
  aws ecs update-service --cluster "$OPS_CLUSTER" --service "$OPS_SERVICE" --desired-count 1 \
    --query 'service.desiredCount' --output text >/dev/null
}

# The backend image the service's settled deployment runs.
ops_running_backend_image() {
  local td
  td=$(aws ecs describe-services --cluster "$OPS_CLUSTER" --services "$OPS_SERVICE" \
    --query "services[0].deployments[?status=='PRIMARY'].taskDefinition | [0]" --output text) || return 1
  aws ecs describe-task-definition --task-definition "$td" \
    --query "taskDefinition.containerDefinitions[?name=='backend'].image | [0]" --output text
}

# Waits for the deployment to settle -- a single deployment, COMPLETED --
# then checks it runs EXPECTED (the backend image deployed), then waits for
# both target groups. A circuit-breaker rollback settles too, on the old
# image: said here, with what to do.
ops_wait_release() {
  local expected=$1 i out lines state running
  ops_say "waiting for the deployment to settle (startup is allowed ten minutes; three failed starts roll it back)"
  # ECS is eventually consistent: asked at once, it can still describe only
  # the old deployment, settled, which would read as a rollback.
  sleep 20
  for i in $(seq 240); do
    out=$(aws ecs describe-services --cluster "$OPS_CLUSTER" --services "$OPS_SERVICE" \
      --query 'services[0].deployments[].[status, rolloutState]' --output text 2>/dev/null) || out=""
    lines=$(printf '%s\n' "$out" | grep -c . || true)
    state=$(printf '%s\n' "$out" | awk 'NR == 1 { print $2 }')
    if [[ "$lines" == 1 && "$state" == COMPLETED ]]; then break; fi
    if [[ $i == 240 ]]; then
      ops_die "the deployment has not settled after an hour: see  aws ecs describe-services --cluster $OPS_CLUSTER --services $OPS_SERVICE"
    fi
    sleep 15
  done
  running=$(ops_running_backend_image) || running=""
  if [[ -n "$expected" && "$running" != "$expected" ]]; then
    aws ecs describe-services --cluster "$OPS_CLUSTER" --services "$OPS_SERVICE" \
      --query 'services[0].events[:8].[createdAt, message]' --output text >&2 || true
    ops_die "ECS rolled the release back: the service runs $running, not $expected. Terraform still names the new release, so before any other apply run scripts/rollback.sh (RUNBOOK.md, \"Rolling back a deploy\")"
  fi
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

# Asks for the hostname, stops the service and empties the schema. The
# service is left stopped: the caller starts it (the backend applies 0001 to
# the empty schema as it starts).
ops_reset_database() {
  local host
  host=$(ops_site_host)
  cat >&2 <<EOF

  RESETTING THE PRODUCTION DATABASE of $host
  Every account, admin flag, API key, job, result and the imported input
  data are deleted: the schema is dropped and the backend makes it again.
  The nightly dumps in the backups bucket are kept (of the old schema).

EOF
  ops_confirm_typed "$host" "reset its database"
  ops_stop_service
  ops_say "dropping and recreating the public schema (through the ops task)"
  INFRA_DIR=$OPS_INFRA "$OPS_ROOT/scripts/prod-sql.sh" "$OPS_RESET_SQL" \
    || ops_die "the reset failed (nothing was dropped if it says so above). The service is stopped: run this again, or start it with  aws ecs update-service --cluster $OPS_CLUSTER --service $OPS_SERVICE --desired-count 1"
  ops_log_release "reset-db host=$host"
}

# ---------------------------------------------------------------------------
# Shipping a change to prod.tfvars (deploy.sh, rollback.sh, set-setting.sh)
# ---------------------------------------------------------------------------

# With infra/prod.tfvars already edited: the plan gate; with OPS_RESET=1 the
# database reset, once the plan is approved and before the new task starts;
# the apply; the service started again after a reset (a plan saved while it
# ran one task does not set the count back); the upload; the release log
# line LOG_LINE; and the wait for the release to be healthy, running the
# backend image prod.tfvars now names.
ops_ship() {
  local log_line=$1 expected desired
  ops_plan_gate
  if [[ "${OPS_RESET:-0}" == 1 ]]; then
    ops_reset_database
  fi
  if [[ -z "$OPS_PLAN_EMPTY" ]]; then
    if ! ops_apply_plan; then
      if [[ "${OPS_RESET:-0}" == 1 ]]; then
        ops_say "the database has been reset and the service is stopped."
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
    ops_say "the service is at desired count 0: nothing to wait for"
  elif [[ "${OPS_RESET:-0}" == 1 ]] || { [[ -z "$OPS_PLAN_EMPTY" ]] && ops_plan_touches_service; }; then
    expected=$(ops_tfvar backend_image)
    ops_wait_release "$expected"
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
