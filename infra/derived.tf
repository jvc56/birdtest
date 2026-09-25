# The derived-file builder: a scheduled ECS task that drains `derived_data`.
#
# See MAGPIE_DEPENDENCY.md. A wordmap and a rack info table are built on every
# contributor's own machine and are far too large to ship, so birdtest checks
# them by building its own reference copy with a pinned MAGPIE and publishing
# the hash. This is where those builds run.
#
# Not in the web task. A rack info table build peaks at about 2.4 GB of memory,
# writes a 1.9 GB file, and takes one to three minutes; the web task has 1 vCPU
# and 2 GB (`task_cpu`, `task_memory`) and does not fit it. A wordmap's 710 MB
# peak is a third of that task's memory alongside everything else it is doing.
#
# Not triggered on demand, either. The web task could call ecs:RunTask the
# moment an admin creates a job, which would start a build seconds earlier and
# would cost an AWS control-plane call on the job-creation path, permission for
# the web task to run tasks and pass roles, and a retry story for a call that
# can be throttled. A build takes minutes; a poll every few minutes is a small
# fraction of that, and the admin UI shows the queue, so the wait is visible.
#
# The image is the backend image with a different entrypoint (`--target
# derived-builder`), which is what keeps the builder's MAGPIE and the server's
# the same binary. Two images would be two builders, and a hash recorded
# against a builder that did not produce it is exactly what this design exists
# to prevent.

variable "derived_builder_image" {
  description = <<-EOT
    Image the derived-file builder runs: the backend image built with
    `--target derived-builder`. It must carry the same MAGPIE as
    `backend_image`, because the builder version recorded beside every hash
    comes from the binary that produced it -- the two move together.
  EOT
  type        = string

  validation {
    condition     = length(trimspace(var.derived_builder_image)) > 0
    error_message = "derived_builder_image is the backend image built with --target derived-builder, at the same tag as backend_image."
  }
}

variable "derived_builder_cpu" {
  description = <<-EOT
    A rack info table build scales close to linearly with cores: 59 seconds on
    8 threads against about 170 on one, for CSW24. Four is the point past
    which the wait stops being what an admin notices.
  EOT
  type        = number
  default     = 4096
}

variable "derived_builder_memory" {
  description = <<-EOT
    MAGPIE peaks at about 2.4 GB building a rack info table and 710 MB building
    a wordmap, and the builder hashes the output in 8 MB chunks rather than
    reading it in. 8 GB leaves room for a larger lexicon than any shipped
    today; below 4 GB a table build is killed rather than slow.
  EOT
  type        = number
  default     = 8192
}

variable "derived_builder_ephemeral_storage_gib" {
  description = <<-EOT
    Scratch space for one conversion at a time: a 1.9 GB rack info table plus
    the 179 MB wordmap it is built from plus their inputs. The 20 GB Fargate
    default would do; this is explicit so that a lexicon twice CSW24's size is
    a number to change rather than a task that dies mid-build.
  EOT
  type        = number
  default     = 30
}

variable "derived_builder_schedule" {
  description = <<-EOT
    How often to drain the queue. Most runs find it empty and exit in seconds,
    which is the intended steady state; the interval only bounds how long an
    admin waits after creating a job that needs a table.
  EOT
  type        = string
  default     = "rate(5 minutes)"
}

# --- Roles -----------------------------------------------------------------

resource "aws_iam_role" "derived_builder_task" {
  name               = "${local.name}-derived-builder"
  assume_role_policy = data.aws_iam_policy_document.task_assume.json
  tags               = local.tags
}

# The builder reads the lexicon and leaves bytes the import stored, and writes
# nothing to the bucket: the derived files themselves are hashed and thrown
# away, so there is nothing to put. Read-only on the artifact bucket is
# therefore the whole of it, and is worth stating rather than reusing the web
# task's role, which can also write.
data "aws_iam_policy_document" "derived_builder_task" {
  statement {
    actions   = ["s3:GetObject"]
    resources = ["${aws_s3_bucket.artifacts.arn}/*"]
  }
  statement {
    actions   = ["s3:ListBucket"]
    resources = [aws_s3_bucket.artifacts.arn]
  }
}

resource "aws_iam_role_policy" "derived_builder_task" {
  role   = aws_iam_role.derived_builder_task.id
  policy = data.aws_iam_policy_document.derived_builder_task.json
}

# --- Task definition -------------------------------------------------------

