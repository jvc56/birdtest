# ---------------------------------------------------------------------------
# Operator access to the database
# ---------------------------------------------------------------------------
# The database has no public address and admits only the service's security
# group, so nothing on an operator's machine can reach it: no psql from a
# laptop, no bastion. Every SQL step of RUNBOOK.md -- making the first admin,
# a selective restore, repairing counters -- runs from this task instead: the
# postgres image (a psql and pg_restore matching the server), DATABASE_URL from
# SSM, the service's network, read access to the backups, and ECS Exec for an
# interactive shell. `scripts/prod-sql.sh` runs one batch of SQL through it;
# `scripts/prod-shell.sh` opens a shell in it.
#
# Nothing runs it on a schedule. It exists only while an operator has one open.

resource "aws_iam_role" "ops_task" {
  name               = "${local.name}-ops"
  assume_role_policy = data.aws_iam_policy_document.task_assume.json
  tags               = local.tags
}

data "aws_iam_policy_document" "ops_task" {
  # A selective restore starts from a nightly dump. Read only, like the drill.
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
  # ECS Exec.
  statement {
    actions = [
      "ssmmessages:CreateControlChannel",
      "ssmmessages:CreateDataChannel",
      "ssmmessages:OpenControlChannel",
      "ssmmessages:OpenDataChannel",
    ]
    resources = ["*"]
  }
}

resource "aws_iam_role_policy" "ops_task" {
  role   = aws_iam_role.ops_task.id
  policy = data.aws_iam_policy_document.ops_task.json
}

resource "aws_ecs_task_definition" "ops" {
  family                   = "${local.name}-ops"
  requires_compatibilities = ["FARGATE"]
  network_mode             = "awsvpc"
  cpu                      = var.backup_task_cpu
  memory                   = var.backup_task_memory
  execution_role_arn       = aws_iam_role.execution.arn
  task_role_arn            = aws_iam_role.ops_task.arn

  # Room for a nightly dump and a scratch restore of it (RUNBOOK.md §2.1).
  ephemeral_storage {
    size_in_gib = var.restore_ephemeral_storage_gib
  }

  container_definitions = jsonencode([
    {
      name      = "ops"
      image     = var.backup_image
      essential = true
      # Not the postgres entrypoint, which would start a server. Each use
      # overrides the command: psql for prod-sql.sh, `sleep` for a shell.
      entryPoint = ["/bin/bash", "-c"]
      command    = ["sleep 3600"]
      environment = [
        { name = "BACKUP_BUCKET", value = aws_s3_bucket.backups.bucket },
        { name = "AWS_REGION", value = var.region },
        { name = "AWS_DEFAULT_REGION", value = var.region },
      ]
      secrets = [
        { name = "DATABASE_URL", valueFrom = aws_ssm_parameter.database_url.arn }
      ]
      logConfiguration = {
        logDriver = "awslogs"
        options = {
          "awslogs-group"         = aws_cloudwatch_log_group.main.name
          "awslogs-region"        = var.region
          "awslogs-stream-prefix" = "ops"
        }
      }
    }
  ])

  tags = local.tags
}
