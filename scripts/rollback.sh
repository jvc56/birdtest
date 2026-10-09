#!/usr/bin/env bash
# Put an earlier release's images back (RUNBOOK.md, "Rolling back a deploy").
#
#   scripts/rollback.sh [--to TAG] [--reset-db] [--min-magpie-version V]
#
# A release is a backend tag and a frontend tag, which differ when a deploy
# rebuilt only one of the two (scripts/deploy.sh): the release log records
# both for every release, and the pair a tag names is read from there
# (ops_release_images). A tag the log has no images for is both at that tag.
#
# First it looks at what ECS runs. If a service's circuit breaker already
# rolled a release back (its -deploy-failed mail), that service runs an older
# image than prod.tfvars names, and the next apply of any kind would deploy
# the broken one again: what ECS runs is then the default target, and this
# makes Terraform agree with it. Otherwise the default is the release before
# the live one, from ~/birdtest-releases.log. The tags the log knows are
# listed and you choose (Enter takes the default; --to skips the question).
#
# Then the runbook's questions: whether the release being undone added an
# enum value (a new job type) that a job uses -- deactivate those first, or
# the older image stops every claim -- and, when the log says the target ran
# with another min_magpie_version, whether to put that back (the backend
# refuses to start below its own MAGPIE). --min-magpie-version sets it
# outright.
#
# A target backend whose migrations differ from the live backend's cannot
# start on this database (until launch 0001 is edited in place): that is
# refused unless --reset-db, which empties the database as scripts/deploy.sh
# --reset-db does. A rollback of the frontend alone never stops the backend.
# Then the plan gate, apply, upload, release log and health wait, as
# deploy.sh. Nothing is built: the images must be in ECR.
#
# Needs git, aws, terraform, jq, curl, the settings in ~/.birdtest-env and
# infra/ initialized with the stack's backend.
set -euo pipefail
# shellcheck source=scripts/lib/ops.sh
source "$(dirname "$0")/lib/ops.sh"

usage() { sed -n '4p' "$0" >&2; exit 2; }

