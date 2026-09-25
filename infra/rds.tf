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
# fails, the submissions and claims first. CloudWatch has no allocated-storage
# metric, and FreeStorageSpace is measured against the *current* allocation --
# a threshold on it either fires from the first apply (autoscaling keeps free
# space near a tenth) or never. RDS's own events say it instead: "low storage"
# is the instance running short, and autoscaling that cannot go further.
resource "aws_db_event_subscription" "db_storage" {
  name        = "${local.name}-db-storage"
  sns_topic   = aws_sns_topic.alerts.arn
  source_type = "db-instance"
  source_ids  = [aws_db_instance.main.identifier]
  # "failure" too: an instance that has failed is the other thing nobody
  # would otherwise hear about until the site was down.
  event_categories = ["low storage", "failure"]
  tags             = local.tags

  # RDS checks it may publish when the subscription is made; the topic policy
  # that lets it is changed in the same apply.
  depends_on = [aws_sns_topic_policy.alerts]
}

# Sustained load. The default class is burstable and runs in unlimited mode:
# out of credits it is billed for the surplus rather than throttled, so the
# credit balance says little (and reads empty from launch). Busy for fifteen
# minutes is the signal either way -- a stats payload rebuilt too often, a
# sweep grown too large.
resource "aws_cloudwatch_metric_alarm" "db_cpu_high" {
  alarm_name          = "${local.name}-db-cpu-high"
  alarm_description   = "birdtest's database has been over 80% CPU for fifteen minutes"
  namespace           = "AWS/RDS"
  metric_name         = "CPUUtilization"
  dimensions          = { DBInstanceIdentifier = aws_db_instance.main.identifier }
  statistic           = "Average"
  period              = 300
  evaluation_periods  = 3
  threshold           = 80
  comparison_operator = "GreaterThanOrEqualToThreshold"
  alarm_actions       = [aws_sns_topic.alerts.arn]
  ok_actions          = [aws_sns_topic.alerts.arn]
  tags                = local.tags
}
