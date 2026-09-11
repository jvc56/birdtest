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
