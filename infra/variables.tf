variable "project" {
  description = "Name prefix applied to every resource."
  type        = string
  default     = "birdtest"
}

variable "region" {
  description = "AWS region to deploy into."
  type        = string
  default     = "us-east-1"
}

variable "vpc_cidr" {
  type    = string
  default = "10.20.0.0/16"
}

variable "azs" {
  description = "Availability zones. Two are required for both the ALB and the RDS subnet group."
  type        = list(string)
  default     = ["us-east-1a", "us-east-1b"]
}

variable "backend_image" {
  description = "ECR image for the Axum backend container."
  type        = string
}

variable "frontend_image" {
  description = "ECR image for the Nginx container serving the SvelteKit build."
  type        = string
}

variable "db_instance_class" {
  type    = string
  default = "db.t4g.micro"
}

variable "db_max_wal_size_mb" {
  description = <<-EOT
    Postgres max_wal_size, in MB: how much WAL may accumulate before a
    checkpoint is forced. Sized so that one leave-generation merge (about
    1.3 GB of WAL for a full-size generation) fits inside a single checkpoint
    cycle; at Postgres's 1 GB default the same merge spans five and writes
    4.8 GB. It is also an upper bound on the disk WAL occupies, so keep it well
    under db_allocated_storage.
  EOT
  type        = number
  default     = 4096
}

variable "db_allocated_storage" {
  type    = number
  default = 20
}

variable "db_backup_retention_days" {
  description = "Length of the RDS point-in-time recovery window, in days."
  type        = number
  default     = 30
}

variable "db_multi_az" {
  description = <<-EOT
    Run the database as a Multi-AZ pair. This is availability, not backup: it
    survives an instance or AZ failure without a restore, and replicates a
    mistaken DELETE just as fast as a healthy write. It roughly doubles the
    instance cost, which is why it is opt-in.
  EOT
  type        = bool
  default     = false
}

variable "dr_region" {
  description = <<-EOT
    Second region that artifacts and backups are replicated into. Only has to
    differ from `region`; nothing is served from it.
  EOT
  type        = string
  default     = "us-west-2"
}

variable "backup_schedule" {
  description = "EventBridge Scheduler expression for the nightly logical dump."
  type        = string
  default     = "cron(0 3 * * ? *)"
}

variable "backup_restore_drill_schedule" {
  description = "Schedule for the automated restore drill (PLAN.md, \"Drills\")."
  type        = string
  default     = "cron(0 5 1 * ? *)"
}

variable "backup_retention_days" {
  description = "How long a nightly dump is kept before expiry."
  type        = number
  default     = 365
}

variable "backup_object_lock_days" {
  description = <<-EOT
    Governance-mode Object Lock retention on each backup object. An object
    cannot be deleted or overwritten within this window without the explicit
    s3:BypassGovernanceRetention permission, which is what makes the backups
    resistant to a compromised credential rather than merely versioned.
    Object Lock can only be enabled when the bucket is created.
  EOT
  type        = number
  default     = 30
}

variable "backup_image" {
  description = <<-EOT
    Image the backup task runs. The official Postgres image, pinned to the
    same major version as the RDS instance: pg_dump refuses to dump a server
    newer than itself, so this and `aws_db_instance.main.engine_version` move
    together.
  EOT
  type        = string
  default     = "public.ecr.aws/docker/library/postgres:16"
}

variable "backup_task_cpu" {
  type    = number
  default = 1024
}

variable "backup_task_memory" {
  type    = number
  default = 4096
}

variable "backup_ephemeral_storage_gib" {
  description = <<-EOT
    Scratch space for the dump before it is uploaded. Must exceed the
    compressed dump size; the results tables compress well, but
    position_analysis_moves is the table that will outgrow a default.
  EOT
  type        = number
  default     = 100
}

variable "backup_dump_jobs" {
  description = "pg_dump -j. Parallelism is what makes a large dump finish."
  type        = number
  default     = 4
}

variable "restore_drill_enabled" {
  description = <<-EOT
    Run the monthly restore drill. It restores the newest dump into a second
    database on the production instance, which needs storage headroom for a
    transient second copy of the corpus; turn it off if that becomes tight and
    run scripts/restore-drill.sh against a scratch instance instead.
  EOT
  type        = bool
  default     = true
}

variable "alert_email" {
  description = <<-EOT
    Where backup failure and staleness alarms are delivered. No default on
    purpose: an unmonitored backup is the failure mode this whole design
    exists to avoid, so `terraform apply` should refuse to run without it.
    SNS sends a subscription confirmation that has to be accepted once.
  EOT
  type        = string
}

variable "task_cpu" {
  type    = number
  default = 1024
}

variable "task_memory" {
  type    = number
  default = 2048
}

variable "desired_count" {
  description = <<-EOT
    Number of ECS tasks. Must be 1. Claiming is coordinated through Postgres,
    but three things are not: input-data imports run as an in-process task that
    a starting instance marks failed if it finds one running, rate limits are
    in-memory per process, and SSE subscribers only hear submissions made to
    their own instance. See PLAN.md's primary/secondary split before raising it.
  EOT
  type        = number
  default     = 1

  validation {
    condition     = var.desired_count <= 1
    error_message = "birdtest runs as a single instance; see the variable description."
  }
}

variable "acm_certificate_arn" {
  description = <<-EOT
    ACM certificate for the public hostname, in `region`. Required: the backend
    sets Secure cookies, which a browser will not keep over plain HTTP, so the
    site cannot be served without TLS.
  EOT
  type        = string
}

variable "min_magpie_version" {
  description = <<-EOT
    The oldest MAGPIE that may contribute (MIN_MAGPIE_VERSION). 0.1.0 is
    `birdtest-contribute`'s pre-release version: nothing is in production yet,
    so everything the protocol relies on is in 0.1.0, and a build reporting a
    lower version is offered nothing. Raise it whenever a MAGPIE release
    changes results. Must not exceed the version the backend image's own
    pinned MAGPIE reports (docker/Dockerfile's MAGPIE_COMMIT), or the backend
    refuses to start: it will not hand out hashes built by a MAGPIE its
    workers may not run.
  EOT
  type        = string
  default     = "0.1.0"
}

variable "github_token_parameter_arn" {
  description = "Optional SSM SecureString parameter ARN holding a GitHub token for input-data imports. Empty for none."
  type        = string
  default     = ""
}

variable "mail_from_address" {
  description = "Envelope From for SES. Must be within ses_domain."
  type        = string
  default     = "no-reply@birdtest.example"
}

variable "ses_domain" {
  description = "Domain to verify with SES for outbound account mail."
  type        = string
  default     = "birdtest.example"
}

variable "public_url" {
  description = "Base URL used to build confirmation and password-reset links."
  type        = string
  default     = "https://birdtest.example"
}
