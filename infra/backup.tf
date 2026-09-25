# Nightly logical backups of Postgres. See PLAN.md's "Backups and Restore"; in short, RDS
# automated backups (rds.tf) are the primary mechanism and these dumps are the
# portable secondary -- the one that can be restored selectively, read on a
# laptop, or carried out of the account.

# --- Encryption key --------------------------------------------------------
# A dump contains argon2 password hashes, email addresses, API key hashes and
# unexpired reset tokens, so it is encrypted with a customer-managed key rather
# than the S3 default. The asymmetry lives in the key policy and the task's
# IAM: the task that writes backups can create one and cannot read one.

resource "aws_kms_key" "backups" {
  description             = "${local.name} logical database backups"
  enable_key_rotation     = true
  deletion_window_in_days = 30
  policy                  = data.aws_iam_policy_document.backups_key.json
  tags                    = local.tags
}

resource "aws_kms_alias" "backups" {
  name          = "alias/${local.name}-backups"
  target_key_id = aws_kms_key.backups.key_id
}

data "aws_iam_policy_document" "backups_key" {
  # Without this the key is unmanageable: KMS does not fall back to IAM.
  statement {
    sid       = "AccountAdmin"
    actions   = ["kms:*"]
    resources = ["*"]
    principals {
      type        = "AWS"
      identifiers = ["arn:aws:iam::${data.aws_caller_identity.current.account_id}:root"]
    }
  }

  statement {
    sid       = "BackupTaskEncrypt"
    actions   = ["kms:Encrypt", "kms:GenerateDataKey*", "kms:DescribeKey"]
    resources = ["*"]
    principals {
      type        = "AWS"
      identifiers = [aws_iam_role.backup_task.arn]
    }
  }

  # A multipart upload under SSE-KMS needs kms:Decrypt: S3 decrypts the data
  # key to complete it, on the caller's behalf. `aws s3 cp` goes multipart
  # past 8 MB, and a directory-format dump has a file per table, so without
  # this every nightly upload of a real database was refused. Only through S3,
  # and only for this bucket (with a bucket key the context is the bucket's
  # ARN): the task cannot decrypt anything itself, and it holds no
  # s3:GetObject, so the backups stay unreadable to it.
  statement {
    sid       = "BackupTaskMultipartViaS3"
    actions   = ["kms:Decrypt"]
    resources = ["*"]
    principals {
      type        = "AWS"
      identifiers = [aws_iam_role.backup_task.arn]
    }
    condition {
      test     = "StringEquals"
      variable = "kms:ViaService"
      values   = ["s3.${var.region}.amazonaws.com"]
    }
    condition {
      test     = "StringLike"
      variable = "kms:EncryptionContext:aws:s3:arn"
      values   = ["${aws_s3_bucket.backups.arn}*"]
    }
  }

  # Replication has to decrypt to re-encrypt into the DR region's key.
  statement {
    sid       = "ReplicationDecrypt"
    actions   = ["kms:Decrypt", "kms:DescribeKey"]
    resources = ["*"]
    principals {
      type        = "AWS"
      identifiers = [aws_iam_role.backup_replication.arn]
    }
  }
}

resource "aws_kms_key" "backups_dr" {
  provider                = aws.dr
  description             = "${local.name} logical database backups (DR copy)"
  enable_key_rotation     = true
  deletion_window_in_days = 30
  tags                    = local.tags
}

resource "aws_kms_alias" "backups_dr" {
  provider      = aws.dr
  name          = "alias/${local.name}-backups"
  target_key_id = aws_kms_key.backups_dr.key_id
}

# --- Buckets ---------------------------------------------------------------
# Deliberately not a prefix in the artifacts bucket: the ECS task role already
# holds PutObject there, so backups sharing it would be writable by the very
# process a backup exists to recover from.

resource "aws_s3_bucket" "backups" {
  bucket = "${local.name}-backups-${data.aws_caller_identity.current.account_id}"
  # Governance-mode retention is configured below. This flag cannot be set on
  # an existing bucket, which is why it is decided at creation.
  object_lock_enabled = true
  tags                = local.tags
}

resource "aws_s3_bucket_public_access_block" "backups" {
  bucket                  = aws_s3_bucket.backups.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_s3_bucket_versioning" "backups" {
  bucket = aws_s3_bucket.backups.id
  versioning_configuration {
    status = "Enabled"
  }
}

