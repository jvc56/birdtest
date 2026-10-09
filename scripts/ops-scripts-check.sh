#!/usr/bin/env bash
#
# The operator scripts (README.md, "Operator scripts") against stand-ins for
# aws, terraform, gh, docker and curl (scripts/lib/ops_fakes.py), in a
# throwaway git repository of their own: nothing here reaches AWS, GitHub or
# a Docker daemon.
#
#   ./scripts/ops-scripts-check.sh
#
# The cases:
#   - every script parses (bash -n);
#   - deploy.sh refuses a commit that changed 0001_initial.sql without
#     --reset-db, before building or planning anything; with it, the order is
#     build and push, plan, the service stopped, the schema dropped, the
#     dumps deleted, apply, the service started, prod.tfvars uploaded, and a
#     release log line; a wrong hostname typed resets nothing; an image ECR
#     already holds is not built again; red CI and a MAGPIE pin that is not
#     pushed are refused;
#   - a reset (reset-prod-db.sh, deploy.sh --reset-db) names both backup
#     buckets and the dump count before it asks, says RDS's backups stay for
#     db_backup_retention_days, and then deletes every version and delete
#     marker under pg/ in both -- each in its own region, more than one page of
#     each, at most 1000 to a delete, past the governance lock -- and nothing
#     outside pg/; a delete refused leaves the backend stopped and says so;
#   - the plan gate refuses a plan that replaces the database (nothing
#     applied), and lets it through only when asked, with a typed word;
#   - set-setting.sh's edit of prod.tfvars and its upload, its refusals (an
#     unknown variable, an image), and an upload refused when the bucket's copy
#     changed during the apply;
#   - rollback.sh takes the previous release from the release log, or the
#     image a circuit breaker went back to, and refuses one across a 0001
#     change without --reset-db;
#   - confirm-user.sh's SQL for hostile usernames: none of the name reaches
#     the SQL except as hex, which decodes back to it, and the SQL that
#     reaches the ops task is that SQL;
#   - prod-psql.sh's payload: the files in order, each variable's value
#     byte for byte, a missing variable refused, a write confirmed; and
#     --task's, in pieces over ECS Exec that make it whole again;
#   - pitr-restore.sh: a restore time not in UTC refused, `swap` in order
#     (rename, rename, state rm, import), resumed from half-way, and `count`
#     refused once the swap has begun;
#   - RUNBOOK.md §4's checks 2 and 3 are scripts/ops-sql/check-restore.sql's.
#
# With PG_EXEC set (e.g. "docker exec -i <postgres 16 container>", as
# scripts/reapply-check.sh takes it) the SQL also runs against a real
# Postgres with the schema: confirm-user.sh's for a hostile name, the reset,
# and each scripts/ops-sql file -- prod-psql.sh's run.sh unpacked and run as
# the ops task runs it, its one-off task's command as sent, and its --task
# ECS Exec commands replayed in the container, down to the detached run.
set -Eeuo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
fail() { echo "FAIL: $*" >&2; [[ -s "$WORK/out" ]] && sed 's/^/  | /' "$WORK/out" >&2; exit 1; }
pass() { echo "ok: $*"; }

for s in "$ROOT"/scripts/lib/ops.sh "$ROOT"/scripts/{deploy,reset-prod-db,rollback,set-setting,check-alerts,confirm-user,assess-damage,onboard-deployer,set-github-token,restore-artifact,rotate-db-password,prod-psql,pitr-restore}.sh; do
  bash -n "$s" || fail "$s does not parse"
done
pass "every script parses"

# ---------------------------------------------------------------------------
# The stand-ins, a repository, MAGPIE, and the settings.
# ---------------------------------------------------------------------------
REAL_PATH=$PATH REAL_HOME=$HOME
export FAKE_DIR="$WORK/fake"
mkdir -p "$WORK/bin" "$FAKE_DIR" "$WORK/home"
for t in aws terraform gh docker curl session-manager-plugin; do
  ln -s "$ROOT/scripts/lib/ops_fakes.py" "$WORK/bin/$t"
done
printf '#!/bin/sh\nexit 0\n' > "$WORK/bin/sleep"
chmod +x "$WORK/bin/sleep"
export PATH="$WORK/bin:$PATH"
export HOME="$WORK/home"
export BIRDTEST_ENV="$WORK/home/.birdtest-env"
export BIRDTEST_RELEASE_LOG="$WORK/home/releases.log"
export OPS_TTY="$WORK/answers"
export MAGPIE_DIR="$WORK/magpie"
export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@example.org GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@example.org
REGISTRY=123456789012.dkr.ecr.us-east-1.amazonaws.com
cat > "$BIRDTEST_ENV" <<EOF
export AWS_PROFILE=birdtest
export SITE=birdtest.test STATE_BUCKET=state-bucket STATE_REGION=us-east-2
export ACCOUNT=123456789012 REGISTRY=$REGISTRY
EOF

