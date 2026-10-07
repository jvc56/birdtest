#!/usr/bin/env bash
# RUNBOOK.md §1's mechanical stages, one subcommand each, in this order:
#
#   scripts/pitr-restore.sh stop                 # 1. stop writes; print the restorable window
#   scripts/pitr-restore.sh restore --time T     # 2-3. restore to a new instance at T
#   scripts/pitr-restore.sh count                # contributors newer than T (before the swap)
#   scripts/pitr-restore.sh swap                 # the renames, then Terraform's state
#   scripts/pitr-restore.sh damaged-host         # the endpoint for the re-apply blocks
#   scripts/pitr-restore.sh repoint              # DATABASE_URL, a new signing key, the service on
#   scripts/pitr-restore.sh finish               # the closing terraform apply
#   scripts/pitr-restore.sh retire               # delete the damaged instance (final snapshot)
#   scripts/pitr-restore.sh status
#
# What stays yours, between the stages, as RUNBOOK §1 says:
#   - the restore time T (ISO 8601 in UTC, e.g. 2026-09-07T02:55:00Z): the
#     latest instant before the damage;
#   - after `swap`, re-applying what the restore undid for security: §1's two
#     blocks in an ops shell (scripts/prod-shell.sh), reviewing the export
#     between them -- `damaged-host` prints the DAMAGED_HOST they need;
#   - after `repoint`, §4's verification and Check artifacts on every
#     leave-generation job; and only once §4 passes, `retire`.
#
# Read RUNBOOK §1 before starting: this script does its steps, not its
# thinking. Each stage confirms before it changes anything. The incident's
# stamp and restore time are kept in ~/.birdtest-pitr/state (BIRDTEST_PITR_DIR)
# so later stages, in another shell, use the same ones. A stage that must not
# run twice refuses: `restore` while a birdtest-restore-* instance exists
# (it resumes the wait for its own), `count` once the swap has begun (it
# would count the restored instance), `repoint` once done (a second key would
# end every session again; --again if you mean it). `swap` works out from
# the instances and the state which of its steps are still to do, so after a
# stop part-way it is run again.
#
# Needs aws, terraform, jq, python3, openssl, curl, the settings in
# ~/.birdtest-env and infra/ initialized with the stack's backend.
set -euo pipefail
# shellcheck source=scripts/lib/ops.sh
source "$(dirname "$0")/lib/ops.sh"

usage() { sed -n '4,12p' "$0" >&2; exit 2; }
stage=${1:-}
[[ -n "$stage" ]] || usage
shift

ops_init
ops_need aws terraform jq python3 openssl curl
ops_load_env
ops_login
ops_stack
db=$OPS_CLUSTER   # rds.tf: identifier = local.name
pitr_dir=${BIRDTEST_PITR_DIR:-$HOME/.birdtest-pitr}
state_file=$pitr_dir/state
mkdir -p "$pitr_dir"

state_get() { [[ -r "$state_file" ]] && sed -n "s/^$1=//p" "$state_file" | tail -1; true; }
state_set() { printf '%s=%s\n' "$1" "$2" >> "$state_file"; }
need_stamp() {
  stamp=$(state_get STAMP)
  restore_time=$(state_get RESTORE_TIME)
  [[ -n "$stamp" && -n "$restore_time" ]] || ops_die "no restore recorded in $state_file: run  $0 restore --time <T>  first"
  restored=$db-restore-$stamp
  damaged=$db-damaged-$stamp
}
exists() { aws rds describe-db-instances --db-instance-identifier "$1" >/dev/null 2>&1; }
status_of() {
  aws rds describe-db-instances --db-instance-identifier "$1" \
    --query 'DBInstances[0].DBInstanceStatus' --output text 2>/dev/null || true
}
# Waits until instance $1 exists under its new name, then until it is
# available. A rename returns before it takes effect, and the CLI's waiter on
# a name that does not exist yet fails at once rather than waiting.
renamed() {
  local tries=0
  until exists "$1"; do
    tries=$((tries + 1))
    ((tries < 90)) || ops_die "no instance named $1 after 15 minutes (or the login expired): run  $0 swap  again"
    sleep 10
  done
  wait_available "$1"
}
wait_available() {
  local round
  for round in 1 2 3 4 5 6; do
    aws rds wait db-instance-available --db-instance-identifier "$1" && return 0
    ops_say "$1 is not available yet (wait $round of 6)"
  done
  ops_die "$1 is still not available after three hours"
}
# The DbiResourceId Terraform's state holds for the database, or nothing.
state_resource_id() {
  terraform -chdir="$OPS_INFRA" show -json \
    | jq -r '.values.root_module.resources[]? | select(.address == "aws_db_instance.main") | .values.resource_id // empty'
}
resource_id() {
  aws rds describe-db-instances --db-instance-identifier "$1" \
    --query 'DBInstances[0].DbiResourceId' --output text
}

