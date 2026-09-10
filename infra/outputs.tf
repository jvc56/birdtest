output "alb_dns_name" {
  description = "Public hostname for the service. Point the site's DNS record here."
  value       = aws_lb.main.dns_name
}

output "artifacts_bucket" {
  value = aws_s3_bucket.artifacts.bucket
}

output "database_endpoint" {
  description = "RDS endpoint. Reachable only from inside the VPC."
  value       = aws_db_instance.main.endpoint
}

output "ssm_parameter_names" {
  description = "Parameters whose values must be set out of band before the first deploy."
  value = [
    aws_ssm_parameter.database_url.name,
    aws_ssm_parameter.session_signing_key.name,
  ]
}

output "ses_dkim_tokens" {
  description = "Add these as CNAME records to finish SES domain verification."
  value       = aws_sesv2_email_identity.domain.dkim_signing_attributes[0].tokens
}

output "backups_bucket" {
  description = "Where nightly logical dumps land. Restores read from here."
  value       = aws_s3_bucket.backups.bucket
}

output "backups_dr_bucket" {
  description = "Cross-region replica of the above. What a region loss restores from."
  value       = aws_s3_bucket.backups_dr.bucket
}

output "artifacts_dr_bucket" {
  value = aws_s3_bucket.artifacts_dr.bucket
}

output "backup_task_definition" {
  description = "Run a backup on demand: aws ecs run-task --task-definition <this>."
  value       = aws_ecs_task_definition.backup.family
}