resource "aws_ecs_task_definition" "derived_builder" {
  family                   = "${local.name}-derived-builder"
  requires_compatibilities = ["FARGATE"]
  network_mode             = "awsvpc"
  cpu                      = var.derived_builder_cpu
  memory                   = var.derived_builder_memory
  execution_role_arn       = aws_iam_role.execution.arn
  task_role_arn            = aws_iam_role.derived_builder_task.arn

  ephemeral_storage {
    size_in_gib = var.derived_builder_ephemeral_storage_gib
  }

  container_definitions = jsonencode([
    {
      name      = "derived-builder"
      image     = var.derived_builder_image
      essential = true
      # Stated rather than left to the image's CMD. Given the backend image by
      # mistake -- the same repository, a different target -- the CMD is the
      # web server, which never exits: a 4-vCPU task started every five
      # minutes, each running the startup reapers that fail the live server's
      # exports and imports. Stated, the wrong image has no `build-derived`
      # and the task fails at once, which the scheduler's failures show.
      command = ["build-derived"]
      environment = [
        { name = "S3_BUCKET", value = aws_s3_bucket.artifacts.bucket },
        { name = "AWS_REGION", value = var.region },
        { name = "RUST_LOG", value = "birdtest=info" },
        # Where the throwaway data directories go. The container's default temp
        # directory is not the ephemeral volume mounted above, and a 1.9 GB
        # table written to the wrong one fills the layer instead.
        { name = "MAGPIE_SCRATCH_DIR", value = "/scratch" },
        # Threads for a conversion. Matched to the vCPUs above: the whole
        # reason this task exists is that it can give MAGPIE the cores the web
        # task cannot.
        # Whole vCPUs, at least one: 512 CPU units made "0.5".
        { name = "MAGPIE_THREADS", value = tostring(max(1, floor(var.derived_builder_cpu / 1024))) },
        { name = "MIN_MAGPIE_VERSION", value = var.min_magpie_version }
      ]
      secrets = [
        { name = "DATABASE_URL", valueFrom = aws_ssm_parameter.database_url.arn },
        # Config::from_env requires it, and this process never mints a session.
        { name = "SESSION_SIGNING_KEY", valueFrom = aws_ssm_parameter.session_signing_key.arn }
      ]
      logConfiguration = {
        logDriver = "awslogs"
        options = {
          "awslogs-group"         = aws_cloudwatch_log_group.main.name
          "awslogs-region"        = var.region
          "awslogs-stream-prefix" = "derived-builder"
        }
      }
    }
  ])

  tags = local.tags
}

# --- Schedule --------------------------------------------------------------

resource "aws_iam_role" "derived_builder_scheduler" {
  name               = "${local.name}-derived-builder-scheduler"
  assume_role_policy = data.aws_iam_policy_document.scheduler_assume.json
  tags               = local.tags
}

data "aws_iam_policy_document" "derived_builder_scheduler" {
  statement {
    actions   = ["ecs:RunTask"]
    resources = ["${aws_ecs_task_definition.derived_builder.arn_without_revision}:*"]
    condition {
      test     = "ArnLike"
      variable = "ecs:cluster"
      values   = [aws_ecs_cluster.main.arn]
    }
  }
  statement {
    actions   = ["iam:PassRole"]
    resources = [aws_iam_role.execution.arn, aws_iam_role.derived_builder_task.arn]
    condition {
      test     = "StringEquals"
      variable = "iam:PassedToService"
      values   = ["ecs-tasks.amazonaws.com"]
    }
  }
}

resource "aws_iam_role_policy" "derived_builder_scheduler" {
  role   = aws_iam_role.derived_builder_scheduler.id
  policy = data.aws_iam_policy_document.derived_builder_scheduler.json
}

resource "aws_scheduler_schedule" "derived_builder" {
  name                         = "${local.name}-derived-builder"
  description                  = "Drain the wordmap and rack info table build queue"
  schedule_expression          = var.derived_builder_schedule
  schedule_expression_timezone = "UTC"
  state                        = var.scheduled_tasks_enabled ? "ENABLED" : "DISABLED"

  flexible_time_window {
    mode = "OFF"
  }

  target {
    arn      = aws_ecs_cluster.main.arn
    role_arn = aws_iam_role.derived_builder_scheduler.arn

    ecs_parameters {
      task_definition_arn = aws_ecs_task_definition.derived_builder.arn_without_revision
      launch_type         = "FARGATE"
      task_count          = 1

      network_configuration {
        subnets          = aws_subnet.public[*].id
        security_groups  = [aws_security_group.service.id]
        assign_public_ip = true
      }
    }

    # No retries. The queue is the retry: a row whose build failed is left
    # `pending` with its attempt counter raised, and the next scheduled run
    # takes it again. Retrying the task instead would start a second builder
    # against the same queue for no gain.
    retry_policy {
      maximum_retry_attempts = 0
    }
  }
}
