#!/usr/bin/env bash
# Deploy the checked-out commit of main to production (LAUNCH_PLAN Part 3).
#
#   scripts/deploy.sh [--reset-db] [--set KEY=VALUE]... [--skip-ci-check]
#                     [--schema-unchanged]
#
# In order, stopping at the first thing wrong:
#   1. the checkout: no uncommitted changes, HEAD on origin/main, and CI green
#      for HEAD (`gh run list`; --skip-ci-check overrides, for an emergency);
#   2. MAGPIE: the commit docker/Dockerfile pins is on MAGPIE's
#      origin/birdtest-contribute (MAGPIE_DIR, default ~/MAGPIE), since the
#      backend image fetches it from GitHub;
#   3. the stack: login, Terraform outputs, prod.tfvars from the state bucket;
#   4. the schema: a migration the live release applied and this commit
#      changed (0001_initial.sql, edited in place before launch) makes the
#      backend refuse to start, so that is refused unless --reset-db, which
#      empties the database (below). The live release is the backend image's
#      tag in prod.tfvars, a commit hash; one this checkout does not know is
#      refused too, unless --schema-unchanged says it is known to be safe;
#   5. the images, tagged with the commit (12 characters): each one ECR does
#      not hold yet is built (--platform linux/amd64, CARGO_BUILD_JOBS=2 and
#      MAKE_JOBS=3 unless set: an uncapped build has frozen the machine) and
#      pushed. ECR's tags are immutable, so one already there is that commit;
#   6. prod.tfvars pointed at the new tag, with any --set KEY=VALUE (e.g.
#      --set min_magpie_version=0.2.0, which a MAGPIE pin that changes results
#      raises in the same deploy);
#   7. the plan, shown, scanned for a database, bucket or network destroyed or
#      replaced (refused), and applied only once you say so;
#   8. with --reset-db, only now: the site's hostname typed (the prompt names
#      both backup buckets and their dump count), the service stopped, the
#      schema dropped and made empty (scripts/prod-sql.sh), and every version
#      under pg/ in the backups bucket and its DR replica deleted (RUNBOOK,
#      "Resetting the production database");
#   9. the apply, the service started again after a reset, prod.tfvars
#      uploaded, a line in ~/birdtest-releases.log, and the wait until the new
#      release runs and both target groups are healthy (a circuit-breaker
#      rollback is reported, with scripts/rollback.sh as the next step).
#
# After a reset it prints what to do next: register, scripts/confirm-user.sh
# --admin, the input data import.
#
# Needs git, gh (logged in), aws, terraform, jq, docker and curl, the settings
# in ~/.birdtest-env (AWS_PROFILE, STATE_BUCKET, STATE_REGION, REGISTRY,
# SITE), and infra/ initialized with the stack's backend.
set -euo pipefail
# shellcheck source=scripts/lib/ops.sh
source "$(dirname "$0")/lib/ops.sh"

usage() { sed -n '3,4p' "$0" >&2; exit 2; }

reset=0 skip_ci=0 schema_unchanged=0 sets=()
while (($#)); do
  case $1 in
    --reset-db) reset=1 ;;
    --skip-ci-check) skip_ci=1 ;;
    --schema-unchanged) schema_unchanged=1 ;;
    --set) [[ $# -ge 2 && "$2" == *=* ]] || usage; sets+=("$2"); shift ;;
    --set=*=*) sets+=("${1#--set=}") ;;
    -h | --help) sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//' >&2; exit 0 ;;
    *) usage ;;
  esac
  shift
done

ops_init
ops_need git gh aws terraform jq docker curl
cd "$OPS_ROOT"

# 1. The checkout.
[[ -z "$(git status --porcelain --untracked-files=no)" ]] \
  || ops_die "uncommitted changes: deploy a clean checkout (the image tag is the commit)"
git fetch -q origin main || ops_die "git fetch origin main failed"
git merge-base --is-ancestor HEAD origin/main \
  || ops_die "HEAD is not on origin/main: deploy only from main (git checkout main && git pull)"
commit=$(git rev-parse HEAD)
tag=$(git rev-parse --short=12 HEAD)
[[ "$commit" == "$(git rev-parse origin/main)" ]] \
  || ops_say "note: HEAD ($tag) is behind origin/main ($(git rev-parse --short=12 origin/main))"

if ((skip_ci)); then
  ops_say "WARNING: CI not checked (--skip-ci-check)"
else
  ci=$(gh run list --workflow ci.yml --commit "$commit" --limit 1 \
    --json status,conclusion,url --jq '.[0] // empty | "\(.status) \(.conclusion) \(.url)"') \
    || ops_die "gh could not list CI runs (gh auth status?)"
  [[ -n "$ci" ]] || ops_die "no CI run for $tag yet: wait for it, or --skip-ci-check in an emergency"
  read -r ci_status ci_conclusion ci_url <<<"$ci"
  [[ "$ci_status" == completed && "$ci_conclusion" == success ]] \
    || ops_die "CI for $tag is $ci_status/${ci_conclusion:-pending}: $ci_url"
  ops_say "CI green for $tag"
fi