resource "aws_s3_bucket_server_side_encryption_configuration" "backups" {
  bucket = aws_s3_bucket.backups.id
  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm     = "aws:kms"
      kms_master_key_id = aws_kms_key.backups.arn
    }
    # One data key per request would be one KMS call per file of a parallel
    # directory-format dump; a bucket key makes it one per dump.
    bucket_key_enabled = true
  }
}

resource "aws_s3_bucket_object_lock_configuration" "backups" {
  bucket = aws_s3_bucket.backups.id
  rule {
    default_retention {
      mode = "GOVERNANCE"
      days = var.backup_object_lock_days
    }
  }
}

resource "aws_s3_bucket_lifecycle_configuration" "backups" {
  bucket = aws_s3_bucket.backups.id

  rule {
    id     = "age-out-dumps"
    status = "Enabled"
    filter {
      prefix = "pg/"
    }

    transition {
      days          = 30
      storage_class = "GLACIER_IR"
    }

    expiration {
      days = var.backup_retention_days
    }

    noncurrent_version_expiration {
      noncurrent_days = 90
    }
  }

  rule {
    id     = "abort-incomplete-uploads"
    status = "Enabled"
    filter {}

    abort_incomplete_multipart_upload {
      days_after_initiation = 7
    }
  }
}

resource "aws_s3_bucket" "backups_dr" {
  provider = aws.dr
  bucket   = "${local.name}-backups-dr-${data.aws_caller_identity.current.account_id}"
  # S3 replicates nothing from a source with Object Lock into a destination
  # without it: every replication would fail, and the dumps would never leave
  # the region. No default retention here -- each replica carries its source
  # object's retention. Like the source's, decided at creation: on a stack
  # applied without it, the bucket is replaced (empty it first; its contents
  # are copies).
  object_lock_enabled = true
  tags                = local.tags
}

resource "aws_s3_bucket_public_access_block" "backups_dr" {
  provider                = aws.dr
  bucket                  = aws_s3_bucket.backups_dr.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_s3_bucket_versioning" "backups_dr" {
  provider = aws.dr
  bucket   = aws_s3_bucket.backups_dr.id
  versioning_configuration {
    status = "Enabled"
  }
}

resource "aws_s3_bucket_server_side_encryption_configuration" "backups_dr" {
  provider = aws.dr
  bucket   = aws_s3_bucket.backups_dr.id
  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm     = "aws:kms"
      kms_master_key_id = aws_kms_key.backups_dr.arn
    }
    bucket_key_enabled = true
  }
}

resource "aws_s3_bucket_lifecycle_configuration" "backups_dr" {
  provider = aws.dr
  bucket   = aws_s3_bucket.backups_dr.id

  rule {
    id     = "age-out-dumps"
    status = "Enabled"
    filter {
      prefix = "pg/"
    }

    transition {
      days          = 30
      storage_class = "GLACIER_IR"
    }

    expiration {
      days = var.backup_retention_days
    }

    # Expiring a replica only adds a delete marker; the bytes stay until the
    # noncurrent version goes, as in the source bucket.
    noncurrent_version_expiration {
      noncurrent_days = 90
    }
  }

  rule {
    id     = "abort-incomplete-uploads"
    status = "Enabled"
    filter {}

    abort_incomplete_multipart_upload {
      days_after_initiation = 7
    }
  }
}

# Region loss is only survivable if the dumps already left the region: the
# 24-hour RPO in PLAN.md, "Objectives and failure scenarios" is this rule.
resource "aws_iam_role" "backup_replication" {
  name               = "${local.name}-backups-replication"
  assume_role_policy = data.aws_iam_policy_document.replication_assume.json
  tags               = local.tags
}

data "aws_iam_policy_document" "backup_replication" {
  statement {
    actions   = ["s3:GetReplicationConfiguration", "s3:ListBucket"]
    resources = [aws_s3_bucket.backups.arn]
  }
  statement {
    # The two retention reads: replicating from an Object Lock bucket copies
    # each object's retention, and fails without them.
    actions = [
      "s3:GetObjectVersionForReplication",
      "s3:GetObjectVersionAcl",
      "s3:GetObjectVersionTagging",
      "s3:GetObjectRetention",
      "s3:GetObjectLegalHold",
    ]
    resources = ["${aws_s3_bucket.backups.arn}/*"]
  }
  statement {
    actions   = ["s3:ReplicateObject", "s3:ReplicateTags"]
    resources = ["${aws_s3_bucket.backups_dr.arn}/*"]
  }
  # Decrypt with the source key, encrypt with the destination's.
  statement {
    actions   = ["kms:Decrypt", "kms:DescribeKey"]
    resources = [aws_kms_key.backups.arn]
  }
  statement {
    actions   = ["kms:Encrypt", "kms:GenerateDataKey*", "kms:DescribeKey"]
    resources = [aws_kms_key.backups_dr.arn]
  }
}