git -c init.defaultBranch=main init -q --bare "$WORK/magpie-origin.git"
git -c init.defaultBranch=main init -q "$WORK/magpie-src"
git -C "$WORK/magpie-src" commit -q --allow-empty -m pinned
pin=$(git -C "$WORK/magpie-src" rev-parse HEAD)
git -C "$WORK/magpie-src" push -q "$WORK/magpie-origin.git" HEAD:refs/heads/birdtest-contribute
git -C "$WORK/magpie-src" commit -q --allow-empty -m unpushed
unpushed=$(git -C "$WORK/magpie-src" rev-parse HEAD)
git clone -q -b birdtest-contribute "$WORK/magpie-origin.git" "$WORK/magpie"
git -C "$WORK/magpie" fetch -q "$WORK/magpie-src" "$unpushed"

R="$WORK/repo"
git -c init.defaultBranch=main init -q --bare "$WORK/origin.git"
git -c init.defaultBranch=main init -q "$R"
mkdir -p "$R/scripts/lib" "$R/infra" "$R/docker" "$R/backend/migrations"
cp "$ROOT"/scripts/lib/ops.sh "$R/scripts/lib/"
cp "$ROOT"/scripts/*.sh "$R/scripts/"
cp -r "$ROOT/scripts/ops-sql" "$R/scripts/"
cp "$ROOT/infra/variables.tf" "$R/infra/"
printf 'ARG MAGPIE_COMMIT=%s\n' "$pin" > "$R/docker/Dockerfile"
printf 'CREATE TABLE users (id int);\n' > "$R/backend/migrations/0001_initial.sql"
printf '*.tfvars\n' > "$R/.gitignore"
git -C "$R" add -A && git -C "$R" commit -q -m A
A=$(git -C "$R" rev-parse --short=12 HEAD)
echo change >> "$R/scripts/README-test" && git -C "$R" add -A && git -C "$R" commit -q -m B
B=$(git -C "$R" rev-parse --short=12 HEAD)
printf 'CREATE TABLE users (id int, games bigint);\n' > "$R/backend/migrations/0001_initial.sql"
git -C "$R" commit -q -am "C changes 0001"
C=$(git -C "$R" rev-parse --short=12 HEAD)
git -C "$R" remote add origin "$WORK/origin.git"
git -C "$R" push -q origin HEAD:refs/heads/main

PLAN_NORMAL='{"resource_changes": [
  {"address": "aws_ecs_task_definition.main", "type": "aws_ecs_task_definition", "change": {"actions": ["delete", "create"]}},
  {"address": "aws_ecs_service.main", "type": "aws_ecs_service", "change": {"actions": ["update"]}},
  {"address": "aws_db_instance.main", "type": "aws_db_instance", "change": {"actions": ["no-op"]}}]}'
PLAN_DB_REPLACE='{"resource_changes": [
  {"address": "aws_db_instance.main", "type": "aws_db_instance", "change": {"actions": ["delete", "create"]}}]}'

tfvars_with() {  # tag -> a prod.tfvars naming it
  cat <<EOF
region                = "us-east-1"
backend_image         = "$REGISTRY/birdtest-backend:$1"
derived_builder_image = "$REGISTRY/birdtest-derived-builder:$1"
frontend_image        = "$REGISTRY/birdtest-frontend:$1"
public_url            = "https://birdtest.test"
min_magpie_version    = "0.1.1"
EOF
}
S3_TFVARS="$FAKE_DIR/s3/state-bucket/birdtest/prod.tfvars"
DUMPS="$FAKE_DIR/s3v/birdtest-backups-123.json" DUMPS_DR="$FAKE_DIR/s3v/birdtest-backups-dr-123.json"
# The backups bucket: three dumps of 800 files each, every file in two
# versions, a manifest overwritten once and three delete markers -- 4,807
# versions and markers under pg/, five pages -- and one object outside pg/,
# which a reset leaves. Its replica: two dumps, 1,202, two pages.
dumps_world() {
  mkdir -p "$FAKE_DIR/s3v"
  python3 - "$DUMPS" "$DUMPS_DR" <<'EOF'
import json, sys
def world(stamps, files, versions):
    e = []
    for s in stamps:
        for f in range(files):
            for v in range(versions):
                e.append({"Key": "pg/%s/toc.%d.dat" % (s, f), "VersionId": "%s-%d-%d" % (s, f, v)})
        e.append({"Key": "pg/%s.manifest.json" % s, "VersionId": s + "-m"})
    return e
primary = world(["2026-10-01", "2026-10-02", "2026-10-03"], 800, 2)
primary += [{"Key": "pg/2026-10-03.manifest.json", "VersionId": "again"}]
primary += [{"Key": "pg/old/%d" % i, "VersionId": "dm%d" % i, "DeleteMarker": True} for i in range(3)]
primary += [{"Key": "elsewhere/keep.txt", "VersionId": "keep"}]
json.dump(primary, open(sys.argv[1], "w"))
json.dump(world(["2026-10-01", "2026-10-02"], 600, 1), open(sys.argv[2], "w"))
EOF
}
# How many versions and markers BUCKET-FILE holds under pg/.
pg_left() { jq '[.[] | select(.Key | startswith("pg/"))] | length' "$1"; }
# A fresh world: the bucket naming LIVE, ECR holding the tags given, the
# service at one task running LIVE, no local prod.tfvars, no calls yet.
fresh() {
  local live=$1; shift
  rm -rf "$FAKE_DIR"/{calls.log,ecr,runtask,rds,ssm,tfstate_db,s3v} "$R"/infra/prod.tfvars* "$WORK/answers"
  mkdir -p "$FAKE_DIR/ecr" "$(dirname "$S3_TFVARS")"
  dumps_world
  tfvars_with "$live" > "$S3_TFVARS"
  local t r
  for t in "$@"; do for r in backend derived-builder frontend; do : > "$FAKE_DIR/ecr/birdtest-$r:$t"; done; done
  echo 1 > "$FAKE_DIR/desired"
  printf '%s' "$REGISTRY/birdtest-backend:$live" > "$FAKE_DIR/running_image"
  printf '%s' "$PLAN_NORMAL" > "$FAKE_DIR/plan.json"
  : > "$FAKE_DIR/calls.log"
  : > "$WORK/answers"
}
answers() { printf '%s\n' "$@" > "$WORK/answers"; }
# Runs a script of the test repository; its output in $WORK/out.
run() { local rc=0; "$R/scripts/$1" "${@:2}" > "$WORK/out" 2>&1 < /dev/null || rc=$?; return "$rc"; }
called() { grep -qF -- "$1" "$FAKE_DIR/calls.log"; }
# The calls matching each pattern happened in this order (the first of each).
in_order() {
  local last=0 n p
  for p in "$@"; do
    n=$(grep -nF -m1 -- "$p" "$FAKE_DIR/calls.log" | cut -d: -f1)
    [[ -n "$n" ]] || fail "no call matching: $p"
    ((n > last)) || fail "out of order: $p (call $n, after call $last)"
    last=$n
  done
}

# ---------------------------------------------------------------------------
# deploy.sh
# ---------------------------------------------------------------------------
git -C "$R" checkout -q "$C"

fresh "$A"
if run deploy.sh; then fail "deploy.sh deployed a changed 0001 without --reset-db"; fi
grep -q -- "--reset-db" "$WORK/out" || fail "the refusal does not say --reset-db"
grep -q "0001_initial.sql" "$WORK/out" || fail "the refusal does not name 0001_initial.sql"
{ ! called "docker build" && ! called "terraform -chdir=$R/infra plan"; } || fail "deploy.sh built or planned before refusing"
pass "deploy.sh refuses a changed 0001 without --reset-db, before building"

fresh "$A"
answers y nottherightname
if run deploy.sh --reset-db; then fail "deploy.sh reset with the wrong hostname typed"; fi
{ ! called "--desired-count 0" && ! called "ecs run-task" && ! called "delete-objects" && ! called " apply "; } \
  || fail "a wrong hostname still stopped, reset, deleted or applied something"
[[ "$(pg_left "$DUMPS")" == 4807 && "$(pg_left "$DUMPS_DR")" == 1202 ]] || fail "a wrong hostname still deleted dumps"
pass "deploy.sh --reset-db resets nothing when the hostname is mistyped"

fresh "$A"
answers y birdtest.test
run deploy.sh --reset-db || fail "deploy.sh --reset-db failed"
in_order "docker push $REGISTRY/birdtest-backend:$C" \
         "docker push $REGISTRY/birdtest-frontend:$C" \
         "terraform -chdir=$R/infra plan" \
         "aws ecs update-service --cluster birdtest --service birdtest --desired-count 0" \
         "aws ecs run-task" \
         "aws s3api delete-objects --bucket birdtest-backups-123 " \
         "aws s3api delete-objects --bucket birdtest-backups-dr-123 " \
         "terraform -chdir=$R/infra apply" \
         "aws ecs update-service --cluster birdtest --service birdtest --desired-count 1" \
         "aws s3 cp $R/infra/prod.tfvars s3://state-bucket/birdtest/prod.tfvars" \
         "aws elbv2 wait target-in-service"
grep -q "DROP SCHEMA public CASCADE" "$FAKE_DIR/runtask/1.json" || fail "the reset's task did not drop the schema"
grep -q "birdtest-backend:$C\"" "$S3_TFVARS" || fail "the uploaded prod.tfvars does not name $C"
grep -q "deployed tag=$C previous=$A " "$BIRDTEST_RELEASE_LOG" || fail "no release log line"
grep -q "reset-db host=birdtest.test" "$BIRDTEST_RELEASE_LOG" || fail "the reset is not in the release log"
grep -q "confirm-user.sh --admin" "$WORK/out" || fail "no post-reset checklist"
{ grep -q "CARGO_BUILD_JOBS=2" "$FAKE_DIR/calls.log" && grep -q -- "--platform linux/amd64" "$FAKE_DIR/calls.log"; } \
  || fail "the build is not capped, or not for linux/amd64"
[[ "$(pg_left "$DUMPS")" == 0 && "$(pg_left "$DUMPS_DR")" == 0 ]] || fail "deploy.sh --reset-db left dumps"
pass "deploy.sh --reset-db: build, push, plan, stop, reset, dumps deleted, apply, start, upload, log, health"

# The reset on its own, and what it says before it asks.
fresh "$B"
answers birdtest.test
run reset-prod-db.sh || fail "reset-prod-db.sh failed"
for want in "birdtest-backups-123 (us-east-1): 3 dumps, 4804 object versions, 3 delete markers" \
            "birdtest-backups-dr-123 (eu-west-1): 2 dumps, 1202 object versions, 0 delete markers" \
            "delete the dumps in birdtest-backups-123 and birdtest-backups-dr-123 (5 in all)" \
            "until they expire, 30 days from now"; do
  grep -qF -- "$want" "$WORK/out" || fail "the reset's warning does not say: $want"
done
[[ "$(pg_left "$DUMPS")" == 0 && "$(pg_left "$DUMPS_DR")" == 0 ]] || fail "reset-prod-db.sh left dumps"
jq -e 'map(select(.Key == "elsewhere/keep.txt")) | length == 1' "$DUMPS" > /dev/null \
  || fail "the reset deleted an object outside pg/"
[[ "$(grep -c "s3api delete-objects --bucket birdtest-backups-123 --region us-east-1 " "$FAKE_DIR/calls.log")" == 5 ]] \
  || fail "the backups bucket was not deleted from in five pages"
[[ "$(grep -c "s3api delete-objects --bucket birdtest-backups-dr-123 --region eu-west-1 " "$FAKE_DIR/calls.log")" == 2 ]] \
  || fail "the DR bucket was not deleted from in two pages, in its own region"
! grep "delete-objects" "$FAKE_DIR/calls.log" | grep -qv -- "--bypass-governance-retention" \
  || fail "a delete did not bypass the governance lock"
in_order "aws ecs update-service --cluster birdtest --service birdtest --desired-count 0" \
         "aws ecs run-task" \
         "aws s3api delete-objects --bucket birdtest-backups-123 " \
         "aws s3api delete-objects --bucket birdtest-backups-dr-123 " \
         "aws ecs update-service --cluster birdtest --service birdtest --desired-count 1"
grep -q "reset-db host=birdtest.test dumps_deleted=5" "$BIRDTEST_RELEASE_LOG" || fail "the reset's log line does not count the dumps"
pass "reset-prod-db.sh names both buckets and the dump count, then deletes every version under pg/ in each"

fresh "$B"
answers birdtest.test
if FAKE_DUMP_DELETE_FAIL=1 run reset-prod-db.sh; then fail "a reset whose dumps were not deleted succeeded"; fi
grep -q "dumps remain in birdtest-backups-123" "$WORK/out" || fail "the refused delete is not reported"
grep -q "AccessDenied" "$WORK/out" || fail "S3's reason is not shown"
! called "--desired-count 1" || fail "the backend was started with dumps left"
pass "a delete S3 refuses stops the reset with the backend stopped, and says so"

git -C "$R" checkout -q "$B"
fresh "$A" "$B"
answers y
run deploy.sh || fail "deploy.sh failed for an unchanged schema"
! called "docker build" || fail "deploy.sh built images ECR already holds"
{ ! called "--desired-count 0" && ! called "ecs run-task"; } || fail "deploy.sh reset without being asked"
in_order "terraform -chdir=$R/infra plan" "terraform -chdir=$R/infra apply" "aws s3 cp $R/infra/prod.tfvars"
pass "deploy.sh: no reset when 0001 is unchanged, no build when ECR has the images"

fresh "$A" "$B"
answers y
if FAKE_CI="completed failure https://ci/2" run deploy.sh; then fail "deploy.sh deployed with CI red"; fi
grep -q "failure" "$WORK/out" || fail "the CI refusal does not say why"
git -C "$R" checkout -q "$C"
printf 'ARG MAGPIE_COMMIT=%s\n' "$unpushed" > "$R/docker/Dockerfile"
git -C "$R" commit -q -am "an unpushed pin" && git -C "$R" push -q origin HEAD:refs/heads/main
fresh "$A" "$B"
if run deploy.sh; then fail "deploy.sh deployed an unpushed MAGPIE pin"; fi
grep -q "birdtest-contribute" "$WORK/out" || fail "the pin refusal does not say why"
git -C "$R" checkout -q "$B"
pass "deploy.sh refuses red CI and a MAGPIE pin that is not pushed"

# ---------------------------------------------------------------------------
# The plan gate and set-setting.sh
# ---------------------------------------------------------------------------
fresh "$B"
printf '%s' "$PLAN_DB_REPLACE" > "$FAKE_DIR/plan.json"
answers y y
if run set-setting.sh mail_max_per_second=14; then fail "a plan replacing the database was applied"; fi
{ grep -q "refused" "$WORK/out" && grep -q "aws_db_instance.main: delete then create" "$WORK/out"; } \
  || fail "the gate does not say what it refused"
! called " apply " || fail "the gate let an apply through"
cmp -s <(tfvars_with "$B") "$S3_TFVARS" || fail "a refused plan still uploaded prod.tfvars"
fresh "$B"
printf '%s' "$PLAN_DB_REPLACE" > "$FAKE_DIR/plan.json"
answers destroy y
BIRDTEST_ALLOW_DESTRUCTIVE_PLAN=yes run set-setting.sh mail_max_per_second=14 \
  || fail "the gate refused a destructive plan it was told to let through"
called " apply " || fail "the allowed plan was not applied"
pass "the plan gate refuses a database replacement, and passes it only when told and typed"

fresh "$B"
answers y
run set-setting.sh mail_max_per_second=14 public_url=https://birdtest.test || fail "set-setting.sh failed"
grep -qx 'mail_max_per_second = 14' "$S3_TFVARS" || fail "the setting was not uploaded"
grep -qx 'public_url = "https://birdtest.test"' "$S3_TFVARS" || fail "a string was not quoted"
grep -q "setting mail_max_per_second=14" "$BIRDTEST_RELEASE_LOG" || fail "the setting is not in the release log"
in_order "terraform -chdir=$R/infra plan" "terraform -chdir=$R/infra apply" "aws s3 cp $R/infra/prod.tfvars"
fresh "$B"
if run set-setting.sh mail_max_per_secnd=14; then fail "an unknown variable was set"; fi
if run set-setting.sh backend_image=x; then fail "an image was set by set-setting.sh"; fi
if run set-setting.sh 'ses_domain=a"b'; then fail "an unquoted value with a quote was taken"; fi
! called "terraform -chdir=$R/infra plan" || fail "a refused setting was planned"
fresh "$B"
answers y
if FAKE_APPLY_TOUCH_S3=1 run set-setting.sh mail_max_per_second=14; then
  fail "prod.tfvars was uploaded over a copy changed during the apply"
fi
grep -q "changed while this ran" "$WORK/out" || fail "the concurrent change is not reported"
grep -q "someone else" "$S3_TFVARS" || fail "the other deployer's copy was overwritten"
pass "set-setting.sh edits, uploads and logs; refuses unknown and image variables, and a concurrent change"

# ---------------------------------------------------------------------------
# rollback.sh
# ---------------------------------------------------------------------------
fresh "$B" "$A" "$B"
printf '%s\n' "2026-10-01T00:00:00Z deployed tag=$A previous=0123456789ab backend_image=x min_magpie_version=0.1.1" \
  "2026-10-02T00:00:00Z deployed tag=$B previous=$A backend_image=y min_magpie_version=0.1.1" > "$BIRDTEST_RELEASE_LOG"
answers "" y y
run rollback.sh || fail "rollback.sh failed"
grep -q "Roll back to which tag? \[$A\]" "$WORK/out" || fail "the default was not the previous release ($A)"
grep -q "birdtest-backend:$A\"" "$S3_TFVARS" || fail "the rollback did not upload $A"
grep -q "rolled-back tag=$A previous=$B" "$BIRDTEST_RELEASE_LOG" || fail "no release log line for the rollback"
pass "rollback.sh takes the previous release from the release log"

fresh "$B" "$A" "$B"
printf '%s' "$REGISTRY/birdtest-backend:$A" > "$FAKE_DIR/running_image"
printf '%s\n' "2026-10-02T00:00:00Z deployed tag=$B previous=0123456789ab" > "$BIRDTEST_RELEASE_LOG"
answers "" y y
run rollback.sh || fail "rollback.sh failed after a circuit breaker"
{ grep -q "circuit breaker" "$WORK/out" && grep -q "Roll back to which tag? \[$A\]" "$WORK/out"; } \
  || fail "the circuit breaker's rollback was not taken as the target"
pass "rollback.sh follows a circuit-breaker rollback"

fresh "$C" "$A" "$C"
printf '%s\n' "2026-10-02T00:00:00Z deployed tag=$C previous=$A" > "$BIRDTEST_RELEASE_LOG"
answers "" y y
if run rollback.sh; then fail "rollback.sh crossed a 0001 change without --reset-db"; fi
grep -q -- "--reset-db" "$WORK/out" || fail "the refusal does not say --reset-db"
! called " apply " || fail "a refused rollback applied"
pass "rollback.sh refuses to cross a 0001 change without --reset-db"

# ---------------------------------------------------------------------------
# confirm-user.sh
# ---------------------------------------------------------------------------
# shellcheck disable=SC2016 # literal dollars, on purpose
hostile=("x'); DROP TABLE users; --" 'a$$b$confirm$c' 'back\slash' "q'uo\"te" ":'job' :job \\gset" 'Zoë 名前')
for name in "${hostile[@]}"; do
  sql=$("$R/scripts/confirm-user.sh" --admin --print-sql "$name")
  hex=$(printf '%s' "$sql" | sed -n "s/.*decode('\([0-9a-f]*\)', 'hex').*/\1/p" | head -1)
  [[ "$(python3 -c 'import sys; print(bytes.fromhex(sys.argv[1]).decode())' "$hex")" == "$name" ]] \
    || fail "the hex in the SQL does not decode to: $name"
  stripped=$(printf '%s' "$sql" | sed "s/decode('[0-9a-f]*', 'hex')//g")
  # shellcheck disable=SC2016,SC1003 # literal pieces of the names
  for frag in "DROP TABLE" 'a$$b' "q'uo" 'back\' ":'job'" "Zoë"; do
    [[ "$name" != *"$frag"* || "$stripped" != *"$frag"* ]] || fail "a piece of the username reached the SQL: $frag"
  done
