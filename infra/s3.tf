# Artifact storage: full ranked move lists and the per-generation combined KLVs
# produced by leave-generation aggregation.

resource "aws_s3_bucket" "artifacts" {
  bucket = "${local.name}-artifacts-${data.aws_caller_identity.current.account_id}"
  tags   = local.tags
}

resource "aws_s3_bucket_public_access_block" "artifacts" {
  bucket                  = aws_s3_bucket.artifacts.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_s3_bucket_versioning" "artifacts" {
  bucket = aws_s3_bucket.artifacts.id
  versioning_configuration {
    status = "Enabled"
  }
}

resource "aws_s3_bucket_server_side_encryption_configuration" "artifacts" {
  bucket = aws_s3_bucket.artifacts.id
  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm = "AES256"
    }
  }
}

# --- Lifecycle -------------------------------------------------------------
# Objects here are only ever added, never deleted (a purged job orphans its
# artifacts rather than removing them -- see PLAN.md, "Artifacts: back up, or rebuild?"), so the only
# thing to expire is the noncurrent versions left behind when a generation is
# rebuilt or overwritten.

resource "aws_s3_bucket_lifecycle_configuration" "artifacts" {
  bucket = aws_s3_bucket.artifacts.id

  rule {
    id     = "expire-noncurrent-versions"
    status = "Enabled"
    filter {}

    noncurrent_version_expiration {
      noncurrent_days = 90
    }
  }

  rule {
    id     = "abort-incomplete-uploads"
    status = "Enabled"
    filter {}

    abort_incomplete_multipart_upload {
      days_after_initiation = 7
    }
  }
}

# --- Cross-region replication ----------------------------------------------
# KLVs are derivable from leave_rack_progress and so could in principle be
# rebuilt rather than replicated (PLAN.md, "Artifacts: back up, or rebuild?"). Replication is still
# worth its storage: a rebuild depends on klv::build producing byte-identical
# output for an old generation forever, and replication does not.

resource "aws_s3_bucket" "artifacts_dr" {
  provider = aws.dr
  bucket   = "${local.name}-artifacts-dr-${data.aws_caller_identity.current.account_id}"
  tags     = local.tags
}

resource "aws_s3_bucket_public_access_block" "artifacts_dr" {
  provider                = aws.dr
  bucket                  = aws_s3_bucket.artifacts_dr.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

# Replication requires versioning on both ends.
resource "aws_s3_bucket_versioning" "artifacts_dr" {
  provider = aws.dr
  bucket   = aws_s3_bucket.artifacts_dr.id
  versioning_configuration {
    status = "Enabled"
  }
}

resource "aws_s3_bucket_server_side_encryption_configuration" "artifacts_dr" {
  provider = aws.dr
  bucket   = aws_s3_bucket.artifacts_dr.id
  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm = "AES256"
    }
  }
}

data "aws_iam_policy_document" "replication_assume" {
  statement {
    actions = ["sts:AssumeRole"]
    principals {
      type        = "Service"
      identifiers = ["s3.amazonaws.com"]
    }
  }
}

resource "aws_iam_role" "replication" {
  name               = "${local.name}-s3-replication"
  assume_role_policy = data.aws_iam_policy_document.replication_assume.json
  tags               = local.tags
}

data "aws_iam_policy_document" "replication" {
  statement {
    actions   = ["s3:GetReplicationConfiguration", "s3:ListBucket"]
    resources = [aws_s3_bucket.artifacts.arn]
  }
  statement {
    actions = [
      "s3:GetObjectVersionForReplication",
      "s3:GetObjectVersionAcl",
      "s3:GetObjectVersionTagging",
    ]
    resources = ["${aws_s3_bucket.artifacts.arn}/*"]
  }
  statement {
    actions = [
      "s3:ReplicateObject",
      "s3:ReplicateDelete",
      "s3:ReplicateTags",
    ]
    resources = ["${aws_s3_bucket.artifacts_dr.arn}/*"]
  }
}

resource "aws_iam_role_policy" "replication" {
  role   = aws_iam_role.replication.id
  policy = data.aws_iam_policy_document.replication.json
}

resource "aws_s3_bucket_replication_configuration" "artifacts" {
  # Versioning must exist before a replication rule may reference the bucket.
  depends_on = [aws_s3_bucket_versioning.artifacts, aws_s3_bucket_versioning.artifacts_dr]

  role   = aws_iam_role.replication.arn
  bucket = aws_s3_bucket.artifacts.id

  rule {
    id     = "all-artifacts"
    status = "Enabled"
    filter {}

    # A delete in the primary must not propagate: the artifact store is the
    # store that is allowed to be *newer* than the database (PLAN.md's "Backups and Restore"
    # 5.1), and a replicated delete would be the one direction that breaks a
    # restored database.
    delete_marker_replication {
      status = "Disabled"
    }

    destination {
      bucket        = aws_s3_bucket.artifacts_dr.arn
      storage_class = "STANDARD_IA"
    }
  }
}