resource "aws_iam_role_policy" "backup_replication" {
  role   = aws_iam_role.backup_replication.id
  policy = data.aws_iam_policy_document.backup_replication.json
}

resource "aws_s3_bucket_replication_configuration" "backups" {
  depends_on = [aws_s3_bucket_versioning.backups, aws_s3_bucket_versioning.backups_dr]

  role   = aws_iam_role.backup_replication.arn
  bucket = aws_s3_bucket.backups.id

  rule {
    id     = "all-backups"
    status = "Enabled"
    filter {}

    # Without this, KMS-encrypted objects are silently skipped.
    source_selection_criteria {
      sse_kms_encrypted_objects {
        status = "Enabled"
      }
    }

    delete_marker_replication {
      status = "Disabled"
    }

    destination {
      bucket        = aws_s3_bucket.backups_dr.arn
      storage_class = "STANDARD_IA"
      encryption_configuration {
        replica_kms_key_id = aws_kms_key.backups_dr.arn
      }
    }
  }
}

# --- The backup task -------------------------------------------------------
# The official postgres image, so pg_dump's major version tracks the server's
# by construction and there is no image of ours to keep rebuilt. The script is
# passed as the command rather than baked in, which keeps `scripts/backup.sh`
# the single copy: `terraform apply` is what deploys a change to it.

resource "aws_iam_role" "backup_task" {
  name               = "${local.name}-backup-task"
  assume_role_policy = data.aws_iam_policy_document.task_assume.json
  tags               = local.tags
}

data "aws_iam_policy_document" "backup_task" {
  # Write-only on the backup prefix, and no ability to delete: a bug in the
  # script cannot destroy previous backups.
  statement {
    actions   = ["s3:PutObject"]
    resources = ["${aws_s3_bucket.backups.arn}/*"]
  }
  statement {
    actions   = ["s3:ListBucket"]
    resources = [aws_s3_bucket.backups.arn]
  }
  statement {
    actions   = ["cloudwatch:PutMetricData"]
    resources = ["*"]
    condition {
      test     = "StringEquals"
      variable = "cloudwatch:namespace"
      values   = ["birdtest/backup"]
    }
  }
}

resource "aws_iam_role_policy" "backup_task" {
  role   = aws_iam_role.backup_task.id
  policy = data.aws_iam_policy_document.backup_task.json
}

# The dump reads DATABASE_URL from the same SSM parameter the service uses, so
# the execution role needs it too.
data "aws_iam_policy_document" "backup_execution_ssm" {
  statement {
    actions   = ["ssm:GetParameters"]
    resources = [aws_ssm_parameter.database_url.arn]
  }
}

resource "aws_iam_role_policy" "backup_execution_ssm" {
  role   = aws_iam_role.execution.id
  policy = data.aws_iam_policy_document.backup_execution_ssm.json
}

resource "aws_ecs_task_definition" "backup" {
  family                   = "${local.name}-backup"
  requires_compatibilities = ["FARGATE"]
  network_mode             = "awsvpc"
  cpu                      = var.backup_task_cpu
  memory                   = var.backup_task_memory
  execution_role_arn       = aws_iam_role.execution.arn
  task_role_arn            = aws_iam_role.backup_task.arn

  # A dump of a tens-of-gigabytes corpus needs somewhere to put it before the
  # upload; the 20 GB Fargate default is not that place.
  ephemeral_storage {
    size_in_gib = var.backup_ephemeral_storage_gib
  }

  container_definitions = jsonencode([
    {
      name      = "backup"
      image     = var.backup_image
      essential = true
      # entryPoint as well as command: the Postgres image's entrypoint script
      # is for starting a server, and this task is not starting one.
      entryPoint = ["/bin/bash", "-c"]
      command    = [file("${path.module}/../scripts/backup.sh")]
      environment = [
        { name = "BACKUP_BUCKET", value = aws_s3_bucket.backups.bucket },
        { name = "BACKUP_KMS_KEY_ARN", value = aws_kms_key.backups.arn },
        { name = "BACKUP_PREFIX", value = "pg" },
        { name = "BACKEND_IMAGE", value = var.backend_image },
        { name = "AWS_REGION", value = var.region },
        { name = "AWS_DEFAULT_REGION", value = var.region },
        { name = "PGDUMP_JOBS", value = tostring(var.backup_dump_jobs) },
      ]
      secrets = [
        { name = "DATABASE_URL", valueFrom = aws_ssm_parameter.database_url.arn }
      ]
      logConfiguration = {
        logDriver = "awslogs"
        options = {
          "awslogs-group"         = aws_cloudwatch_log_group.main.name
          "awslogs-region"        = var.region
          "awslogs-stream-prefix" = "backup"
        }
      }
    }
  ])

  tags = local.tags
}