done
fresh "$B"
run confirm-user.sh --admin "${hostile[0]}" || fail "confirm-user.sh failed"
sent=$(jq -r '.containerOverrides[0].environment[0].value' "$FAKE_DIR/runtask/1.json")
[[ "$sent" == "$("$R/scripts/confirm-user.sh" --admin --print-sql "${hostile[0]}")" ]] \
  || fail "the SQL sent to the ops task is not the SQL printed"
grep -q "make_admin boolean := true" <<<"$sent" || fail "--admin did not make it into the SQL"
pass "confirm-user.sh: hostile usernames reach the SQL only as hex"

# ---------------------------------------------------------------------------
# prod-psql.sh
# ---------------------------------------------------------------------------
fresh "$B"
if run prod-psql.sh "$R/scripts/ops-sql/clear-job.sql"; then fail "clear-job.sql ran without -v job"; fi
grep -q "needs -v job" "$WORK/out" || fail "the missing variable is not named"
answers n
if run prod-psql.sh -v job=11111111-2222-3333-4444-555555555555 "$R/scripts/ops-sql/clear-job.sql"; then
  fail "a writing file ran unconfirmed"
fi
! called "ecs run-task" || fail "an unconfirmed write started a task"
fresh "$B"
answers y
odd=$'it\'s "odd" $(x) `y` \\ :z\n'
run prod-psql.sh -v job=11111111-2222-3333-4444-555555555555 -v "note=$odd" \
  "$R/scripts/ops-sql/clear-job.sql" "$R/scripts/ops-sql/check-restore.sql" || fail "prod-psql.sh failed"
