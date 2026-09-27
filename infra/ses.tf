# Outbound account mail: confirmation codes and password-reset links.
# DNS records for the verification tokens are added wherever the domain is
# hosted; this only declares the identities.

resource "aws_sesv2_email_identity" "domain" {
  email_identity = var.ses_domain
  tags           = local.tags
}

resource "aws_sesv2_email_identity_mail_from_attributes" "domain" {
  email_identity   = aws_sesv2_email_identity.domain.email_identity
  mail_from_domain = "mail.${var.ses_domain}"
}

# --- Deliverability --------------------------------------------------------
# SES pauses an account's sending when its bounce or complaint rate stays high
# (it reviews the account at 5% and 0.1%, and may pause it at 10% and 0.5%),
# and from then on every confirmation
# code and reset link fails -- logged, but answered to the caller exactly as a
# success, so nobody who registers or resets can tell. Registration mails any
# address it is given, so bounces are the one rate a visitor can drive. Three
# things stand against that: addresses that bounced or complained are never
# mailed again, the two rates alarm well before SES acts, and a failed send
# alarms at once, whatever its cause (the audit's pass 24).

# Account-wide in this region, and SES's default for accounts made since 2019;
# declared so that it is certain. SES accepts a send to a suppressed address
# and delivers nothing: not a failure here, and not counted in the bounce rate.
# Only hard bounces and complaints are suppressed.
resource "aws_sesv2_account_suppression_attributes" "main" {
  suppressed_reasons = ["BOUNCE", "COMPLAINT"]
}

resource "aws_cloudwatch_metric_alarm" "ses_reputation" {
  for_each = {
    # Below SES's own thresholds, to leave time to act before a review.
    bounce    = { metric = "Reputation.BounceRate", threshold = 0.04 }
    complaint = { metric = "Reputation.ComplaintRate", threshold = 0.0008 }
  }

  alarm_name          = "${local.name}-ses-${each.key}-rate"
  alarm_description   = "birdtest's SES ${each.key} rate is nearing the rate at which SES pauses sending"
  namespace           = "AWS/SES"
  metric_name         = each.value.metric
  statistic           = "Maximum"
  period              = 3600
  evaluation_periods  = 1
  threshold           = each.value.threshold
  comparison_operator = "GreaterThanOrEqualToThreshold"
  # SES publishes these irregularly, and not at all without mail: an hour
  # with no datapoint keeps the state it had, rather than clearing a rate
  # that is still high (AWS's own advice for these alarms).
  treat_missing_data = "ignore"
  alarm_actions      = [aws_sns_topic.alerts.arn]
  ok_actions         = [aws_sns_topic.alerts.arn]
  tags               = local.tags
}

# Every failed account mail is logged with the field `alarm = "mail_failed"`
# (routes/auth.rs), and SES's reason. A JSON pattern on that field, not a
# phrase: the frontend's nginx logs to this group too, and its access lines
# carry any User-Agent or Referer a visitor sends, so a phrase let anyone raise
# the alarm, and hold it raised over a real outage (the audit's pass 24). Only
# the backend's own JSON lines have fields. A failure is never transient by
# then -- the SDK has already retried -- so one is enough to alarm.
resource "aws_cloudwatch_log_metric_filter" "mail_failed" {
  name           = "${local.name}-mail-failed"
  log_group_name = aws_cloudwatch_log_group.main.name
  pattern        = "{ $.fields.alarm = \"mail_failed\" }"

  metric_transformation {
    name      = "MailFailed"
    namespace = "birdtest/mail"
    value     = "1"
  }
}

resource "aws_cloudwatch_metric_alarm" "mail_failed" {
  alarm_name          = "${local.name}-mail-failed"
  alarm_description   = "An account mail (confirmation, reset, notice) failed to send; the backend's log says why"
  namespace           = "birdtest/mail"
  metric_name         = aws_cloudwatch_log_metric_filter.mail_failed.metric_transformation[0].name
  statistic           = "Sum"
  period              = 300
  evaluation_periods  = 1
  threshold           = 1
  comparison_operator = "GreaterThanOrEqualToThreshold"
  treat_missing_data  = "notBreaching"
  alarm_actions       = [aws_sns_topic.alerts.arn]
  # No OK mail: five minutes with no failed send is not mail working again --
  # with nobody registering, a paused account fails nothing.
  tags = local.tags
}
