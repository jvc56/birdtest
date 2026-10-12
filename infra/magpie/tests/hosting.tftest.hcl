mock_provider "aws" {
  mock_resource "aws_cloudfront_function" {
    defaults = { arn = "arn:aws:cloudfront::123456789012:function/magpie-entry" }
  }
  mock_resource "aws_cloudfront_distribution" {
    defaults = { arn = "arn:aws:cloudfront::123456789012:distribution/TEST123" }
  }
  mock_resource "aws_s3_bucket" {
    defaults = { arn = "arn:aws:s3:::magpie-test" }
  }
  mock_data "aws_s3_object" {
    defaults = { body = "{\"version\":\"wasm-preview-v0.1.0\"}" }
  }
}

variables {
  certificate_arn = "arn:aws:acm:us-east-1:123456789012:certificate/12345678-1234-1234-1234-123456789abc"
}

run "bootstrap_without_release" {
  # Mock apply makes the bucket ID known for subsequent release data reads.
  # The provider is mocked throughout; no AWS operations are performed.
  command = apply
  assert {
    condition     = length(data.aws_s3_object.release) == 0
    error_message = "Bootstrap must not require objects before the bucket exists."
  }
  assert {
    condition     = aws_s3_bucket_public_access_block.releases.block_public_policy && aws_s3_bucket_public_access_block.releases.restrict_public_buckets
    error_message = "The origin must remain private."
  }
  assert {
    condition     = aws_cloudfront_distribution.app.default_cache_behavior[0].viewer_protocol_policy == "redirect-to-https"
    error_message = "WASM threads require a secure context."
  }
  assert {
    condition = { for header in aws_cloudfront_response_headers_policy.wasm.custom_headers_config[0].items : header.header => header.value if header.override } == {
      "Cross-Origin-Opener-Policy"   = "same-origin"
      "Cross-Origin-Embedder-Policy" = "require-corp"
      "Cross-Origin-Resource-Policy" = "same-origin"
    }
    error_message = "CloudFront must supply isolation headers itself on every asset."
  }
}

run "activate_uploaded_release" {
  command = plan
  variables { release = "wasm-preview-v0.1.0" }
}

run "wrong_manifest_refused" {
  command = plan
  variables { release = "wasm-preview-v0.2.0" }
  expect_failures = [data.aws_s3_object.release]
}

run "wrong_certificate_region_refused" {
  command = plan
  variables { certificate_arn = "arn:aws:acm:us-west-2:123456789012:certificate/12345678" }
  expect_failures = [var.certificate_arn]
}

run "unsafe_release_path_refused" {
  command = plan
  variables { release = "../other-app" }
  expect_failures = [var.release]
}
