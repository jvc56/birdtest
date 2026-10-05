# S-TF-1, S-TF-2 (TESTING.md): every variable validation refuses a wrong value
# and lets the right one through. `terraform validate` evaluates no condition,
# so a validation was otherwise first run by a real plan -- one with a broken
# regex refused a correct first apply. S-TF-3, at the end: the deploy-failed
# alert. These plan against mock providers: no credentials, no account,
# nothing created. `terraform test` from infra/.
#
# Each refusal run changes one variable from the good set below and names the
# variables it expects refused; a run that plans instead, or fails anywhere
# else, fails the test. A validation that reads a variable refused itself is
# not evaluated (Terraform skips it), so a bad CPU names only the CPU.

mock_provider "aws" {
  # What `locals.azs` slices: a mock list is empty, and the slice would fail.
  mock_data "aws_availability_zones" {
    defaults = {
      names = ["us-east-1a", "us-east-1b", "us-east-1c"]
    }
  }

  mock_data "aws_caller_identity" {
    defaults = {
      account_id = "123456789012"
    }
  }

  mock_data "aws_partition" {
    defaults = {
      partition = "aws"
    }
  }

  # The provider still checks a role's policy is a JSON policy, and a mock
  # string is not one.
  mock_data "aws_iam_policy_document" {
    defaults = {
      json = "{\"Version\":\"2012-10-17\",\"Statement\":[]}"
    }
  }
}

mock_provider "aws" {
  alias = "dr"
}

# The eight variables with no default, as prod.tfvars sets them.
variables {
  backend_image         = "123456789012.dkr.ecr.us-east-1.amazonaws.com/birdtest-backend:r42"
  derived_builder_image = "123456789012.dkr.ecr.us-east-1.amazonaws.com/birdtest-derived-builder:r42"
  frontend_image        = "123456789012.dkr.ecr.us-east-1.amazonaws.com/birdtest-frontend:r42"
  alert_email           = "ops@birdtest.org"
  acm_certificate_arn   = "arn:aws:acm:us-east-1:123456789012:certificate/0f3c2c4e-1b2a-4c5d-9e8f-1234567890ab"
  ses_domain            = "birdtest.org"
  mail_from_address     = "noreply@birdtest.org"
  public_url            = "https://birdtest.org"
}

# --- Values that must plan ---------------------------------------------------

run "the_defaults_plan" {
  command = plan
}

# The first apply of README "Deploying", step 4.
run "a_stack_with_no_task_and_no_schedules_plans" {
  command = plan
  variables {
    desired_count           = 0
    scheduled_tasks_enabled = false
  }
}

# RUNBOOK §5's copy: another region, a suffix, and dr_region moved off it.
run "a_dr_copy_plans" {
  command = plan
  variables {
    name_suffix          = "-dr"
    region               = "us-west-2"
    dr_region            = "us-east-1"
    azs                  = ["us-west-2b", "us-west-2c"]
    acm_certificate_arn  = "arn:aws:acm:us-west-2:123456789012:certificate/0f3c2c4e-1b2a-4c5d-9e8f-1234567890ab"
    db_allocated_storage = 59578
  }
}

run "every_edge_of_the_ranges_plans" {
  command = plan
  variables {
    db_backup_retention_days      = 35
    backup_retention_days         = 31
    backup_object_lock_days       = 1
    backup_task_cpu               = 256
    backup_task_memory            = 512
    backup_ephemeral_storage_gib  = 21
    restore_ephemeral_storage_gib = 21
    backup_dump_jobs              = 1
    task_cpu                      = 16384
    task_memory                   = 122880
    derived_builder_cpu           = 512
    derived_builder_memory        = 4096
    mail_max_per_second           = 1000
  }
}

# Cross-variable rules (KL-62), on their accepting side.
run "related_values_that_agree_plan" {
  command = plan
  variables {
    # A subdomain of the verified domain, in another case.
    mail_from_address = "NoReply@Mail.Birdtest.org"
    # No tag on either is `latest` on both.
    backend_image         = "registry.example.org:5000/birdtest-backend"
    derived_builder_image = "registry.example.org:5000/birdtest-derived-builder"
  }
}

run "images_named_by_digest_plan" {
  command = plan
  variables {
    backend_image         = "registry.example.org/birdtest-backend@sha256:0000000000000000000000000000000000000000000000000000000000000001"
    derived_builder_image = "registry.example.org/birdtest-derived-builder@sha256:0000000000000000000000000000000000000000000000000000000000000002"
  }
}

# A floor the backend reads as 0.2.0 (`Version::parse_strict`).
run "a_two_part_magpie_floor_plans" {
  command = plan
  variables { min_magpie_version = "0.2" }
}

# --- One variable at a time, refused -----------------------------------------

run "a_suffix_without_its_hyphen_is_refused" {
  command = plan
  variables { name_suffix = "dr" }
  expect_failures = [var.name_suffix]
}

