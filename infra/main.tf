terraform {
  required_version = ">= 1.6"
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

# Replication target for the artifact and backup buckets. Nothing is served
# from this region; it exists so that losing the primary one is survivable.
provider "aws" {
  alias  = "dr"
  region = var.dr_region
}

data "aws_caller_identity" "current" {}

locals {
  # Every name that has to be unique beyond this region -- buckets are global,
  # IAM roles account-wide -- is built from this, so a second copy of the stack
  # in the same account (a region-loss rebuild, RUNBOOK.md §5) differs from the
  # first by `name_suffix` alone.
  name = "${var.project}${var.name_suffix}"
  tags = {
    Project   = var.project
    ManagedBy = "terraform"
  }
}

# --- Network ---------------------------------------------------------------

# The zones, unless `azs` names them: the region's first two that need no
# opt-in (Local and Wavelength Zones do), in name order. In us-east-1 that is
# us-east-1a and us-east-1b, what the variable used to default to.
data "aws_availability_zones" "available" {
  state = "available"

  filter {
    name   = "opt-in-status"
    values = ["opt-in-not-required"]
  }
}

locals {
  azs = var.azs != null ? var.azs : slice(sort(data.aws_availability_zones.available.names), 0, 2)
}
# Public subnets carry the ALB and the Fargate tasks; private subnets hold RDS,
# which is never reachable from outside the VPC.

resource "aws_vpc" "main" {
  cidr_block           = var.vpc_cidr
  enable_dns_support   = true
  enable_dns_hostnames = true
  tags                 = merge(local.tags, { Name = local.name })
}

resource "aws_internet_gateway" "main" {
  vpc_id = aws_vpc.main.id
  tags   = local.tags
}

resource "aws_subnet" "public" {
  count                   = length(local.azs)
  vpc_id                  = aws_vpc.main.id
  cidr_block              = cidrsubnet(var.vpc_cidr, 8, count.index)
  availability_zone       = local.azs[count.index]
  map_public_ip_on_launch = true
  tags                    = merge(local.tags, { Name = "${local.name}-public-${count.index}" })
}

resource "aws_subnet" "private" {
  count             = length(local.azs)
  vpc_id            = aws_vpc.main.id
  cidr_block        = cidrsubnet(var.vpc_cidr, 8, count.index + 100)
  availability_zone = local.azs[count.index]
  tags              = merge(local.tags, { Name = "${local.name}-private-${count.index}" })
}

resource "aws_route_table" "public" {
  vpc_id = aws_vpc.main.id
  route {
    cidr_block = "0.0.0.0/0"
    gateway_id = aws_internet_gateway.main.id
  }
  tags = local.tags
}

resource "aws_route_table_association" "public" {
  count          = length(aws_subnet.public)
  subnet_id      = aws_subnet.public[count.index].id
  route_table_id = aws_route_table.public.id
}
