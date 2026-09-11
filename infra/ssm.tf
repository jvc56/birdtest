# Secret-valued configuration. Only the parameter *names* are managed here —
# the values are set manually (or by rotation) so they never land in state or a
# plan diff. `ignore_changes = [value]` is what keeps Terraform from reverting
# a value it should not know.

locals {
  ssm_prefix = "/${var.project}"
}

resource "aws_ssm_parameter" "database_url" {
  name        = "${local.ssm_prefix}/DATABASE_URL"
  description = "postgres://user:password@host:5432/birdtest"
  type        = "SecureString"
  value       = "set-me"

  lifecycle {
    ignore_changes = [value]
  }
  tags = local.tags
}

resource "aws_ssm_parameter" "session_signing_key" {
  name        = "${local.ssm_prefix}/SESSION_SIGNING_KEY"
  description = "32 bytes, hex-encoded, for Paseto v4.local session tokens"
  type        = "SecureString"
  value       = "set-me"

  lifecycle {
    ignore_changes = [value]
  }
  tags = local.tags
}
