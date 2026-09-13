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

resource "aws_db_instance" "main" {
  identifier     = local.name
  engine         = "postgres"
  engine_version = "16"

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
