# Two containers in one task definition: the Axum backend and an Nginx
# container serving the SvelteKit static build. The frontend moves to
# S3 + CloudFront in a later phase.

resource "aws_ecs_cluster" "main" {
  name = local.name
  tags = local.tags
}

resource "aws_cloudwatch_log_group" "main" {
  name              = "/ecs/${local.name}"
  retention_in_days = 30
  tags              = local.tags
}

# --- Security groups -------------------------------------------------------

resource "aws_security_group" "alb" {
  name        = "${local.name}-alb"
  description = "Public HTTP/HTTPS ingress"
  vpc_id      = aws_vpc.main.id

  ingress {
    from_port   = 80
    to_port     = 80
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]
  }

  ingress {
    from_port   = 443
    to_port     = 443
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = local.tags
}

resource "aws_security_group" "service" {
  name        = "${local.name}-service"
  description = "ECS tasks; only the ALB may reach them"
  vpc_id      = aws_vpc.main.id

  ingress {
    from_port       = 8080
    to_port         = 8080
    protocol        = "tcp"
    security_groups = [aws_security_group.alb.id]
  }

  ingress {
    from_port       = 80
    to_port         = 80
    protocol        = "tcp"
    security_groups = [aws_security_group.alb.id]
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = local.tags
}

# --- Load balancer ---------------------------------------------------------

resource "aws_lb" "main" {
  name               = local.name
  load_balancer_type = "application"
  # Not the 60-second default. Seeding a leave generation's rack universe and
  # running its transition both happen on tasks of their own now, but a worker
  # uploading a large batch, an admin's results stream and an artifact rebuild
  # (about 13 seconds per generation, inline) still outlast a minute on a small
  # instance. MAGPIE's own request timeout is 120s. SSE streams are unaffected:
  # they send keep-alives.
  idle_timeout    = 300
  security_groups = [aws_security_group.alb.id]
  subnets         = aws_subnet.public[*].id
  tags            = local.tags
}

resource "aws_lb_target_group" "backend" {
  name        = "${local.name}-backend"
  port        = 8080
  protocol    = "HTTP"
  target_type = "ip"
  vpc_id      = aws_vpc.main.id

  health_check {
    path    = "/health"
    matcher = "200"
  }

  tags = local.tags
}

resource "aws_lb_target_group" "frontend" {
  name        = "${local.name}-frontend"
  port        = 80
  protocol    = "HTTP"
  target_type = "ip"
  vpc_id      = aws_vpc.main.id

  health_check {
    path    = "/"
    matcher = "200"
  }

  tags = local.tags
}

# Plain HTTP only redirects. The backend runs with SECURE_COOKIES=true, and a
# browser discards a Secure cookie set over http, so serving the app on port 80
# would make signing in silently impossible -- and would send session cookies
# and API keys in the clear if it did not.
resource "aws_lb_listener" "http" {
  load_balancer_arn = aws_lb.main.arn
  port              = 80
  protocol          = "HTTP"

  default_action {
    type = "redirect"
    redirect {
      port        = "443"
      protocol    = "HTTPS"
      status_code = "HTTP_301"
    }
  }
}

resource "aws_lb_listener" "https" {
  load_balancer_arn = aws_lb.main.arn
  port              = 443
  protocol          = "HTTPS"
  ssl_policy        = "ELBSecurityPolicy-TLS13-1-2-2021-06"
  certificate_arn   = var.acm_certificate_arn

  default_action {
    type             = "forward"
    target_group_arn = aws_lb_target_group.frontend.arn
  }
}

# Everything under /api (and the health check) goes to the backend; every other
# path is the SPA, served by Nginx.
resource "aws_lb_listener_rule" "api" {
  listener_arn = aws_lb_listener.https.arn
  priority     = 100

  action {
    type             = "forward"
    target_group_arn = aws_lb_target_group.backend.arn
  }

  condition {
    path_pattern {
      values = ["/api/*", "/health"]
    }
  }
}

# --- IAM -------------------------------------------------------------------

data "aws_iam_policy_document" "task_assume" {
  statement {
    actions = ["sts:AssumeRole"]
    principals {
      type        = "Service"
      identifiers = ["ecs-tasks.amazonaws.com"]
    }
  }
}

resource "aws_iam_role" "execution" {
  name               = "${local.name}-execution"
  assume_role_policy = data.aws_iam_policy_document.task_assume.json
  tags               = local.tags
}

resource "aws_iam_role_policy_attachment" "execution" {
  role       = aws_iam_role.execution.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AmazonECSTaskExecutionRolePolicy"
}

# The execution role needs to read the SSM parameters injected as `secrets`.
data "aws_iam_policy_document" "execution_ssm" {
  statement {
    actions = ["ssm:GetParameters"]
    resources = compact([
      aws_ssm_parameter.database_url.arn,
      aws_ssm_parameter.session_signing_key.arn,
      var.github_token_parameter_arn,
    ])
  }
}

resource "aws_iam_role_policy" "execution_ssm" {
  role   = aws_iam_role.execution.id
  policy = data.aws_iam_policy_document.execution_ssm.json
}

resource "aws_iam_role" "task" {
  name               = "${local.name}-task"
  assume_role_policy = data.aws_iam_policy_document.task_assume.json
  tags               = local.tags
}

# What the running process itself needs: the artifact bucket and SES.
data "aws_iam_policy_document" "task" {
  statement {
    actions   = ["s3:GetObject", "s3:PutObject"]
    resources = ["${aws_s3_bucket.artifacts.arn}/*"]
  }
  statement {
    actions   = ["s3:ListBucket"]
    resources = [aws_s3_bucket.artifacts.arn]
  }
  statement {
    actions   = ["ses:SendEmail"]
    resources = ["*"]
  }
}

resource "aws_iam_role_policy" "task" {
  role   = aws_iam_role.task.id
  policy = data.aws_iam_policy_document.task.json
}

# --- Task definition and service -------------------------------------------

resource "aws_ecs_task_definition" "main" {
  family                   = local.name
  requires_compatibilities = ["FARGATE"]
  network_mode             = "awsvpc"
  cpu                      = var.task_cpu
  memory                   = var.task_memory
  execution_role_arn       = aws_iam_role.execution.arn
  task_role_arn            = aws_iam_role.task.arn

  container_definitions = jsonencode([
    {
      name         = "backend"
      image        = var.backend_image
      essential    = true
      portMappings = [{ containerPort = 8080, protocol = "tcp" }]
      environment = [
        { name = "BIND_ADDR", value = "0.0.0.0:8080" },
        { name = "SECURE_COOKIES", value = "true" },
        { name = "MAIL_BACKEND", value = "ses" },
        { name = "MAIL_FROM", value = var.mail_from_address },
        { name = "PUBLIC_URL", value = var.public_url },
        { name = "S3_BUCKET", value = aws_s3_bucket.artifacts.bucket },
        { name = "AWS_REGION", value = var.region },
        { name = "RUST_LOG", value = "birdtest=info,tower_http=info" },
        # The ALB appends the client address to X-Forwarded-For; per-IP rate
        # limits (registration, login, password reset) key on it. Without this
        # they key on the ALB's own address, one bucket for the whole site.
        { name = "TRUSTED_PROXY_HOPS", value = "1" },
        # The fleet-wide MAGPIE floor, and the default floor for new jobs.
        { name = "MIN_MAGPIE_VERSION", value = var.min_magpie_version }
      ]
      # Pulled from SSM at task start, so the values never appear in the task
      # definition or in Terraform state.
      secrets = concat(
        [
          { name = "DATABASE_URL", valueFrom = aws_ssm_parameter.database_url.arn },
          { name = "SESSION_SIGNING_KEY", valueFrom = aws_ssm_parameter.session_signing_key.arn }
        ],
        # Optional: unauthenticated GitHub ref resolution for input-data
        # imports is 60 calls an hour per IP.
        var.github_token_parameter_arn == "" ? [] : [
          { name = "GITHUB_TOKEN", valueFrom = var.github_token_parameter_arn }
        ]
      )
      logConfiguration = {
        logDriver = "awslogs"
        options = {
          "awslogs-group"         = aws_cloudwatch_log_group.main.name
          "awslogs-region"        = var.region
          "awslogs-stream-prefix" = "backend"
        }
      }
    },
    {
      name         = "frontend"
      image        = var.frontend_image
      essential    = true
      portMappings = [{ containerPort = 80, protocol = "tcp" }]
      logConfiguration = {
        logDriver = "awslogs"
        options = {
          "awslogs-group"         = aws_cloudwatch_log_group.main.name
          "awslogs-region"        = var.region
          "awslogs-stream-prefix" = "frontend"
        }
      }
    }
  ])

  tags = local.tags
}

resource "aws_ecs_service" "main" {
  name            = local.name
  cluster         = aws_ecs_cluster.main.id
  task_definition = aws_ecs_task_definition.main.arn
  desired_count   = var.desired_count
  launch_type     = "FARGATE"

  # Stop the old task before starting the new one, rather than ECS's default
  # rolling deploy (minimum 100%, maximum 200%), which runs both at once.
  #
  # birdtest is a single instance by construction and several things depend on
  # it: a starting process marks any input-data import or job export left
  # `running` as failed, on the assumption that the process that owned it is
  # gone. Under the default the new task does that to the old task's live work.
  # Rate limits are per process and SSE subscribers only hear submissions made
  # to their own instance, so an overlap is wrong for those too -- see
  # `desired_count`'s description in variables.tf.
  #
  # The cost is a few seconds with no instance serving during a deployment.
  # Worker claims retry, the dashboard's stream reconnects, and the alternative
  # is an invariant that silently does not hold exactly when the code changes.
  deployment_minimum_healthy_percent = 0
  deployment_maximum_percent         = 100

  network_configuration {
    subnets          = aws_subnet.public[*].id
    security_groups  = [aws_security_group.service.id]
    assign_public_ip = true
  }

  load_balancer {
    target_group_arn = aws_lb_target_group.backend.arn
    container_name   = "backend"
    container_port   = 8080
  }

  load_balancer {
    target_group_arn = aws_lb_target_group.frontend.arn
    container_name   = "frontend"
    container_port   = 80
  }

  depends_on = [aws_lb_listener.https]
  tags       = local.tags
}