# --- Schedule --------------------------------------------------------------

data "aws_iam_policy_document" "scheduler_assume" {
  statement {
    actions = ["sts:AssumeRole"]
    principals {
      type        = "Service"
      identifiers = ["scheduler.amazonaws.com"]
    }
  }
}

resource "aws_iam_role" "scheduler" {
  name               = "${local.name}-backup-scheduler"
  assume_role_policy = data.aws_iam_policy_document.scheduler_assume.json
  tags               = local.tags
}

data "aws_iam_policy_document" "scheduler" {
  statement {
    actions   = ["ecs:RunTask"]
    resources = ["${aws_ecs_task_definition.backup.arn_without_revision}:*"]
    condition {
      test     = "ArnLike"
      variable = "ecs:cluster"
      values   = [aws_ecs_cluster.main.arn]
    }
  }
  # RunTask with roles attached requires the caller to be able to pass them.
  statement {
    actions   = ["iam:PassRole"]
    resources = [aws_iam_role.execution.arn, aws_iam_role.backup_task.arn]
    condition {
      test     = "StringEquals"
      variable = "iam:PassedToService"
      values   = ["ecs-tasks.amazonaws.com"]
    }
  }
}

resource "aws_iam_role_policy" "scheduler" {
  role   = aws_iam_role.scheduler.id
  policy = data.aws_iam_policy_document.scheduler.json
}

resource "aws_scheduler_schedule" "backup" {
  name                         = "${local.name}-backup"
  description                  = "Nightly pg_dump to s3://${aws_s3_bucket.backups.bucket}"
  schedule_expression          = var.backup_schedule
  schedule_expression_timezone = "UTC"
  state                        = var.scheduled_tasks_enabled ? "ENABLED" : "DISABLED"

  flexible_time_window {
    mode = "OFF"
  }

  target {
    arn      = aws_ecs_cluster.main.arn
    role_arn = aws_iam_role.scheduler.arn

    ecs_parameters {
      task_definition_arn = aws_ecs_task_definition.backup.arn_without_revision
      launch_type         = "FARGATE"
      task_count          = 1

      network_configuration {
        subnets          = aws_subnet.public[*].id
        security_groups  = [aws_security_group.service.id]
        assign_public_ip = true
      }
    }

    # Retries a RunTask call the scheduler could not make (a throttle, a
    # capacity error) -- not a dump that ran and failed: that is the failure
    # alarm's, and the next night's.
    retry_policy {
      maximum_retry_attempts       = 1
      maximum_event_age_in_seconds = 3600
    }
  }
}

# --- Alerting --------------------------------------------------------------
# Two independent signals, because they fail differently: the first catches a
# dump that ran and broke, the second a dump that never ran at all.

resource "aws_sns_topic" "alerts" {
  name = "${local.name}-alerts"
  tags = local.tags
}

resource "aws_sns_topic_subscription" "alerts_email" {
  topic_arn = aws_sns_topic.alerts.arn
  protocol  = "email"
  endpoint  = var.alert_email
}

resource "aws_cloudwatch_event_rule" "backup_failed" {
  name        = "${local.name}-backup-failed"
  description = "A backup task exited non-zero"

  event_pattern = jsonencode({
    source      = ["aws.ecs"]
    detail-type = ["ECS Task State Change"]
    detail = {
      clusterArn = [aws_ecs_cluster.main.arn]
      lastStatus = ["STOPPED"]
      group      = ["family:${aws_ecs_task_definition.backup.family}"]
      containers = { exitCode = [{ "anything-but" = 0 }] }
    }
  })

  tags = local.tags
}

