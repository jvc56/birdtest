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

variable "db_allocated_storage" {
  type    = number
  default = 20
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
  description = "Number of ECS tasks. Task scheduling is coordinated through Postgres, so more than one is safe; see the primary/secondary split under Possible Future Improvements before scaling far."
  type        = number
  default     = 1
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
