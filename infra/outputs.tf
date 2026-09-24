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

# What a one-off task inside the VPC needs: `scripts/prod-sql.sh` runs SQL
# against the database this way, and RUNBOOK.md's restores reach it the same
# way. The database is not publicly accessible and nothing else can reach it.
output "cluster_name" {
  value = aws_ecs_cluster.main.name
}

output "service_subnet_ids" {
  value = aws_subnet.public[*].id
}

output "service_security_group_id" {
  description = "The only security group the database accepts connections from."
  value       = aws_security_group.service.id
}

output "db_security_group_id" {
  description = "The database's own security group, for an instance restored beside it (RUNBOOK.md §1)."
  value       = aws_security_group.db.id
}

output "log_group_name" {
  value = aws_cloudwatch_log_group.main.name
}

output "ops_task_definition" {
  description = "The task scripts/prod-sql.sh and scripts/prod-shell.sh run: psql inside the VPC."
  value       = aws_ecs_task_definition.ops.family
}