resource "aws_cloudwatch_event_target" "backup_failed" {
  rule      = aws_cloudwatch_event_rule.backup_failed.name
  target_id = "sns"
  arn       = aws_sns_topic.alerts.arn

  input_transformer {
    input_paths = {
      task   = "$.detail.taskArn"
      reason = "$.detail.stoppedReason"
    }
    input_template = "\"birdtest backup task failed: <task> (<reason>). See the CloudWatch log group ${aws_cloudwatch_log_group.main.name}, stream prefix backup.\""
  }
}

data "aws_iam_policy_document" "alerts_topic" {
  statement {
    actions   = ["SNS:Publish"]
    resources = [aws_sns_topic.alerts.arn]
    # RDS's event subscription (rds.tf) publishes as events.rds.
    principals {
      type        = "Service"
      identifiers = ["events.amazonaws.com", "cloudwatch.amazonaws.com", "events.rds.amazonaws.com"]
    }
    # Only on this account's behalf: a service principal alone would let any
    # account's rule or alarm publish here.
    condition {
      test     = "StringEquals"
      variable = "aws:SourceAccount"
      values   = [data.aws_caller_identity.current.account_id]
    }
  }
}

resource "aws_sns_topic_policy" "alerts" {
  arn    = aws_sns_topic.alerts.arn
  policy = data.aws_iam_policy_document.alerts_topic.json
}

# The staleness alarm. `treat_missing_data = "breaching"` is the entire point:
# it fires when the metric stops arriving, which is what a schedule that
# silently stopped firing looks like. A metric that is merely low never
# happens -- the script emits 1 or nothing.
resource "aws_cloudwatch_metric_alarm" "backup_stale" {
  alarm_name        = "${local.name}-backup-stale"
  alarm_description = "No successful birdtest backup in 36 hours"
  namespace         = "birdtest/backup"
  metric_name       = "Success"
  statistic         = "Maximum"
  # 36 hours, as three 12-hour periods: CloudWatch caps a single alarm period
  # at one day, so the window is expressed in the evaluation count instead.
  period              = 43200
  evaluation_periods  = 3
  threshold           = 1
  comparison_operator = "LessThanThreshold"
  treat_missing_data  = "breaching"
  alarm_actions       = [aws_sns_topic.alerts.arn]
  ok_actions          = [aws_sns_topic.alerts.arn]
  tags                = local.tags
}

# --- Restore drill ---------------------------------------------------------
# A restore procedure that has never run is a hypothesis. This one restores the
# newest dump into a throwaway Postgres of its own, started inside the task,
# every month and runs the verification queries against it (PLAN.md,
# "Drills"). It never connects to the production instance, so it holds no
# credentials for it: the drill's disk is the task's ephemeral storage, which
# must hold the downloaded dump and the restored database side by side.
#
# Its role is the mirror image of the backup task's: read and decrypt, never
# write. Between the two, no single compromised role can both read old backups
# and replace them.

resource "aws_iam_role" "drill_task" {
  name               = "${local.name}-restore-drill"
  assume_role_policy = data.aws_iam_policy_document.task_assume.json
  tags               = local.tags
}

data "aws_iam_policy_document" "drill_task" {
  statement {
    actions   = ["s3:GetObject", "s3:GetObjectVersion"]
    resources = ["${aws_s3_bucket.backups.arn}/*"]
  }
  statement {
    actions   = ["s3:ListBucket"]
    resources = [aws_s3_bucket.backups.arn]
  }
  statement {
    actions   = ["kms:Decrypt", "kms:DescribeKey"]
    resources = [aws_kms_key.backups.arn]
  }
  statement {
    actions   = ["cloudwatch:PutMetricData"]
    resources = ["*"]
    condition {
      test     = "StringEquals"
      variable = "cloudwatch:namespace"
      values   = ["birdtest/backup"]
    }
  }
}

resource "aws_iam_role_policy" "drill_task" {
  role   = aws_iam_role.drill_task.id
  policy = data.aws_iam_policy_document.drill_task.json
}