run "the_backup_suffix_is_refused" {
  command = plan
  variables { name_suffix = "-backup" }
  expect_failures = [var.name_suffix]
}

run "an_empty_region_is_refused" {
  command = plan
  variables { region = "" }
  expect_failures = [var.region]
}

run "one_zone_is_refused" {
  command = plan
  variables { azs = ["us-east-1a"] }
  expect_failures = [var.azs]
}

run "an_empty_backend_image_is_refused" {
  command = plan
  variables { backend_image = " " }
  expect_failures = [var.backend_image]
}

run "an_empty_frontend_image_is_refused" {
  command = plan
  variables { frontend_image = "" }
  expect_failures = [var.frontend_image]
}

run "a_database_under_20_gib_is_refused" {
  command = plan
  variables { db_allocated_storage = 19 }
  expect_failures = [var.db_allocated_storage]
}

run "a_database_too_large_to_autoscale_is_refused" {
  command = plan
  variables { db_allocated_storage = 59579 }
  expect_failures = [var.db_allocated_storage]
}

run "no_point_in_time_recovery_is_refused" {
  command = plan
  variables { db_backup_retention_days = 0 }
  expect_failures = [var.db_backup_retention_days]
}

run "more_recovery_than_rds_keeps_is_refused" {
  command = plan
  variables { db_backup_retention_days = 36 }
  expect_failures = [var.db_backup_retention_days]
}

run "a_malformed_dr_region_is_refused" {
  command = plan
  variables { dr_region = "west" }
  expect_failures = [var.dr_region]
}

run "a_dr_region_equal_to_the_region_is_refused" {
  command = plan
  variables { dr_region = "us-east-1" }
  expect_failures = [var.dr_region]
}

run "dumps_expiring_before_glacier_are_refused" {
  command = plan
  variables { backup_retention_days = 30 }
  expect_failures = [var.backup_retention_days]
}

run "no_object_lock_is_refused" {
  command = plan
  variables { backup_object_lock_days = 0 }
  expect_failures = [var.backup_object_lock_days]
}

run "a_backup_cpu_fargate_lacks_is_refused" {
  command = plan
  variables { backup_task_cpu = 300 }
  expect_failures = [var.backup_task_cpu]
}

run "a_backup_memory_its_cpu_cannot_take_is_refused" {
  command = plan
  variables { backup_task_memory = 1024 }
  expect_failures = [var.backup_task_memory]
}

run "too_little_backup_disk_is_refused" {
  command = plan
  variables { backup_ephemeral_storage_gib = 20 }
  expect_failures = [var.backup_ephemeral_storage_gib]
}

run "more_restore_disk_than_fargate_has_is_refused" {
  command = plan
  variables { restore_ephemeral_storage_gib = 201 }
  expect_failures = [var.restore_ephemeral_storage_gib]
}

run "no_dump_jobs_is_refused" {
  command = plan
  variables { backup_dump_jobs = 0 }
  expect_failures = [var.backup_dump_jobs]
}

run "an_address_with_no_dot_in_its_domain_is_refused" {
  command = plan
  variables { alert_email = "ops@birdtest" }
  expect_failures = [var.alert_email]
}

run "two_alert_addresses_are_refused" {
  command = plan
  variables { alert_email = "ops@birdtest.org, oncall@birdtest.org" }
  expect_failures = [var.alert_email]
}

run "a_task_cpu_fargate_lacks_is_refused" {
  command = plan
  variables { task_cpu = 300 }
  expect_failures = [var.task_cpu]
}

run "a_task_memory_its_cpu_cannot_take_is_refused" {
  command = plan
  variables { task_memory = 1536 }
  expect_failures = [var.task_memory]
}

# The single-instance rule: KL-82, and desired_count's description.
run "two_tasks_are_refused" {
  command = plan
  variables { desired_count = 2 }
  expect_failures = [var.desired_count]
}

run "a_negative_task_count_is_refused" {
  command = plan
  variables { desired_count = -1 }
  expect_failures = [var.desired_count]
}

run "a_placeholder_ses_domain_is_refused" {
  command = plan
  variables {
    ses_domain        = "birdtest.example"
    mail_from_address = "noreply@birdtest.example"
  }
  expect_failures = [var.ses_domain]
}

run "a_placeholder_sender_is_refused" {
  command = plan
  variables { mail_from_address = "noreply@birdtest.example" }
  expect_failures = [var.mail_from_address]
}

run "a_url_for_a_ses_domain_is_refused" {
  command = plan
  variables { ses_domain = "https://birdtest.org" }
  expect_failures = [var.ses_domain]
}

run "a_public_url_with_a_trailing_slash_is_refused" {
  command = plan
  variables { public_url = "https://birdtest.org/" }
  expect_failures = [var.public_url]
}