# 2. MAGPIE's pin.
pin=$(git show HEAD:docker/Dockerfile | sed -n 's/^ARG MAGPIE_COMMIT=//p' | head -1)
[[ "$pin" =~ ^[0-9a-f]{40}$ ]] || ops_die "no ARG MAGPIE_COMMIT=<40-hex commit> in docker/Dockerfile"
magpie=${MAGPIE_DIR:-$HOME/MAGPIE}
[[ -d "$magpie/.git" ]] || ops_die "no MAGPIE checkout at $magpie (set MAGPIE_DIR)"
git -C "$magpie" fetch -q origin || ops_die "git fetch in $magpie failed"
git -C "$magpie" branch -r --contains "$pin" 2>/dev/null | grep -qx '[[:space:]]*origin/birdtest-contribute' \
  || ops_die "MAGPIE $pin is not on origin/birdtest-contribute: push it first (the backend image fetches it)"
ops_say "MAGPIE pin ${pin:0:12} is on birdtest-contribute"

# 3. The stack.
ops_load_env
ops_require STATE_BUCKET STATE_REGION REGISTRY
ops_login
ops_stack
ops_tfvars_fetch
live_image=$(ops_tfvar backend_image)
live_tag=$(ops_image_tag "$live_image")
for v in backend_image derived_builder_image frontend_image; do
  img=$(ops_tfvar "$v")
  [[ "$img" == "$REGISTRY/birdtest-"*:* ]] \
    || ops_die "$v in prod.tfvars is $img, not in REGISTRY ($REGISTRY), where this pushes"
done
ops_say "live: $live_tag; deploying: $tag"
if [[ "$live_tag" == "$tag" ]] && ((${#sets[@]} == 0)) && ((reset == 0)); then
  ops_say "$tag is already what prod.tfvars names: nothing to deploy"
  exit 0
fi

# 4. The schema.
schema_rc=0
changed=$(ops_migrations_changed "$live_tag" "$commit") || schema_rc=$?
case $schema_rc in
  0)
    if ((reset)); then
      ops_say "migrations changed since $live_tag; the database will be reset (--reset-db):"
      printf '%s\n' "$changed" >&2
    else
      printf '%s\n' "$changed" >&2
      ops_die "these migrations changed since the live release ($live_tag): its database refuses the new backend (sqlx checksums them). Until launch that means a reset, which deletes every account and the input data: run again with --reset-db (UPDATES_PLAN, \"schema changes keep editing 0001\")"
    fi ;;
  1) ((reset)) && ops_say "no migration changed, but --reset-db was given: the database will be reset anyway" ;;
  2)
    if ((reset || schema_unchanged)); then
      ops_say "the live tag $live_tag is not a commit here; going on (--reset-db or --schema-unchanged)"
    else
      ops_die "the live tag $live_tag is not a commit in this checkout, so whether a migration changed is unknown: git fetch, or --schema-unchanged if you know, or --reset-db"
    fi ;;
esac

# 5. The images.
registry_region=${REGISTRY#*.dkr.ecr.}
registry_region=${registry_region%%.*}
missing=()
for repo in backend derived-builder frontend; do
  if aws ecr describe-images --region "$registry_region" --repository-name "birdtest-$repo" \
       --image-ids "imageTag=$tag" >/dev/null 2>&1; then
    ops_say "birdtest-$repo:$tag is already in ECR"
  else
    missing+=("$repo")
  fi
done
if ((${#missing[@]})); then
  aws ecr get-login-password --region "$registry_region" \
    | docker login --username AWS --password-stdin "$REGISTRY" >/dev/null \
    || ops_die "docker login to $REGISTRY failed"
  build_args=(--pull --platform linux/amd64
    --build-arg "CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}" --build-arg "MAKE_JOBS=${MAKE_JOBS:-3}")
  for repo in "${missing[@]}"; do
    ops_say "building birdtest-$repo:$tag"
    case $repo in
      backend | derived-builder)
        docker build "${build_args[@]}" -f docker/Dockerfile --target "$repo" \
          -t "$REGISTRY/birdtest-$repo:$tag" . ;;
      frontend)
        docker build --pull --platform linux/amd64 -t "$REGISTRY/birdtest-frontend:$tag" frontend ;;
    esac
  done
  for repo in "${missing[@]}"; do
    docker push "$REGISTRY/birdtest-$repo:$tag"
  done
  ops_say "pushed $tag"
fi

# 6. prod.tfvars.
ops_retag "$tag"
for kv in ${sets[@]+"${sets[@]}"}; do
  # Assigned first: a refusal inside "$(...)" as an argument would not stop it.
  value=$(ops_hcl_value "${kv#*=}")
  ops_tfvars_set "${kv%%=*}" "$value"
done
diff -u --label "prod.tfvars (live)" --label "prod.tfvars (this deploy)" \
  "$OPS_TFVARS_BASE" "$OPS_TFVARS" >&2 || true

# 7-9. Plan, the reset if asked, apply, upload, log, wait.
export OPS_RESET=$reset   # read by ops_ship
ops_ship "deployed tag=$tag previous=$live_tag $(ops_release_fields)"
ops_say "deployed $tag"
if ((reset)); then
  ops_reset_checklist "$(ops_site_host)"
fi