target="" reset=0 floor=""
while (($#)); do
  case $1 in
    --to) [[ $# -ge 2 ]] || usage; target=$2; shift ;;
    --reset-db) reset=1 ;;
    --min-magpie-version) [[ $# -ge 2 ]] || usage; floor=$2; shift ;;
    *) usage ;;
  esac
  shift
done

ops_init
ops_need git aws terraform jq curl
ops_load_env
ops_require STATE_BUCKET STATE_REGION
ops_login
ops_stack
ops_tfvars_fetch
state_backend=$(ops_image_tag "$(ops_tfvar backend_image)")
state_frontend=$(ops_image_tag "$(ops_tfvar frontend_image)")
state_tag=$(ops_newer_tag "$state_backend" "$state_frontend")
describe() {  # backend frontend -> how a release reads in a message
  if [[ "$1" == "$2" ]]; then printf '%s' "$1"; else printf 'backend %s, frontend %s' "$1" "$2"; fi
}

# What ECS runs, against what Terraform names.
running_backend=$(ops_running_backend_image) || running_backend=""
running_frontend=$(ops_running_frontend_image) || running_frontend=""
broken_backend="" broken_frontend=""
if [[ -n "$running_backend" && "$running_backend" != None ]]; then
  broken_backend=$(ops_image_tag "$running_backend")
  [[ "$broken_backend" != "$state_backend" ]] || broken_backend=""
fi
if [[ -n "$running_frontend" && "$running_frontend" != None ]]; then
  broken_frontend=$(ops_image_tag "$running_frontend")
  [[ "$broken_frontend" != "$state_frontend" ]] || broken_frontend=""
fi
default="" ecs_pair=""
if [[ -n "$broken_backend$broken_frontend" ]]; then
  ecs_pair="${broken_backend:-$state_backend} ${broken_frontend:-$state_frontend}"
  ops_say "ECS runs $(describe "${broken_backend:-$state_backend}" "${broken_frontend:-$state_frontend}"), but prod.tfvars (and Terraform) name $(describe "$state_backend" "$state_frontend"):"
  ops_say "ECS's circuit breaker rolled the release back. Recent events:"
  for svc in ${broken_backend:+"$OPS_SERVICE"} ${broken_frontend:+"$OPS_FRONTEND_SERVICE"}; do
    aws ecs describe-services --cluster "$OPS_CLUSTER" --services "$svc" \
      --query 'services[0].events[:6].[createdAt, message]' --output text >&2 || true
  done
  default=${broken_backend:-$broken_frontend}
fi

# The logged releases, newest first, with the images each ran; the live one
# (the same images as prod.tfvars) left out.
logged=""
while read -r t; do
  [[ -n "$t" ]] || continue
  pair=$(ops_release_images "$t")
  [[ "$pair" != "$state_backend $state_frontend" ]] || continue
  logged+="$t $pair"$'\n'
done < <(ops_logged_tags)
if [[ -z "$default" ]]; then
  default=$(printf '%s' "$logged" | awk 'NR == 1 { print $1 }')
fi

if [[ -z "$target" ]]; then
  ops_say "live (prod.tfvars): $(describe "$state_backend" "$state_frontend")"
  if [[ -n "$logged" ]]; then
    ops_say "earlier releases in $OPS_RELEASE_LOG, newest first:"
    printf '%s' "$logged" | head -8 | while read -r t b f; do
      if [[ "$b" == "$t" && "$f" == "$t" ]]; then echo "    $t"; else echo "    $t  (backend $b, frontend $f)"; fi
    done >&2
  fi
  [[ -n "$default" ]] || ops_die "no earlier release known: give one with --to TAG"
  printf 'Roll back to which tag? [%s] ' "$default" >&2
  ops_read target || target=""
  target=${target:-$default}
fi

# The images the target means: what ECS runs, when that is the default
# taken; otherwise the release log's record of it.
if [[ -n "$ecs_pair" && "$target" == "$default" ]]; then
  pair=$ecs_pair
else
  pair=$(ops_release_images "$target")
fi
read -r target_backend target_frontend <<<"$pair"
[[ "$target_backend $target_frontend" != "$state_backend $state_frontend" ]] \
  || ops_die "$target is already what prod.tfvars names"
ops_say "rolling back to $(describe "$target_backend" "$target_frontend")"

# The images must exist: nothing is built here.
for ref in "backend:$target_backend" "derived-builder:$target_backend" "frontend:$target_frontend"; do
  aws ecr describe-images --repository-name "birdtest-${ref%%:*}" --image-ids "imageTag=${ref#*:}" >/dev/null 2>&1 \
    || ops_die "birdtest-$ref is not in ECR"
done

# The schema the live backend left, if the backend changes.
schema_rc=1
if [[ "$target_backend" != "$state_backend" ]]; then
  schema_rc=0
  changed=$(ops_migrations_changed "$target_backend" "$state_backend") || schema_rc=$?
fi
case $schema_rc in
  0)
    printf '%s\n' "$changed" >&2
    if ((reset)); then
      ops_say "these migrations differ between $target_backend and the live $state_backend: the database will be reset (--reset-db)"
    else
      ops_die "these migrations differ between $target_backend and the live backend ($state_backend): $target_backend's backend cannot start on this database (sqlx checksums them). Until launch that needs a reset: --reset-db"
    fi ;;
  2)
    if ((reset)); then
      ops_say "$target_backend or $state_backend is not a commit here; going on with a reset (--reset-db)"
    else
      ops_die "$target_backend or $state_backend is not a commit in this checkout (git fetch), so whether the migrations differ is unknown; --reset-db if they do"
    fi ;;
esac

if [[ "$target_backend" != "$state_backend" ]]; then
  ops_say "If $state_backend added an enum value (a new job type, say) that a job uses, deactivate every such job first: the older image fails to read it and one active job of an unknown type stops every claim."
  ops_confirm "Done, or nothing to do?" || ops_die "stopped: nothing changed"
fi

current_floor=$(ops_tfvar min_magpie_version)
if [[ -z "$floor" ]]; then
  logged_floor=$(ops_logged_floor "$target")
  if [[ -n "$logged_floor" && "$logged_floor" != "$current_floor" ]]; then
    ops_say "$target ran with min_magpie_version ${logged_floor:-<default>}; prod.tfvars now says ${current_floor:-<default>}"
    if ops_confirm "Put $logged_floor back? (the backend refuses to start below its own MAGPIE; it readmits the builds the raise kept out)"; then
      floor=$logged_floor
    fi
  fi
fi

ops_retag "$target_backend" backend
ops_retag "$target_frontend" frontend
if [[ -n "$floor" ]]; then
  value=$(ops_hcl_value "$floor")
  ops_tfvars_set min_magpie_version "$value"
fi
diff -u --label "prod.tfvars (live)" --label "prod.tfvars (rollback)" \
  "$OPS_TFVARS_BASE" "$OPS_TFVARS" >&2 || true
ops_say "the apply uses this checkout's infra/: a task-definition change the release made stays unless it is reverted here too"

export OPS_RESET=$reset   # read by ops_ship
ops_ship "rolled-back tag=$target previous=$state_tag $(ops_release_fields)"
ops_say "rolled back to $target"
if ((reset)); then
  ops_reset_checklist "$(ops_site_host)"
fi