run "a_plain_http_public_url_is_refused" {
  command = plan
  variables { public_url = "http://birdtest.org" }
  expect_failures = [var.public_url]
}

run "the_placeholder_public_url_is_refused" {
  command = plan
  variables { public_url = "https://birdtest.example" }
  expect_failures = [var.public_url]
}

run "no_mail_rate_is_refused" {
  command = plan
  variables { mail_max_per_second = 0 }
  expect_failures = [var.mail_max_per_second]
}

run "a_fractional_mail_rate_is_refused" {
  command = plan
  variables { mail_max_per_second = 1.5 }
  expect_failures = [var.mail_max_per_second]
}

run "an_empty_builder_image_is_refused" {
  command = plan
  variables { derived_builder_image = "" }
  expect_failures = [var.derived_builder_image]
}

run "a_builder_cpu_too_small_for_a_table_is_refused" {
  command = plan
  variables { derived_builder_cpu = 256 }
  expect_failures = [var.derived_builder_cpu]
}

run "a_builder_memory_its_cpu_cannot_take_is_refused" {
  command = plan
  variables { derived_builder_memory = 4096 }
  expect_failures = [var.derived_builder_memory]
}

run "a_builder_memory_too_small_for_a_table_is_refused" {
  command = plan
  variables {
    derived_builder_cpu    = 512
    derived_builder_memory = 2048
  }
  expect_failures = [var.derived_builder_memory]
}

run "a_prerelease_magpie_floor_is_refused" {
  command = plan
  variables { min_magpie_version = "0.2.0-rc1" }
  expect_failures = [var.min_magpie_version]
}

run "a_magpie_floor_with_a_v_is_refused" {
  command = plan
  variables { min_magpie_version = "v0.2.0" }
  expect_failures = [var.min_magpie_version]
}

run "too_little_builder_disk_is_refused" {
  command = plan
  variables { derived_builder_ephemeral_storage_gib = 20 }
  expect_failures = [var.derived_builder_ephemeral_storage_gib]
}

# --- Cross-variable rules (KL-62), refused -----------------------------------

run "a_builder_at_another_tag_is_refused" {
  command = plan
  variables { derived_builder_image = "123456789012.dkr.ecr.us-east-1.amazonaws.com/birdtest-derived-builder:r41" }
  expect_failures = [var.derived_builder_image]
}

run "a_tagged_builder_beside_an_untagged_backend_is_refused" {
  command = plan
  variables { backend_image = "123456789012.dkr.ecr.us-east-1.amazonaws.com/birdtest-backend" }
  expect_failures = [var.derived_builder_image]
}

run "a_certificate_from_another_region_is_refused" {
  command = plan
  variables { acm_certificate_arn = "arn:aws:acm:us-west-2:123456789012:certificate/0f3c2c4e-1b2a-4c5d-9e8f-1234567890ab" }
  expect_failures = [var.acm_certificate_arn]
}

run "an_arn_that_is_not_a_certificate_is_refused" {
  command = plan
  variables { acm_certificate_arn = "arn:aws:iam::123456789012:server-certificate/birdtest" }
  expect_failures = [var.acm_certificate_arn]
}

run "a_sender_outside_the_ses_domain_is_refused" {
  command = plan
  variables { mail_from_address = "noreply@birdtest.net" }
  expect_failures = [var.mail_from_address]
}

# Not a subdomain: the domain's name ending another's.
run "a_sender_at_a_lookalike_domain_is_refused" {
  command = plan
  variables { mail_from_address = "noreply@notbirdtest.org" }
  expect_failures = [var.mail_from_address]
}

# --- S-TF-3: alerts -----------------------------------------------------------

# A circuit-breaker rollback mails the alerts topic. The service's ARN in the
# pattern and the delivery itself need a real account (TESTING.md); what plans
# is that the rule exists, matches the failed deployment and only that, on one
# resource, and has a target. That the target is the alerts topic is not
# checked: a mock topic's ARN is unknown at plan on CI's Terraform.
run "a_failed_deploy_is_alerted" {
  command = plan
  assert {
    condition     = local.deploy_failed_pattern.source == ["aws.ecs"] && local.deploy_failed_pattern["detail-type"] == ["ECS Deployment State Change"]
    error_message = "The deploy-failed rule must match ECS deployment state changes."
  }
  assert {
    condition     = local.deploy_failed_pattern.detail == { eventName = ["SERVICE_DEPLOYMENT_FAILED"] }
    error_message = "The deploy-failed rule must match only a failed deployment."
  }
  assert {
    condition     = length(local.deploy_failed_pattern.resources) == 1
    error_message = "The deploy-failed rule must name the one web service."
  }
  assert {
    condition     = aws_cloudwatch_event_target.deploy_failed.rule == "birdtest-deploy-failed"
    error_message = "The deploy-failed rule must have a target."
  }
}