case $stage in

stop)
  ops_say "RUNBOOK §1 step 1: stop writes. Workers submitting into a database about to be replaced have their results discarded; the -down alarms fire in ten minutes and clear when the service is back."
  ops_confirm "Stop the service $OPS_SERVICE?" || ops_die "nothing changed"
  ops_stop_service
  latest=$(aws rds describe-db-instances --db-instance-identifier "$db" \
    --query 'DBInstances[0].LatestRestorableTime' --output text)
  ops_say "latest restorable time of $db: $latest"
  ops_say "choose the restore point -- the latest instant before the damage -- then:  $0 restore --time <ISO time in UTC, e.g. 2026-09-07T02:55:00Z>"
  ;;

restore)
  [[ "${1:-}" == --time && -n "${2:-}" ]] || usage
  time=$2
  [[ "$time" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}(:[0-9]{2}(\.[0-9]+)?)?Z$ ]] \
    || ops_die "the time must be ISO 8601 in UTC, ending in Z: e.g. 2026-09-07T02:55:00Z"
  stamp=$(state_get STAMP)
  if [[ -n "$stamp" ]] && exists "$db-restore-$stamp"; then
    [[ "$(state_get RESTORE_TIME)" == "$time" ]] \
      || ops_die "$db-restore-$stamp was restored to $(state_get RESTORE_TIME), not $time: delete it (and $state_file) to start over"
    ops_say "$db-restore-$stamp exists already: finishing its setup"
  else
    others=$(aws rds describe-db-instances \
      --query "DBInstances[?starts_with(DBInstanceIdentifier, '$db-restore-')].DBInstanceIdentifier" --output text)
    [[ -z "$others" || "$others" == None ]] \
      || ops_die "a restore instance exists already ($others): finish or delete it first; $state_file records the current one"
    if [[ -n "$(state_get STAMP)" ]]; then
      ops_die "$state_file records the restore $(state_get STAMP), whose $db-restore-* instance is no longer there (swapped, or deleted): restore does not run twice. To start a new incident, move the file aside"
    fi
    stamp=$(date -u +%Y%m%d%H%M)
    # The source's parameter group (a generated name): a restore without one
    # gets the default group and loses the WAL settings leave-generation
    # merges need. Its class: the restore serves production once repointed,
    # before the closing apply puts anything else back.
    parameter_group=$(aws rds describe-db-instances --db-instance-identifier "$db" \
      --query 'DBInstances[0].DBParameterGroups[0].DBParameterGroupName' --output text)
    instance_class=$(aws rds describe-db-instances --db-instance-identifier "$db" \
      --query 'DBInstances[0].DBInstanceClass' --output text)
    # A storage ceiling, which a point-in-time restore is not promised to carry
    # over: without one the restored instance cannot grow at all. At least 30%
    # above the allocation (RDS refuses one less than 10% above, and warns past
    # 80%), never past RDS's 65,536 GiB, and none past 59,578 GiB allocated.
    read -r allocated max_storage < <(aws rds describe-db-instances --db-instance-identifier "$db" \
      --query 'DBInstances[0].[AllocatedStorage,MaxAllocatedStorage]' --output text)
    [[ "$allocated" =~ ^[0-9]+$ ]] || ops_die "could not read $db's allocated storage"
    [[ "$max_storage" =~ ^[0-9]+$ ]] || max_storage=0
    min_ceiling=$(( (allocated * 130 + 99) / 100 ))
    ((min_ceiling <= 65536)) || min_ceiling=65536
    ((max_storage >= min_ceiling)) || max_storage=$min_ceiling
    ceiling=(--max-allocated-storage "$max_storage")
    ((allocated * 11 <= 655360)) || { ceiling=(); max_storage=""; }
    security_group=$(tf -raw db_security_group_id)
    cat >&2 <<EOF

  Restore $db to $time as $db-restore-$stamp:
    class $instance_class, parameter group $parameter_group,
    storage ceiling ${max_storage:-none} GiB (allocated $allocated),
    subnet group $db-db, security group $security_group, not public.
  The original is left untouched until the restore is confirmed good.

