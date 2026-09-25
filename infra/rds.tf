resource "aws_db_subnet_group" "main" {
  name       = "${local.name}-db"
  subnet_ids = aws_subnet.private[*].id
  tags       = local.tags
}

resource "aws_security_group" "db" {
  name        = "${local.name}-db"
  description = "Postgres, reachable only from the ECS tasks"
  vpc_id      = aws_vpc.main.id

  ingress {
    from_port       = 5432
    to_port         = 5432
    protocol        = "tcp"
    security_groups = [aws_security_group.service.id]
  }

  tags = local.tags
}

# Two WAL settings, both about leave generation (PLAN.md, "Leave Generation",
# "What a merge costs"). A merge folds half an hour of accepted results into a
# generation's 3.2 million per-rack rows in one pass, and touches most pages of
# the table and its indexes doing it. The first change to a page after a
# checkpoint is logged as a whole 8 kB page, so what a merge writes is mostly
# page images -- and how many times over depends on how many checkpoints it
# spans, which WAL volume triggers. Measured on a full-size generation, one
# merge of thirty 200,000-rack results:
#
#   max_wal_size 1 GB (Postgres's default)   4.8 GB of WAL, five checkpoints
#   max_wal_size large enough to span it     1.3 GB
#   ... and wal_compression on               1.0 GB
#
# That WAL is kept for the whole point-in-time recovery window below, so the
# difference is storage as well as I/O. Both are dynamic; neither needs a
# reboot.
resource "aws_db_parameter_group" "main" {
  name_prefix = "${local.name}-pg16-"
  family      = "postgres16"
  description = "birdtest: WAL settings sized for leave-generation merges"

  parameter {
    name  = "wal_compression"
    value = "on"
  }

  parameter {
    name  = "max_wal_size" # in MB
    value = tostring(var.db_max_wal_size_mb)
  }

  lifecycle {
    create_before_destroy = true
  }

  tags = local.tags
}

resource "aws_db_instance" "main" {
  identifier     = local.name
  engine         = "postgres"
  engine_version = "16"

  parameter_group_name = aws_db_parameter_group.main.name

  instance_class        = var.db_instance_class
  allocated_storage     = var.db_allocated_storage
  max_allocated_storage = var.db_allocated_storage * 5
  storage_encrypted     = true

  db_name  = var.project
  username = var.project
  # The master password is set by hand, not managed by RDS: RDS-managed
  # passwords rotate every 7 days, and the tasks read a fixed DATABASE_URL from
  # SSM, so the first rotation would lock the service out. The instance is
  # created with this placeholder, which the first deploy
  # replaces immediately (README.md, "Deploying"); ignore_changes keeps
  # Terraform from reverting it. The real password lives only in the
  # DATABASE_URL SSM parameter declared in ssm.tf. The instance is reachable
  # only from the ECS tasks' security group in the meantime. (Setting
  # `password` at all is what turns RDS-managed passwords off; the provider
  # refuses `manage_master_user_password` alongside it, even as false.)
  password = "placeholder-replace-on-first-deploy"

  db_subnet_group_name   = aws_db_subnet_group.main.name
  vpc_security_group_ids = [aws_security_group.db.id]
  publicly_accessible    = false

  # Thirty days of point-in-time recovery. See PLAN.md's "Backups and Restore"; this window
  # is the primary mechanism for infrastructure failure and for a bad
  # migration, and the nightly logical dump is the secondary.
  backup_retention_period = var.db_backup_retention_days
  backup_window           = "07:00-08:00" # UTC; deliberately clear of the 03:00 dump
  copy_tags_to_snapshot   = true
  # Availability, not backup: it removes the most likely reason to ever perform
  # a restore, and doubles the instance cost. Off by default for that reason.
  multi_az                  = var.db_multi_az
  skip_final_snapshot       = false
  final_snapshot_identifier = "${local.name}-final"
  deletion_protection       = true

  lifecycle {
    ignore_changes = [password]
  }

  tags = local.tags
}

# --- Database alarms -------------------------------------------------------
# Storage autoscales only up to max_allocated_storage; past it every write
# fails, the submissions and claims first. Nothing warned before that. Alarmed
# at a fifth of the ceiling free, which at a claim's ~420 bytes with its
# indexes is weeks of headroom, not hours.
resource "aws_cloudwatch_metric_alarm" "db_storage_low" {
  alarm_name          = "${local.name}-db-storage-low"
  alarm_description   = "birdtest's database is within a fifth of its storage ceiling"
  namespace           = "AWS/RDS"
  metric_name         = "FreeStorageSpace"
  dimensions          = { DBInstanceIdentifier = aws_db_instance.main.identifier }
  statistic           = "Minimum"
  period              = 300
  evaluation_periods  = 3
  threshold           = aws_db_instance.main.max_allocated_storage * 1073741824 / 5
  comparison_operator = "LessThanThreshold"
  alarm_actions       = [aws_sns_topic.alerts.arn]
  ok_actions          = [aws_sns_topic.alerts.arn]
  tags                = local.tags
}

# A burstable instance (the default db.t4g.micro) that spends its CPU credits
# is throttled to its baseline -- a tenth of a vCPU -- and claims and
# submissions slow with everything else. Only burstable classes report it.
resource "aws_cloudwatch_metric_alarm" "db_cpu_credits_low" {
  count               = startswith(var.db_instance_class, "db.t") ? 1 : 0
  alarm_name          = "${local.name}-db-cpu-credits-low"
  alarm_description   = "birdtest's database is running out of CPU credits"
  namespace           = "AWS/RDS"
  metric_name         = "CPUCreditBalance"
  dimensions          = { DBInstanceIdentifier = aws_db_instance.main.identifier }
  statistic           = "Minimum"
  period              = 300
  evaluation_periods  = 3
  threshold           = 10
  comparison_operator = "LessThanThreshold"
  alarm_actions       = [aws_sns_topic.alerts.arn]
  ok_actions          = [aws_sns_topic.alerts.arn]
  tags                = local.tags
}
