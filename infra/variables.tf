variable "project" {
  description = "Name prefix applied to every resource."
  type        = string
  default     = "birdtest"
}

variable "name_suffix" {
  description = <<-EOT
    Appended to the resource-name prefix. Empty for the one production stack.
    A region-loss rebuild (RUNBOOK.md §5) applies a second copy of this stack
    into the same account, where every bucket name (global) and IAM role name
    (account-wide) of the first is still taken -- the lost region's buckets
    and the DR replicas alike -- so it sets one, such as "-dr".
  EOT
  type        = string
  default     = ""

  validation {
    condition     = can(regex("^(-[a-z0-9]+)?$", var.name_suffix))
    error_message = "name_suffix is empty or a hyphen followed by lowercase letters and digits, such as \"-dr\"."
  }
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
  description = <<-EOT
    Availability zones, at least two (the ALB and the RDS subnet group both need
    two). Unset, the region's first two zones that need no opt-in: the old
    default named us-east-1's, so a stack in any other region that set
    `region` without `azs` failed at its first subnet -- as did RUNBOOK §5's
    "<region>a" and "<region>b" in a region whose accounts get other letters.
  EOT
  type        = list(string)
  default     = null

  validation {
    condition     = var.azs == null || length(coalesce(var.azs, [])) >= 2
    error_message = "azs needs at least two availability zones."
  }
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
  description = <<-EOT
    The database's storage in GiB. Autoscaling grows it up to five times this;
    a value below the current (autoscaled) size is ignored rather than shrunk,
    and a value above it grows the volume. RUNBOOK.md §5 sets it for the DR
    copy from the size of the dump being restored.
  EOT
  type        = number
  default     = 20

  validation {
    # RDS refuses a Postgres instance under 20 GiB -- after the rest of the
    # stack has been built, so a small computed size (RUNBOOK §5, for a young
    # database) failed the apply half-way. Refused at plan instead.
    condition     = var.db_allocated_storage >= 20
    error_message = "db_allocated_storage must be at least 20 (GiB): RDS refuses less."
  }
}

variable "scheduled_tasks_enabled" {
  description = <<-EOT
    Whether the scheduled tasks run: the derived-data builder, the nightly
    backup and the restore drill. RUNBOOK §5 turns them off while the DR copy's
    database is being restored -- the builder would fail rows whose inputs are
    not synced yet, and a backup would dump the half-restored database as the
    newest -- and back on with the stack.
  EOT
  type        = bool
  default     = true
}

variable "db_apply_immediately" {
  description = <<-EOT
    Apply database modifications at the apply rather than in the next weekly
    maintenance window: instance class, Multi-AZ, storage, backup window and
    retention, CA certificate, parameter-group association. A class change is
    a restart -- a few minutes with no database -- so with this on, an apply
    that changes the class takes the site down at that moment. On an existing
    stack created without it, check `PendingModifiedValues` first: queued
    changes are applied by the first apply that turns it on.
  EOT
  type        = bool
  default     = true
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
    Scratch space for the nightly dump before it is uploaded. Must exceed the
    compressed dump size; the results tables compress well, but
    position_analysis_moves is the table that will outgrow a default.
  EOT
  type        = number
  default     = 100
}

variable "restore_ephemeral_storage_gib" {
  description = <<-EOT
    Disk for the monthly restore drill and the ops task (RUNBOOK.md §2.1), each
    of which holds a downloaded dump *and* a full restored copy of the database
    side by side, with WAL. Fargate's ceiling is 200 GiB, which covers a
    database a little over 150 GiB; the drill refuses to start, naming this
    variable, when the manifest says the database will not fit.
  EOT
  type        = number
  default     = 200

  validation {
    condition     = var.restore_ephemeral_storage_gib >= 21 && var.restore_ephemeral_storage_gib <= 200
    error_message = "Fargate ephemeral storage is 21 to 200 GiB."
  }
}

variable "backup_dump_jobs" {
  description = "pg_dump -j. Parallelism is what makes a large dump finish."
  type        = number
  default     = 4
}

variable "restore_drill_enabled" {
  description = <<-EOT
    Run the monthly restore drill. It restores the newest dump into a Postgres
    of its own, inside the drill task, on restore_ephemeral_storage_gib of disk;
    it never touches the production instance.
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

  # A typo leaves the subscription pending forever, and the alarms go nowhere.
  validation {
    condition     = can(regex("^[^@\\s<>,;]+@[^@\\s<>,;]+\\.[^@\\s<>,;]+$", var.alert_email))
    error_message = "alert_email must be one plain email address."
  }
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
    The oldest MAGPIE that may contribute (MIN_MAGPIE_VERSION). 0.1.1 is the
    `birdtest-contribute` version the backend image pins, and a build reporting
    a lower version is offered nothing. Raise it whenever a MAGPIE release
    changes results: it is the only way to keep a build known to compute
    something wrong off the fleet. Must not exceed the version the backend image's own
    pinned MAGPIE reports (docker/Dockerfile's MAGPIE_COMMIT), or the backend
    refuses to start: it will not hand out hashes built by a MAGPIE its
    workers may not run.
  EOT
  type        = string
  default     = "0.1.1"
}

variable "github_token_parameter_arn" {
  description = "Optional SSM SecureString parameter ARN holding a GitHub token for input-data imports. Empty for none."
  type        = string
  default     = ""
}

# The next three had placeholder defaults under birdtest.example. Forgotten,
# the apply succeeded, every confirmation and reset mail linked to a domain
# nobody owns, and SES was asked to verify it. Required now, and a leftover
# placeholder is refused.

variable "mail_from_address" {
  description = "Envelope From for SES. Must be within ses_domain."
  type        = string

  validation {
    condition     = can(regex("^[^@\\s]+@[^@\\s]+$", var.mail_from_address)) && !endswith(var.mail_from_address, ".example")
    error_message = "mail_from_address must be a real address within ses_domain."
  }
}

variable "ses_domain" {
  description = <<-EOT
    Domain to verify with SES for outbound account mail. A new AWS account's
    SES starts in the sandbox, where it sends only to verified addresses: until
    production access is granted, every confirmation and reset mail to anyone
    else fails. See README.md, "Deploying".
  EOT
  type        = string

  validation {
    condition     = length(var.ses_domain) > 0 && !can(regex("[/:@\\s]", var.ses_domain)) && !endswith(var.ses_domain, ".example")
    error_message = "ses_domain must be a bare domain you own, such as birdtest.org."
  }
}

variable "public_url" {
  description = "Base URL used to build confirmation and password-reset links: the site's https:// origin, no trailing slash."
  type        = string

  validation {
    condition     = can(regex("^https://[^/]+$", var.public_url)) && !can(regex("\\.example$", var.public_url))
    error_message = "public_url must be the site's https:// origin (no path or trailing slash), and not the birdtest.example placeholder."
  }
}
