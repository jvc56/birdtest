# The derived-file builder: a scheduled ECS task that drains `derived_data`.
#
# See README.md's "MAGPIE on the server". A wordmap and a rack info table are
# built on every contributor's own machine and are far too large to ship, so birdtest checks
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

  validation {
    # Not 256: it takes at most 2 GB, and a table build needs 4.
    condition     = contains([512, 1024, 2048, 4096, 8192, 16384], var.derived_builder_cpu)
    error_message = "derived_builder_cpu must be a Fargate CPU size that can hold 4 GB: 512, 1024, 2048, 4096, 8192 or 16384."
  }
}

variable "derived_builder_memory" {
  description = <<-EOT
    MAGPIE peaks at about 2.4 GB building a rack info table and 710 MB building
    a wordmap, and the builder hashes the output in 8 MB chunks rather than
    reading it in. 8 GB leaves room for a larger lexicon than any shipped
    today; below 4 GB a table build is killed rather than slow. It must also be
    a Fargate size for derived_builder_cpu: 8 GB is the least 4 vCPU takes.
  EOT
  type        = number
  default     = 8192

  validation {
    # Fargate's CPU and memory pairs: refused otherwise only by
    # RegisterTaskDefinition, part-way through an apply.
    condition = (
      (var.derived_builder_cpu == 512 && var.derived_builder_memory >= 1024 && var.derived_builder_memory <= 4096 && var.derived_builder_memory % 1024 == 0) ||
      (var.derived_builder_cpu == 1024 && var.derived_builder_memory >= 2048 && var.derived_builder_memory <= 8192 && var.derived_builder_memory % 1024 == 0) ||
      (var.derived_builder_cpu == 2048 && var.derived_builder_memory >= 4096 && var.derived_builder_memory <= 16384 && var.derived_builder_memory % 1024 == 0) ||
      (var.derived_builder_cpu == 4096 && var.derived_builder_memory >= 8192 && var.derived_builder_memory <= 30720 && var.derived_builder_memory % 1024 == 0) ||
      (var.derived_builder_cpu == 8192 && var.derived_builder_memory >= 16384 && var.derived_builder_memory <= 61440 && var.derived_builder_memory % 4096 == 0) ||
      (var.derived_builder_cpu == 16384 && var.derived_builder_memory >= 32768 && var.derived_builder_memory <= 122880 && var.derived_builder_memory % 8192 == 0)
    )
    error_message = "derived_builder_memory is not a Fargate memory size for derived_builder_cpu's CPU (512 CPU takes 1024-4096; 1024, 2048-8192; 2048, 4096-16384; 4096, 8192-30720, all in 1024 steps; 8192, 16384-61440 in 4096 steps; 16384, 32768-122880 in 8192 steps)."
  }

  validation {
    condition     = var.derived_builder_memory >= 4096
    error_message = "derived_builder_memory must be at least 4096 MiB: MAGPIE peaks at about 2.4 GB building a rack info table, and below 4 GB the build is killed."
  }
}

variable "derived_builder_ephemeral_storage_gib" {
  description = <<-EOT
    Scratch space for one conversion at a time: a 1.9 GB rack info table plus
    the 179 MB wordmap it is built from plus their inputs. The 20 GiB Fargate
    default would do (21 is the least that can be set); this is explicit so
    that a lexicon twice CSW24's size is a number to change rather than a task
    that dies mid-build.
  EOT
  type        = number
  default     = 30

  validation {
    condition     = var.derived_builder_ephemeral_storage_gib >= 21 && var.derived_builder_ephemeral_storage_gib <= 200
    error_message = "Fargate ephemeral storage is 21 to 200 GiB."
  }
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
# away, so there is nothing to put. It may delete input objects (`inputs/*`),
# and does so only for one whose bytes are not the ones imported under its
# content address, so that a re-import uploads it again (an import skips an
# object that exists); inputs are re-importable and the bucket versioned, so
# a delete leaves the bytes as a noncurrent version for 90 days
# (`expire-noncurrent-versions`, s3.tf). Worth stating rather
# than reusing the web task's role, which can also write.
data "aws_iam_policy_document" "derived_builder_task" {
  statement {
    actions   = ["s3:GetObject"]
    resources = ["${aws_s3_bucket.artifacts.arn}/*"]
  }
  statement {
    actions   = ["s3:DeleteObject"]
    resources = ["${aws_s3_bucket.artifacts.arn}/inputs/*"]
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
        # Where the throwaway data directories go: a directory the image makes
        # for them. On Fargate the ephemeral storage above backs the whole
        # writable layer, /tmp included, so this names the place rather than a
        # different disk.
        { name = "MAGPIE_SCRATCH_DIR", value = "/scratch" },
        # Threads for a conversion. Matched to the vCPUs above: the whole
        # reason this task exists is that it can give MAGPIE the cores the web
        # task cannot.
        # Whole vCPUs, at least one: 512 CPU units made "0.5".
        { name = "MAGPIE_THREADS", value = tostring(max(1, floor(var.derived_builder_cpu / 1024))) },
        { name = "MIN_MAGPIE_VERSION", value = var.min_magpie_version }
      ]
      secrets = [
        { name = "DATABASE_URL", valueFrom = local.ssm_database_url_arn },
        # Config::from_env requires it, and this process never mints a session.
        { name = "SESSION_SIGNING_KEY", valueFrom = local.ssm_session_signing_key_arn }
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
    actions = ["ecs:RunTask"]
    # The schedule names the family without a revision; RunTask's resource is
    # always a revisioned task-definition ARN (the service authorization
    # reference), which `:*` matches. The bare family is listed as well, since
    # no AWS page says outright which one a revisionless schedule is checked
    # against, and it widens nothing.
    resources = [
      aws_ecs_task_definition.derived_builder.arn_without_revision,
      "${aws_ecs_task_definition.derived_builder.arn_without_revision}:*",
    ]
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
    # `pending` with its attempt counter raised and a wait before its next
    # attempt (5, then 15 minutes), and the first scheduled run after the
    # wait takes it again; after three it is `failed` until an admin retries
    # it. Retrying the task instead would start a second builder against the
    # same queue for no gain.
    retry_policy {
      maximum_retry_attempts = 0
    }
  }
}
