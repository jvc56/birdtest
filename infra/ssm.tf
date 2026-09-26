# Secret-valued configuration. The two parameters are created and set out of
# band (README.md, "Deploying"); Terraform only names them. Managed here as
# resources with `ignore_changes = [value]`, as they were, the provider still
# read each value -- decrypted -- into state on every refresh, so the database
# password and the session signing key sat in plain text in a state file
# README said held no secret (thirty-second audit, pass 17). A data source
# would do the same. So they are referenced by an ARN built from their names.

locals {
  ssm_prefix                   = "/${var.project}"
  ssm_database_url_name        = "${local.ssm_prefix}/DATABASE_URL"
  ssm_session_signing_key_name = "${local.ssm_prefix}/SESSION_SIGNING_KEY"
  ssm_parameter_arn_prefix     = "arn:${data.aws_partition.current.partition}:ssm:${var.region}:${data.aws_caller_identity.current.account_id}:parameter"
  ssm_database_url_arn         = "${local.ssm_parameter_arn_prefix}${local.ssm_database_url_name}"
  ssm_session_signing_key_arn  = "${local.ssm_parameter_arn_prefix}${local.ssm_session_signing_key_name}"
}

data "aws_partition" "current" {}

# A stack applied before pass 17 holds both as resources: they leave the state
# here without being destroyed. Its old state files still hold both values,
# so rotate them (README.md, "Deploying").
removed {
  from = aws_ssm_parameter.database_url
  lifecycle {
    destroy = false
  }
}

removed {
  from = aws_ssm_parameter.session_signing_key
  lifecycle {
    destroy = false
  }
}