grep -q "psql printed this" "$WORK/out" || fail "the run's output was not followed"
mkdir "$WORK/payload"
jq -r '.containerOverrides[0].environment[0].value' "$FAKE_DIR/runtask/1.json" | base64 -d | tar -xzf - -C "$WORK/payload"
[[ "$(cat "$WORK/payload/vars/note"; printf x)" == "${odd}x" ]] || fail "a variable's value changed on the way"
[[ "$(ls "$WORK/payload/sql")" == $'01-clear-job.sql\n02-check-restore.sql' ]] || fail "the files are not in order"
cmp -s "$WORK/payload/sql/01-clear-job.sql" "$R/scripts/ops-sql/clear-job.sql" || fail "a file changed on the way"
grep -q -- "--single-transaction" "$WORK/payload/run.sh" && fail "run.sh forces one transaction"
grep -q "ON_ERROR_STOP=1" "$WORK/payload/run.sh" || fail "run.sh does not stop on errors"
pass "prod-psql.sh: the files and variables reach the task intact, writes confirmed, needs enforced"

# --task: the payload over ECS Exec in pieces, put back together whole. The
# stand-in's log never shows the run, so it gives up and says where to look.
TASK_ARN=arn:aws:ecs:us-east-1:123456789012:task/birdtest/shelltask
{ echo "CREATE TABLE ops_task_ran (n int);"; head -c 9000 /dev/urandom | base64 | sed 's/^/-- /'; } > "$WORK/big.sql"
fresh "$B"
if run prod-psql.sh --task "$TASK_ARN" -v job=x "$WORK/big.sql"; then fail "a run never seen in the log counted as done"; fi
grep -q "cat /tmp/ops/run-" "$WORK/out" || fail "the give-up does not say where the output is"
grep "aws ecs execute-command" "$FAKE_DIR/calls.log" | sed -n 's/.* --command //p' > "$WORK/exec-commands"
chunks=$(grep -c "printf %s [A-Za-z0-9+/=]* >> /tmp/ops/run-" "$WORK/exec-commands" || true)
((chunks >= 3)) || fail "a large payload went in $chunks pieces"
sed -n "s#^/bin/bash -c 'printf %s \([A-Za-z0-9+/=]*\) >> /tmp/ops/run-.*#\1#p" "$WORK/exec-commands" | tr -d '\n' \
  | base64 -d | tar -tzf - > "$WORK/listing" || fail "the pieces do not make the payload"
{ grep -q "run.sh" "$WORK/listing" && grep -q "big.sql" "$WORK/listing"; } || fail "the payload lacks its files"
pass "prod-psql.sh --task: the payload in pieces over ECS Exec, whole again, and a run never seen is not waited on forever"

