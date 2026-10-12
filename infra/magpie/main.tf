# An independent Terraform root/state: no dependency on Birdtest's ECS service,
# database or deploy.sh. MAGPIE owns the application and its release bundles.
terraform {
  required_version = ">= 1.9"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.0"
    }
  }
}

provider "aws" {
  region = var.region
}

variable "region" {
  type    = string
  default = "us-east-1"
}

variable "domain_name" {
  type    = string
  default = "magpie.birdtest.org"
}

variable "certificate_arn" {
  description = "Issued ACM certificate covering domain_name, in us-east-1 (CloudFront's required region)."
  type        = string
  validation {
    condition     = can(regex("^arn:aws:acm:us-east-1:[0-9]{12}:certificate/[0-9a-f-]+$", var.certificate_arn))
    error_message = "CloudFront needs an ACM certificate in us-east-1."
  }
}

variable "release" {
  description = "An uploaded MAGPIE release tag. Empty creates hosting without advertising an unuploaded release."
  type        = string
  default     = ""
  validation {
    condition     = var.release == "" || can(regex("^wasm-preview-v[0-9]+\\.[0-9]+\\.[0-9]+(-[a-z0-9.]+)?$", var.release))
    error_message = "release must be empty or a wasm-preview-vMAJOR.MINOR.PATCH tag."
  }
}

resource "aws_s3_bucket" "releases" {
  bucket_prefix = "birdtest-magpie-"
  # Keep releases for existing tabs and rollback; no expiry or force_destroy.
}

resource "aws_s3_bucket_public_access_block" "releases" {
  bucket                  = aws_s3_bucket.releases.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_s3_bucket_ownership_controls" "releases" {
  bucket = aws_s3_bucket.releases.id
  rule {
    object_ownership = "BucketOwnerEnforced"
  }
}

resource "aws_s3_bucket_versioning" "releases" {
  bucket = aws_s3_bucket.releases.id
  versioning_configuration {
    status = "Enabled"
  }
}

resource "aws_s3_bucket_server_side_encryption_configuration" "releases" {
  bucket = aws_s3_bucket.releases.id
  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm = "AES256"
    }
  }
}

resource "aws_cloudfront_origin_access_control" "releases" {
  name                              = aws_s3_bucket.releases.id
  origin_access_control_origin_type = "s3"
  signing_behavior                  = "always"
  signing_protocol                  = "sigv4"
}

# The uploader writes this completion marker only after all verified files.
# Missing releases fail the plan before the entry redirect can change.
data "aws_s3_object" "release" {
  count  = var.release == "" ? 0 : 1
  bucket = aws_s3_bucket.releases.id
  key    = "releases/${var.release}/release.json"
  lifecycle {
    postcondition {
      condition     = try(jsondecode(self.body).version == var.release, false)
      error_message = "The uploaded release manifest does not match the requested release."
    }
  }
}

resource "aws_cloudfront_function" "entry" {
  name       = "${aws_s3_bucket.releases.id}-entry"
  runtime    = "cloudfront-js-2.0"
  publish    = true
  code       = templatefile("${path.module}/entry.js.tftpl", { release = var.release })
  depends_on = [data.aws_s3_object.release]
}

resource "aws_cloudfront_response_headers_policy" "wasm" {
  name = "${aws_s3_bucket.releases.id}-wasm"
  custom_headers_config {
    items {
      header   = "Cross-Origin-Opener-Policy"
      value    = "same-origin"
      override = true
    }
    items {
      header   = "Cross-Origin-Embedder-Policy"
      value    = "require-corp"
      override = true
    }
    items {
      header   = "Cross-Origin-Resource-Policy"
      value    = "same-origin"
      override = true
    }
  }
  security_headers_config {
    content_type_options {
      override = true
    }
    frame_options {
      frame_option = "DENY"
      override     = true
    }
    strict_transport_security {
      access_control_max_age_sec = 31536000
      override                   = true
    }
  }
}

resource "aws_cloudfront_distribution" "app" {
  enabled         = true
  is_ipv6_enabled = true
  comment         = "MAGPIE browser analysis preview"
  aliases         = [var.domain_name]
  price_class     = "PriceClass_100"
  origin {
    origin_id                = "releases"
    domain_name              = aws_s3_bucket.releases.bucket_regional_domain_name
    origin_access_control_id = aws_cloudfront_origin_access_control.releases.id
    s3_origin_config {
      origin_access_identity = ""
    }
  }
  default_cache_behavior {
    target_origin_id       = "releases"
    viewer_protocol_policy = "redirect-to-https"
    allowed_methods        = ["GET", "HEAD"]
    cached_methods         = ["GET", "HEAD"]
    compress               = true
    # AWS managed CachingOptimized: no cookies/Authorization go to S3.
    cache_policy_id            = "658327ea-f89d-4fab-a63d-7e88639e58f6"
    response_headers_policy_id = aws_cloudfront_response_headers_policy.wasm.id
    function_association {
      event_type   = "viewer-request"
      function_arn = aws_cloudfront_function.entry.arn
    }
  }
  # Never turn a missing JS/WASM/data file into the app's HTML page.
  custom_error_response {
    error_code            = 403
    error_caching_min_ttl = 0
  }
  custom_error_response {
    error_code            = 404
    error_caching_min_ttl = 0
  }
  restrictions {
    geo_restriction {
      restriction_type = "none"
    }
  }
  viewer_certificate {
    acm_certificate_arn      = var.certificate_arn
    ssl_support_method       = "sni-only"
    minimum_protocol_version = "TLSv1.2_2021"
  }
}

resource "aws_s3_bucket_policy" "cloudfront" {
  bucket = aws_s3_bucket.releases.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect    = "Allow"
      Principal = { Service = "cloudfront.amazonaws.com" }
      Action    = "s3:GetObject"
      Resource  = "${aws_s3_bucket.releases.arn}/releases/*"
      Condition = { StringEquals = { "AWS:SourceArn" = aws_cloudfront_distribution.app.arn } }
    }]
  })
}

output "bucket" {
  value = aws_s3_bucket.releases.id
}
output "distribution_id" {
  value = aws_cloudfront_distribution.app.id
}
output "dns_cname" {
  description = "Point domain_name at this hostname in the existing DNS provider."
  value       = aws_cloudfront_distribution.app.domain_name
}
output "url" {
  value = "https://${var.domain_name}/"
}