EOF
    ops_confirm "Start the restore?" || ops_die "nothing changed"
    aws rds restore-db-instance-to-point-in-time \
      --source-db-instance-identifier "$db" \
      --target-db-instance-identifier "$db-restore-$stamp" \
      --restore-time "$time" \
      --db-subnet-group-name "$db-db" \
      --vpc-security-group-ids "$security_group" \
      --db-parameter-group-name "$parameter_group" \
      --no-publicly-accessible \
      --db-instance-class "$instance_class" \
      ${ceiling[@]+"${ceiling[@]}"} \
      --query 'DBInstance.DBInstanceIdentifier' --output text >/dev/null
    state_set STAMP "$stamp"
    state_set RESTORE_TIME "$time"
    state_set CEILING "$max_storage"
    ops_say "restoring as $db-restore-$stamp (recorded in $state_file)"
  fi
  wait_available "$db-restore-$stamp"
  # A restored instance does not inherit the source's backup settings.
  aws rds modify-db-instance --db-instance-identifier "$db-restore-$stamp" \
    --backup-retention-period 30 --deletion-protection --apply-immediately \
    --query 'DBInstance.DBInstanceIdentifier' --output text >/dev/null
  ops_say "$db-restore-$stamp is available, with backups and deletion protection on"
  ops_say "next, while $db is still the damaged instance:  $0 count"
  ;;

count)
  need_stamp
  if exists "$damaged" || ! exists "$restored"; then
    ops_die "the swap has begun: $db is no longer the damaged instance, and this would count the restored one (0). RUNBOOK §1, \"Contributors whose identity is newer\", says how to count on the damaged one by hand"
  fi
  ops_say "contributors whose identity is newer than $restore_time, on $db (the damaged instance):"
  # RESTORE_TIME was checked as ISO 8601 when recorded: a literal, not SQL.
  INFRA_DIR=$OPS_INFRA "$OPS_ROOT/scripts/prod-sql.sh" "
    SELECT 'anonymous_workers' AS identities, count(*) FROM anonymous_workers WHERE first_seen_at > '$restore_time'
    UNION ALL SELECT 'api_keys', count(*) FROM api_keys WHERE created_at > '$restore_time'
    UNION ALL SELECT 'users', count(*) FROM users WHERE created_at > '$restore_time'" \
    | tee "$pitr_dir/counts-$stamp.txt"
  state_set COUNTED 1
  ops_say "these contributors are stopped by the restore (RUNBOOK §1): say so where they will read it. Next:  $0 swap"
  ;;