# ---------------------------------------------------------------------------
# pitr-restore.sh
# ---------------------------------------------------------------------------
fresh "$B"
mkdir -p "$FAKE_DIR/rds"
echo db-DAMAGED > "$FAKE_DIR/rds/birdtest"
rm -rf "$HOME/.birdtest-pitr"
if run pitr-restore.sh restore --time "2026-10-06 11:00"; then fail "a restore time not in ISO UTC was taken"; fi
! called "restore-db-instance-to-point-in-time" || fail "a bad time still restored"
answers y
run pitr-restore.sh restore --time 2026-10-06T11:00:00Z || fail "pitr restore failed"
stamp=$(sed -n 's/^STAMP=//p' "$HOME/.birdtest-pitr/state")
[[ -e "$FAKE_DIR/rds/birdtest-restore-$stamp" ]] || fail "no restore instance"
{ grep -q -- "--db-parameter-group-name birdtest-pg-1" "$FAKE_DIR/calls.log" \
  && grep -q -- "--max-allocated-storage 100" "$FAKE_DIR/calls.log"; } || fail "the restore lost its parameter group or ceiling"
if run pitr-restore.sh restore --time 2026-10-06T11:05:00Z; then fail "a second restore was started"; fi
run pitr-restore.sh count || fail "pitr count failed"
echo db-DAMAGED > "$FAKE_DIR/tfstate_db"
: > "$FAKE_DIR/calls.log"
answers swap
run pitr-restore.sh swap || fail "pitr swap failed"
in_order "--db-instance-identifier birdtest --new-db-instance-identifier birdtest-damaged-$stamp" \
         "--db-instance-identifier birdtest-restore-$stamp --new-db-instance-identifier birdtest" \
         "terraform -chdir=$R/infra state rm aws_db_instance.main" \
         "terraform -chdir=$R/infra import -input=false -var-file=prod.tfvars aws_db_instance.main birdtest"