resource "aws_ecs_task_definition" "restore_drill" {
  family                   = "${local.name}-restore-drill"
  requires_compatibilities = ["FARGATE"]
  network_mode             = "awsvpc"
  cpu                      = var.backup_task_cpu
  memory                   = var.backup_task_memory
  execution_role_arn       = aws_iam_role.execution.arn
  task_role_arn            = aws_iam_role.drill_task.arn

  ephemeral_storage {
    size_in_gib = var.restore_ephemeral_storage_gib
  }

  container_definitions = jsonencode([
    {
      name       = "restore-drill"
      image      = var.backup_image
      essential  = true
      entryPoint = ["/bin/bash", "-c"]
      command    = [file("${path.module}/../scripts/restore-drill.sh")]
      environment = [
        { name = "BACKUP_BUCKET", value = aws_s3_bucket.backups.bucket },
        { name = "BACKUP_PREFIX", value = "pg" },
        { name = "AWS_REGION", value = var.region },
        { name = "AWS_DEFAULT_REGION", value = var.region },
        { name = "PGRESTORE_JOBS", value = tostring(var.backup_dump_jobs) },
        { name = "DRILL_TARGET", value = "local" },
      ]
      logConfiguration = {
        logDriver = "awslogs"
        options = {
          "awslogs-group"         = aws_cloudwatch_log_group.main.name
          "awslogs-region"        = var.region
          "awslogs-stream-prefix" = "restore-drill"
        }
      }
    }
  ])

  tags = local.tags
}

data "aws_iam_policy_document" "drill_scheduler" {
  statement {
    actions   = ["ecs:RunTask"]
    resources = ["${aws_ecs_task_definition.restore_drill.arn_without_revision}:*"]
    condition {
      test     = "ArnLike"
      variable = "ecs:cluster"
      values   = [aws_ecs_cluster.main.arn]
    }
  }
  statement {
    actions   = ["iam:PassRole"]
    resources = [aws_iam_role.execution.arn, aws_iam_role.drill_task.arn]
    condition {
      test     = "StringEquals"
      variable = "iam:PassedToService"
      values   = ["ecs-tasks.amazonaws.com"]
    }
  }
}

resource "aws_iam_role_policy" "drill_scheduler" {
  role   = aws_iam_role.scheduler.id
  policy = data.aws_iam_policy_document.drill_scheduler.json
}

resource "aws_scheduler_schedule" "restore_drill" {
  name                         = "${local.name}-restore-drill"
  description                  = "Monthly restore of the newest dump into a throwaway database"
  schedule_expression          = var.backup_restore_drill_schedule
  schedule_expression_timezone = "UTC"
  state                        = var.scheduled_tasks_enabled && var.restore_drill_enabled ? "ENABLED" : "DISABLED"

  flexible_time_window {
    mode = "OFF"
  }

  target {
    arn      = aws_ecs_cluster.main.arn
    role_arn = aws_iam_role.scheduler.arn

    ecs_parameters {
      task_definition_arn = aws_ecs_task_definition.restore_drill.arn_without_revision
      launch_type         = "FARGATE"
      task_count          = 1

      network_configuration {
        subnets          = aws_subnet.public[*].id
        security_groups  = [aws_security_group.service.id]
        assign_public_ip = true
      }
    }

    # No retry: a failed drill is a thing to read the log for, not to repeat.
    retry_policy {
      maximum_retry_attempts = 0
    }
  }
}

resource "aws_cloudwatch_event_rule" "drill_failed" {
  name        = "${local.name}-restore-drill-failed"
  description = "A restore drill exited non-zero -- the backups may not be restorable"

  event_pattern = jsonencode({
    source      = ["aws.ecs"]
    detail-type = ["ECS Task State Change"]
    detail = {
      clusterArn = [aws_ecs_cluster.main.arn]
      lastStatus = ["STOPPED"]
      group      = ["family:${aws_ecs_task_definition.restore_drill.family}"]
      containers = { exitCode = [{ "anything-but" = 0 }] }
    }
  })

  tags = local.tags
}

resource "aws_cloudwatch_event_target" "drill_failed" {
  rule      = aws_cloudwatch_event_rule.drill_failed.name
  target_id = "sns"
  arn       = aws_sns_topic.alerts.arn

  input_transformer {
    input_paths = {
      task = "$.detail.taskArn"
    }
    input_template = "\"birdtest restore drill FAILED: <task>. The nightly dumps may not be restorable -- see the CloudWatch log group ${aws_cloudwatch_log_group.main.name}, stream prefix restore-drill.\""
  }
}