swap)
  need_stamp
  [[ "${1:-}" == --skip-count || "$(state_get COUNTED)" == 1 ]] \
    || ops_die "count first ($0 count): after the swap it cannot be done this way (--skip-count to go on without)"
  [[ -e "$OPS_TFVARS" ]] || ops_tfvars_fetch
  confirmed=0
  while :; do
    has_p=0 has_r=0 has_d=0
    exists "$db" && has_p=1
    exists "$restored" && has_r=1
    exists "$damaged" && has_d=1
    if ((has_p && has_r && !has_d)); then
      [[ "$(status_of "$restored")" == available ]] || wait_available "$restored"
      if [[ "$(status_of "$db")" != renaming ]]; then
        if ((!confirmed)); then
          ops_say "renaming $db (the damaged instance) to $damaged, then $restored to $db; the endpoint moves with the name"
          ops_confirm_typed swap "rename the instances"
          confirmed=1
        fi
        aws rds modify-db-instance --db-instance-identifier "$db" \
          --new-db-instance-identifier "$damaged" --apply-immediately \
          --query 'DBInstance.DBInstanceIdentifier' --output text >/dev/null
      fi
      renamed "$damaged"
    elif ((has_d && has_r && !has_p)); then
      if [[ "$(status_of "$restored")" != renaming ]]; then
        if ((!confirmed)); then
          ops_confirm_typed swap "rename $restored to $db"
          confirmed=1
        fi
        aws rds modify-db-instance --db-instance-identifier "$restored" \
          --new-db-instance-identifier "$db" --apply-immediately \
          --query 'DBInstance.DBInstanceIdentifier' --output text >/dev/null
      fi
      renamed "$db"
    elif ((has_d && has_p && !has_r)); then
      break
    else
      ops_die "unexpected instances: $db=$has_p $restored=$has_r $damaged=$has_d (1 = exists). Look with  aws rds describe-db-instances --query 'DBInstances[].DBInstanceIdentifier'  and finish by hand from RUNBOOK §1"
    fi
  done
  ops_say "renamed: $db is the restored instance, $damaged the damaged one"
  # Terraform's state holds the database by resource id (db-...), not name, so
  # it would follow the damaged one under its new name: point it at the
  # restored one. After a `state rm` the import must succeed before anything
  # else -- with nothing in the state, the next apply creates an empty database.
  want=$(resource_id "$db")
  have=$(state_resource_id)
  if [[ "$have" == "$want" ]]; then
    ops_say "Terraform's state already holds $db ($want)"
  else
    if [[ -n "$have" ]]; then
      [[ "$have" == "$(resource_id "$damaged")" ]] \
        || ops_die "Terraform's state holds $have, which is neither $db ($want) nor $damaged: stop and look"
      terraform -chdir="$OPS_INFRA" state rm aws_db_instance.main
    fi
    terraform -chdir="$OPS_INFRA" import -input=false "${OPS_VARFILES[@]}" aws_db_instance.main "$db" \
      || ops_die "the import failed and the state holds no database: fix the cause and run  $0 swap  again before any apply"
    [[ "$(state_resource_id)" == "$want" ]] || ops_die "after the import the state does not hold $want: stop and look"
    ops_say "Terraform's state now holds $db ($want)"
  fi
  state_set SWAPPED 1
  ops_say "next, before repointing: re-apply what the restore undid for security (RUNBOOK §1), in an ops shell, with  $0 damaged-host"
  ;;

damaged-host)
  need_stamp
  exists "$damaged" || ops_die "no $damaged: run  $0 swap  first"
  host=$(aws rds describe-db-instances --db-instance-identifier "$damaged" \
    --query 'DBInstances[0].Endpoint.Address' --output text)
  printf '%s\n' "$host"
  ops_say "in scripts/prod-shell.sh, set  DAMAGED_HOST='$host'  and run RUNBOOK §1's export block, review, then its apply block (restore time $restore_time)"
  ;;

repoint)
  need_stamp
  [[ "$(state_get SWAPPED)" == 1 ]] || ops_die "swap first ($0 swap)"
  if [[ "$(state_get REPOINTED)" == 1 && "${1:-}" != --again ]]; then
    ops_die "already repointed: a second run writes another signing key and ends every session again (--again if you mean that)"
  fi
  [[ "$(state_resource_id)" == "$(resource_id "$db")" ]] || ops_die "Terraform's state does not hold $db: run  $0 swap  again"
  cat >&2 <<EOF

  Repointing starts the service on the restored instance. After that the
  re-apply blocks of RUNBOOK §1 refuse to run (the restored instance writes
  audit rows of its own), so they must be done -- or, if the damaged instance
  cannot be read at all, every reset link spent and the incident notes
  written. If the master password was rotated after $restore_time, set it on
  $db first (RUNBOOK, "Rotating the database password"): the restore has the
  password of that time.

