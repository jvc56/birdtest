#!/usr/bin/env bash
# Put back a known-good older version of a leave-generation KLV (RUNBOOK.md
# §3, "Corrupted object with a known-good older version").
#
#   scripts/restore-artifact.sh JOB_ID GENERATION
#
# Lists the versions the artifacts bucket keeps of
# leaves/<job>/generation-<n>.klv2, newest first, and asks which to restore:
# the one whose hash matches what the admin job page's "Check artifacts"
# report records. That choice is yours. The chosen version is copied over the
# current object (the bucket is versioned, so the current bytes stay as a
# noncurrent version); the bucket is never rolled back as a whole. Then press
# Check artifacts (without force) on the job's admin page: until it records
# the restored object's hash, workers decline the next generation's tasks.
#
# Needs aws, terraform, jq, the settings in ~/.birdtest-env and infra/
# initialized with the stack's backend.
set -euo pipefail
# shellcheck source=scripts/lib/ops.sh
source "$(dirname "$0")/lib/ops.sh"

[[ $# == 2 ]] || { sed -n '5p' "$0" >&2; exit 2; }
job=$(printf '%s' "$1" | tr 'A-F' 'a-f')
generation=$2
[[ "$job" =~ ^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$ ]] || ops_die "not a job id: $1"
[[ "$generation" =~ ^(0|[1-9][0-9]{0,5})$ ]] || ops_die "not a generation number: $2"

ops_init
ops_need aws terraform jq
ops_load_env
ops_login
ops_stack
bucket=$(tf -raw artifacts_bucket)
key="leaves/$job/generation-$generation.klv2"

aws s3api list-object-versions --bucket "$bucket" --prefix "$key" --output json > "$OPS_TMP/versions.json"
jq -r --arg k "$key" '[.Versions[]? | select(.Key == $k)] | sort_by(.LastModified) | reverse
  | to_entries[] | "\(.key + 1)\t\(.value.VersionId)\t\(.value.LastModified)\t\(.value.Size)\t\(.value.ETag)\(if .value.IsLatest then "\tcurrent" else "" end)"' \
  "$OPS_TMP/versions.json" > "$OPS_TMP/list"
[[ -s "$OPS_TMP/list" ]] || ops_die "no versions of s3://$bucket/$key"
ops_say "versions of s3://$bucket/$key, newest first (n, version id, time, bytes, ETag):"
sed 's/^/    /' "$OPS_TMP/list" >&2
printf 'Restore which (its number or version id)? ' >&2
choice=""
ops_read choice || choice=""
[[ -n "$choice" ]] || ops_die "nothing chosen: nothing changed"
version=$(awk -F'\t' -v c="$choice" '$1 == c || $2 == c { print $2; exit }' "$OPS_TMP/list")
[[ -n "$version" ]] || ops_die "no such version: $choice"
if grep -q "^[0-9]*	$version	.*	current$" "$OPS_TMP/list"; then
  ops_die "$version is already the current version: nothing to do"
fi
ops_confirm "Copy version $version over the current s3://$bucket/$key?" || ops_die "nothing changed"
aws s3api copy-object --bucket "$bucket" --copy-source "$bucket/$key?versionId=$version" --key "$key" \
  --query 'VersionId' --output text >&2
ops_say "restored. Now press Check artifacts (not Force rebuild) on the job's admin page."