[[ "$(cat "$FAKE_DIR/tfstate_db")" == db-RESTORED ]] || fail "the state does not hold the restored instance"
[[ "$(cat "$FAKE_DIR/rds/birdtest-damaged-$stamp")" == db-DAMAGED ]] || fail "the damaged instance was not renamed"
if run pitr-restore.sh count; then fail "count ran after the swap"; fi
# Half-way: the first rename done, the second not.
mv "$FAKE_DIR/rds/birdtest" "$FAKE_DIR/rds/birdtest-restore-$stamp"
echo db-DAMAGED > "$FAKE_DIR/tfstate_db"
: > "$FAKE_DIR/calls.log"
answers swap
run pitr-restore.sh swap || fail "pitr swap did not resume"
! called "--new-db-instance-identifier birdtest-damaged" || fail "the resumed swap renamed the damaged instance again"
in_order "--new-db-instance-identifier birdtest " "state rm" "import"
[[ "$(cat "$FAKE_DIR/tfstate_db")" == db-RESTORED ]] || fail "the resumed swap did not import the restored instance"
pass "pitr-restore.sh: time checked, one restore, swap in order and resumable, count refused after it"

# ---------------------------------------------------------------------------
# RUNBOOK §4 and check-restore.sql say the same.
# ---------------------------------------------------------------------------
from_runbook=$(awk '/^-- 2\. Referential sanity\./ { on = 1 } /^-- 3b\./ { on = 0 } on' "$ROOT/RUNBOOK.md")
from_file=$(awk '/^-- 2\. Referential sanity\./ { on = 1 } /^COMMIT;/ { on = 0 } on' "$ROOT/scripts/ops-sql/check-restore.sql")
[[ -n "$from_runbook" && "$from_runbook" == "$from_file" ]] \
  || fail "RUNBOOK.md §4's checks 2 and 3 and scripts/ops-sql/check-restore.sql differ"