EOF
  ops_confirm_typed repoint "point the application at $db and start it"
  endpoint=$(aws rds describe-db-instances --db-instance-identifier "$db" \
    --query 'DBInstances[0].Endpoint.Address' --output text)
  url_param=$(tf -json ssm_parameter_names | jq -r '.[] | select(endswith("/DATABASE_URL"))')
  key_param=$(tf -json ssm_parameter_names | jq -r '.[] | select(endswith("/SESSION_SIGNING_KEY"))')
  URL=$(aws ssm get-parameter --name "$url_param" --with-decryption --query Parameter.Value --output text)
  export URL ENDPOINT=$endpoint
  # shellcheck disable=SC2016
  new_url=$(python3 -c 'import os, urllib.parse as u
p = u.urlsplit(os.environ["URL"])
print(p._replace(netloc=p.netloc.rsplit("@", 1)[0] + "@" + os.environ["ENDPOINT"] + ":5432").geturl())')
  unset URL
  [[ "$new_url" == *"@$endpoint:5432"* ]] || ops_die "could not build the new DATABASE_URL"
  # The service starts only once both are written: started on the old key, it
  # would honour again the sessions the new one ends.
  aws ssm put-parameter --name "$url_param" --type SecureString --overwrite --value "$new_url" >/dev/null
  unset new_url
  ops_say "$url_param now points at $endpoint"
  # A new signing key ends every session: one ended on the damaged instance
  # after the restore point matches the restored instance's session
  # generation again, and only the key tells it from one that was not.
  aws ssm put-parameter --name "$key_param" --type SecureString --overwrite \
    --value "$(openssl rand -hex 32)" >/dev/null
  state_set REPOINTED 1
  ops_say "$key_param rotated: every session ends"
  ops_start_service
  ops_wait_release ""
  ops_say "next: RUNBOOK §4 (scripts/prod-psql.sh scripts/ops-sql/check-restore.sql, and check 3b), Check artifacts on every leave-generation job (§3), then  $0 finish"
  ;;

finish)
  need_stamp
  [[ "$(state_get REPOINTED)" == 1 ]] || ops_die "repoint first ($0 repoint)"
  ops_tfvars_fetch
  # The closing apply sets the ceiling back to five times db_allocated_storage
  # (capped at 65,536), and RDS refuses one less than a tenth above the
  # allocation: a ceiling the restore had to raise needs the variable raised.
  restored_ceiling=$(state_get CEILING)
  if [[ "$restored_ceiling" =~ ^[0-9]+$ ]]; then
    var=$(ops_tfvar db_allocated_storage)
    if [[ -z "$var" ]]; then
      var=$(awk '/^variable "db_allocated_storage"/ { f = 1 } f && /^[ \t]*default[ \t]*=/ { print $3; exit }' "$OPS_INFRA/variables.tf")
    fi
    if [[ "$var" =~ ^[0-9]+$ ]] && ((var * 5 < restored_ceiling && var * 5 < 65536)); then
      need=$(( (restored_ceiling + 4) / 5 ))
      ops_say "the restore's ceiling ($restored_ceiling GiB) is above db_allocated_storage ($var) x 5"
      ops_confirm "Set db_allocated_storage = $need in prod.tfvars for this apply?" \
        || ops_die "raise db_allocated_storage first (RUNBOOK §1), then run this again"
      ops_tfvars_set db_allocated_storage "$need"
    fi
  fi
  ops_say "the closing apply: the restore set none of Multi-AZ, the backup window or tag copying"
  ops_plan_gate
  [[ -n "$OPS_PLAN_EMPTY" ]] || ops_apply_plan
  ops_tfvars_upload
  state_set FINISHED 1
  ops_say "done. Only once RUNBOOK §4 passes:  $0 retire"
  ;;

retire)
  need_stamp
  [[ "$(state_get FINISHED)" == 1 ]] || ops_die "finish first ($0 finish)"
  exists "$damaged" || ops_die "no $damaged"
  cat >&2 <<EOF

  Deleting $damaged, with a final snapshot $damaged-final. Its audit rows
  since $restore_time are the only record of what happened then (the
  re-apply applied them but did not copy them): only once RUNBOOK §4 passes.

EOF
  ops_confirm_typed "$damaged" "delete it"
  aws rds modify-db-instance --db-instance-identifier "$damaged" --no-deletion-protection \
    --apply-immediately --query 'DBInstance.DBInstanceIdentifier' --output text >/dev/null
  wait_available "$damaged"
  aws rds delete-db-instance --db-instance-identifier "$damaged" \
    --final-db-snapshot-identifier "$damaged-final" \
    --query 'DBInstance.DBInstanceStatus' --output text >/dev/null
  state_set RETIRED 1
  ops_say "deleting $damaged; its final snapshot is $damaged-final. Move $state_file aside to close the incident."
  ;;

status)
  if [[ -r "$state_file" ]]; then cat "$state_file"; else ops_say "no incident recorded in $state_file"; fi
  aws rds describe-db-instances \
    --query "DBInstances[?starts_with(DBInstanceIdentifier, '$db')].[DBInstanceIdentifier, DBInstanceStatus, DbiResourceId]" \
    --output text
  ops_say "Terraform's state holds: $(state_resource_id)"
  ;;

*) usage ;;
esac