pass "RUNBOOK.md §4's checks 2 and 3 are check-restore.sql's"

# ---------------------------------------------------------------------------
# Against a real Postgres, when one is given.
# ---------------------------------------------------------------------------
if [[ -n "${PG_EXEC:-}" ]]; then
  PGU="${PGUSER_:-postgres}"
  # The real tools, not the stand-ins (a fake `docker` is on PATH).
  # shellcheck disable=SC2086
  pgx() { PATH="$REAL_PATH" HOME="$REAL_HOME" ${PG_EXEC} "$@"; }
  pg() { pgx psql -U "$PGU" -X -q -v ON_ERROR_STOP=1 "$@"; }
  pg -d postgres -c "DROP DATABASE IF EXISTS ops_check" -c "CREATE DATABASE ops_check" >/dev/null
  pg -d ops_check < "$ROOT/backend/migrations/0001_initial.sql" > /dev/null
  name="x'); DROP TABLE users; --"
  pg -d ops_check -c "INSERT INTO users (username, email, password_hash) VALUES ('X''); DROP TABLE users; --', 'a@example.org', 'h')" >/dev/null
  pg -d ops_check -c "INSERT INTO email_confirmations (user_id, code_hash, expires_at) SELECT id, 'c', now() + interval '1 day' FROM users" >/dev/null
  "$R/scripts/confirm-user.sh" --admin --print-sql "$name" | pg -d ops_check -f - > "$WORK/out" 2>&1 || fail "confirm-user.sh's SQL failed"
  got=$(pgx psql -U "$PGU" -X -tA -d ops_check -c "
    SELECT (SELECT email_confirmed_at IS NOT NULL AND is_admin FROM users),
           (SELECT count(*) FROM email_confirmations WHERE used_at IS NULL),
           (SELECT action || ' ' || target_type || ' ' || (target_id = actor_user_id::text) FROM audit_log)")
  [[ "$got" == $'t|0|user.email_confirmed user true' ]] || fail "confirm-user.sh's SQL left: $got"
  "$R/scripts/confirm-user.sh" --print-sql "$name" | pg -d ops_check -f - > "$WORK/out" 2>&1 || fail "a second confirm failed"
  [[ "$(pgx psql -U "$PGU" -X -tA -d ops_check -c "SELECT count(*) FROM audit_log")" == 1 ]] \
    || fail "a second confirm wrote a second audit row"
  if "$R/scripts/confirm-user.sh" --print-sql "nobody" | pg -d ops_check -f - > "$WORK/out" 2>&1; then
    fail "confirm-user.sh confirmed an account that does not exist"
  fi
  pass "confirm-user.sh's SQL against the schema: confirmed, codes spent, one audit row, a missing name refused"

  # The scripts/ops-sql files, through prod-psql.sh's own run.sh, as the ops
  # task runs it: unpacked from the payload, DATABASE_URL in the environment.
  job=$(pgx psql -U "$PGU" -X -tA -d ops_check -c "SELECT gen_random_uuid()")
  pgx bash -c "rm -rf /tmp/ops-check && mkdir -p /tmp/ops-check"
  printf '%s' "$job" > "$WORK/payload/vars/job"
  rm -f "$WORK/payload/vars/note" "$WORK/payload/sql/"*
  cp "$ROOT/scripts/ops-sql/clear-job.sql" "$WORK/payload/sql/01-clear-job.sql"
  cp "$ROOT/scripts/ops-sql/repair-job-counters.sql" "$WORK/payload/sql/02-repair.sql"
  cp "$ROOT/scripts/ops-sql/check-restore.sql" "$WORK/payload/sql/03-check.sql"
  tar -czf - -C "$WORK/payload" . | pgx bash -c "cd /tmp/ops-check && tar -xzf - && DATABASE_URL='postgresql:///ops_check?user=$PGU' bash run.sh" \
    > "$WORK/out" 2>&1 || fail "the ops-sql files failed in run.sh"
  grep -q "exit 0" "$WORK/out" || fail "run.sh did not report its end"
  { grep -q "jobs_missing_data" "$WORK/out" && grep -q "counter_disagreements" "$WORK/out"; } || fail "check-restore.sql printed nothing"
  # §2.6's copy, with the scratch copy named as §2.1 names it, as far as its
  # refusal of a pool the copy does not have: nothing loaded.
  rm -f "$WORK/payload/sql/"* "$WORK/payload/vars/"*
  cp "$ROOT/scripts/ops-sql/restore-rating-pool.sql" "$WORK/payload/sql/01-pool.sql"
  printf '%s' "$job" > "$WORK/payload/vars/pool"
  if tar -czf - -C "$WORK/payload" . | pgx bash -c "rm -rf /tmp/ops-check && mkdir -p /tmp/ops-check && cd /tmp/ops-check && tar -xzf - \
       && echo \"export SCRATCH_URL='postgresql:///ops_check?user=$PGU'\" > /tmp/restore.env \
       && DATABASE_URL='postgresql:///ops_check?user=$PGU' bash run.sh; rc=\$?; rm -f /tmp/restore.env; exit \$rc" \
       > "$WORK/out" 2>&1; then
    fail "restore-rating-pool.sql copied a pool the scratch copy does not have"
  fi
  grep -q "has no such pool" "$WORK/out" || fail "restore-rating-pool.sql did not refuse for the right reason"
  # The whole one-shot task as prod-psql.sh starts it: its command and its
  # environment, in the postgres image, as the ops task's `bash -c`.
  fresh "$B"
  run prod-psql.sh "$R/scripts/ops-sql/check-restore.sql" || fail "prod-psql.sh failed"
  command=$(jq -r '.containerOverrides[0].command[0]' "$FAKE_DIR/runtask/1.json")
  payload=$(jq -r '.containerOverrides[0].environment[0].value' "$FAKE_DIR/runtask/1.json")
  pgx env BIRDTEST_PAYLOAD="$payload" DATABASE_URL="postgresql:///ops_check?user=$PGU" bash -c "$command" \
    > "$WORK/out" 2>&1 || fail "the task's command failed"
  { grep -q "__birdtest_run run-.* exit 0" "$WORK/out" && grep -q "counter_disagreements" "$WORK/out"; } \
    || fail "the task's command did not run the file"
  # And --task's ECS Exec commands, as recorded above, replayed in the
  # container: the pieces, the checksum, the unpacking and the detached run.
  # (Its copy to /proc/1/fd/1 is denied here, where PID 1 is the server's
  # user rather than root as in the ops task; the run does not depend on it.)
  pgx bash -c "rm -rf /tmp/ops" > /dev/null
  while IFS= read -r cmd; do
    pgx env DATABASE_URL="postgresql:///ops_check?user=$PGU" bash -c "$cmd" < /dev/null > /dev/null 2>&1 \
      || fail "an ECS Exec command failed in the container: ${cmd:0:80}"
  done < "$WORK/exec-commands"
  ran=""
  for _ in $(seq 30); do
    ran=$(pgx psql -U "$PGU" -X -tA -d ops_check -c "SELECT to_regclass('ops_task_ran') IS NOT NULL")
    [[ "$ran" == t ]] && break
    PATH="$REAL_PATH" sleep 1
  done
  [[ "$ran" == t ]] || fail "the detached run never ran the file"
  pgx bash -c "cat /tmp/ops/run-*.log" | grep -q "__birdtest_run run-.* exit 0" \
    || fail "the detached run's log does not end with its exit"
  pg -d ops_check -c "$(sed -n '/^OPS_RESET_SQL="/,/^"$/p' "$ROOT/scripts/lib/ops.sh" | sed '1d;$d')" > "$WORK/out" 2>&1 \
    || fail "the reset SQL failed"
  [[ "$(pgx psql -U "$PGU" -X -tA -d ops_check -c "SELECT count(*) FROM pg_tables WHERE schemaname = 'public'")" == 0 ]] \
    || fail "the reset left tables"
  pg -d ops_check < "$ROOT/backend/migrations/0001_initial.sql" > /dev/null || fail "0001 does not apply after the reset"
  pg -d postgres -c "DROP DATABASE ops_check" >/dev/null
  pass "the ops-sql files run through run.sh, the one-off and --task paths in the postgres image, and the reset empties a schema 0001 applies to again"
fi

echo "ops scripts check passed"
