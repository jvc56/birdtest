# Two services, each with its own task definition: the Axum backend, a single
# instance whose old task stops before the new one starts, and Nginx serving
# the SvelteKit static build, which rolls without a gap. Split so that a
# release that changes only the frontend never stops the backend
# (scripts/deploy.sh rebuilds only the images whose sources changed, and each
# service has its own image variable). The load balancer routes between them;
# neither talks to the other.

resource "aws_ecs_cluster" "main" {
  name = local.name
  tags = local.tags

  # ECS Exec (scripts/prod-shell.sh, RUNBOOK §2 and §5) is not logged. By
  # default a session is logged through the task's awslogs driver, which asks
  # the ops task role for CloudWatch Logs permissions it does not have -- and
  # a transcript there would hold what the operator echoes, `DATABASE_URL`
  # among it (thirty-second audit, pass 17).
  configuration {
    execute_command_configuration {
      logging = "NONE"
    }
  }
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

# The backend's, and that of every one-off task that needs the database (the
# backup, the ops task, the derived builder): the only group the database
# admits (rds.tf).
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

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = local.tags
}

# The frontend's own group, not the one above: a static file server is no way
# into the database. Egress for the image pull and the log driver.
resource "aws_security_group" "frontend" {
  name        = "${local.name}-frontend"
  description = "The frontend's ECS tasks; only the ALB may reach them"
  vpc_id      = aws_vpc.main.id

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
  # they send keep-alives. Nginx's `keepalive_timeout`
  # (frontend/docker/default.conf.template) stays above this, or a pooled
  # connection it closes first answers a page request 502 (F-NGINX-2).
  idle_timeout    = 300
  security_groups = [aws_security_group.alb.id]
  subnets         = aws_subnet.public[*].id
  tags            = local.tags
}

# The backend's target group sets `deregistration_delay` and the health check's
# cadence rather than taking the defaults, because its service (below) stops
# its one task before it starts the next, and the defaults decide how long
# nothing serves.
#
# ECS deregisters a stopping task's targets and waits out the deregistration
# delay -- 300 seconds by default -- before it sends SIGTERM; the load balancer
# routes nothing new to a draining target, so with a single task that is five
# minutes of 503s before the old process is even asked to stop. The new task
# then has to pass `healthy_threshold` checks `interval` apart (3 x 30 s by
# default) before it is sent a request. Together with Fargate's own start-up
# that made a deployment seven or eight minutes with nothing serving, where the
# comment on the service says "a few seconds" -- longer than the heartbeat
# timeout, so every claim in flight across the fleet lapsed on every deploy.
#
# Ten seconds of draining, not thirty: every one of them is a second with
# nothing serving. A request in flight is milliseconds; a large result upload
# that outlasts the drain is cut, and the worker sends it again (MAGPIE's
# `http_client` retries for about fifteen minutes) to the new task, and the
# process finishes what it still holds on SIGTERM (`main.rs`). Two checks ten
# seconds apart put a started task in service within about twenty seconds.
# What is left is Fargate's provisioning, a minute or so, which MAGPIE's retry
# budget and the server's reclamation grace after a restart
# (`scheduler::reclaim_lapsed`) are sized to ride out.
#
# The frontend's service rolls instead -- its new task serves before the old
# one drains -- so its cadence is not a gap; it is set the same way to keep a
# frontend deploy to a minute or two.
resource "aws_lb_target_group" "backend" {
  name        = "${local.name}-backend"
  port        = 8080
  protocol    = "HTTP"
  target_type = "ip"
  vpc_id      = aws_vpc.main.id

  deregistration_delay = 10

  health_check {
    path                = "/health"
    matcher             = "200"
    interval            = 10
    timeout             = 5
    healthy_threshold   = 2
    unhealthy_threshold = 3
  }

  tags = local.tags
}

resource "aws_lb_target_group" "frontend" {
  name        = "${local.name}-frontend"
  port        = 80
  protocol    = "HTTP"
  target_type = "ip"
  vpc_id      = aws_vpc.main.id

  deregistration_delay = 10

  health_check {
    path                = "/"
    matcher             = "200"
    interval            = 10
    timeout             = 5
    healthy_threshold   = 2
    unhealthy_threshold = 3
  }

  tags = local.tags
}

# The site is down: no healthy target behind the load balancer for ten
# minutes. Nothing else alarms on it -- a crash-looping task, a rollback onto a
# schema it refuses, a health check that never passes -- and the backend's
# service keeps no healthy task through a deploy, so ten minutes, not one,
# clears a deploy (migrations run inside the health check grace) without
# paging. Only while the services are meant to run: at desired_count 0 (a
# first apply, RUNBOOK §5's first step) there is nothing to be healthy. One
# alarm per service, each on its own target group; a database reset stops the
# backend's alone, so only its alarm fires then.
resource "aws_cloudwatch_metric_alarm" "no_healthy_targets" {
  for_each = var.desired_count > 0 ? {
    backend  = aws_lb_target_group.backend.arn_suffix
    frontend = aws_lb_target_group.frontend.arn_suffix
  } : {}

  alarm_name        = "${local.name}-${each.key}-down"
  alarm_description = "birdtest's ${each.key} has had no healthy target for ten minutes"
  namespace         = "AWS/ApplicationELB"
  metric_name       = "HealthyHostCount"
  dimensions = {
    TargetGroup  = each.value
    LoadBalancer = aws_lb.main.arn_suffix
  }
  statistic           = "Maximum"
  period              = 60
  evaluation_periods  = 10
  threshold           = 1
  comparison_operator = "LessThanThreshold"
  # No datapoints at all -- as with nothing registered -- counts as down.
  treat_missing_data = "breaching"
  alarm_actions      = [aws_sns_topic.alerts.arn]
  ok_actions         = [aws_sns_topic.alerts.arn]
  tags               = local.tags
}

# Plain HTTP redirects pages (and refuses the API, below). The backend runs
# with SECURE_COOKIES=true, and a browser discards a Secure cookie set over
# http, so serving the app on port 80 would make signing in silently
# impossible -- and would send session cookies and API keys in the clear if it
# did not.
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

# Except the API: a worker configured with `server http://...` was redirected,
# and MAGPIE followed -- sending its API key or anonymous UUID in the clear on
# every request, with nothing to show for it (an anonymous worker simply
# worked; a keyed one lost its key header at the scheme change and its work
# with it). Refused here instead, so the first request fails and says why.
# The app never calls the API over http -- its page is redirected -- though
# someone opening a copied http://.../api/... link now reads the 426's text.
resource "aws_lb_listener_rule" "http_api_refused" {
  listener_arn = aws_lb_listener.http.arn
  priority     = 1

  condition {
    path_pattern {
      values = ["/api/*"]
    }
  }

  action {
    type = "fixed-response"
    fixed_response {
      content_type = "text/plain"
      status_code  = "426"
      message_body = "birdtest's API is served over https only: set server to https://, and treat any API key sent over http as disclosed (revoke it on the site)."
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
# path is the SPA, served by Nginx. The only rule on this listener, so nothing
# can come before it: Nginx's own /api/ and /health locations, which do this
# routing in compose, are never reached in production (BACKEND_UPSTREAM in the
# frontend's task definition, below).
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
      local.ssm_database_url_arn,
      local.ssm_session_signing_key_arn,
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
  # A failed or abandoned streaming upload (an export, a KLV) is aborted rather
  # than left for the bucket's seven-day rule to find.
  statement {
    actions   = ["s3:AbortMultipartUpload"]
    resources = ["${aws_s3_bucket.artifacts.arn}/*"]
  }
  # Purging a job deletes its exports' objects, which hold the results the
  # purge has just removed. Exports only: every other object here -- input
  # data, derived files, leave-generation KLVs -- the web process never
  # deletes, and a compromised one should not be able to either. (The derived
  # builder's own role may delete a damaged `inputs/` object; derived.tf.)
  statement {
    actions   = ["s3:DeleteObject"]
    resources = ["${aws_s3_bucket.artifacts.arn}/exports/*"]
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

# --- Task definitions and services -----------------------------------------

# The backend and the frontend were one task definition and one service until
# the October 2026 split; these keep the backend's resources rather than
# replacing them (its service keeps its name, its task definition its family).
moved {
  from = aws_ecs_task_definition.main
  to   = aws_ecs_task_definition.backend
}

moved {
  from = aws_ecs_service.main
  to   = aws_ecs_service.backend
}

resource "aws_ecs_task_definition" "backend" {
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
      # Fargate's most. The default 30 s SIGKILLs the graceful shutdown's
      # in-flight work -- a submission's insert, an artifact rebuild -- that
      # it exists to let finish.
      stopTimeout = 120
      environment = [
        { name = "BIND_ADDR", value = "0.0.0.0:8080" },
        { name = "SECURE_COOKIES", value = "true" },
        { name = "MAIL_BACKEND", value = "ses" },
        { name = "MAIL_FROM", value = var.mail_from_address },
        { name = "MAIL_MAX_PER_SECOND", value = tostring(var.mail_max_per_second) },
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
          { name = "DATABASE_URL", valueFrom = local.ssm_database_url_arn },
          { name = "SESSION_SIGNING_KEY", valueFrom = local.ssm_session_signing_key_arn }
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
    }
  ])

  tags = local.tags
}

resource "aws_ecs_service" "backend" {
  name            = local.name
  cluster         = aws_ecs_cluster.main.id
  task_definition = aws_ecs_task_definition.backend.arn
  desired_count   = var.desired_count
  launch_type     = "FARGATE"

  # Stop the old task before starting the new one, rather than ECS's default
  # rolling deploy (minimum 100%, maximum 200%), which runs both at once.
  #
  # birdtest is a single instance by construction and several things depend on
  # it: a starting process marks any input-data import or job export left
  # `running` as failed, and releases any open leave-generation transition, on
  # the assumption that the process that owned it is gone. Under the default
  # the new task does that to the old task's live work. The dispatch holds
  # that keep claims off a job being purged or deleted, and the purge count the
  # finish check compares, are in-process, so neither instance would see the
  # other's purge; rate limits are per process and SSE subscribers only hear
  # submissions made to their own instance, so an overlap is wrong for those
  # too -- see `desired_count`'s description in variables.tf.
  #
  # The cost is a gap with no instance serving during a deployment: the old
  # task's draining (10 s, set on its target group above), its shutdown, the
  # new task's provisioning and its first health checks -- a minute or two in
  # all, not seconds. Worker requests retry for about fifteen minutes (MAGPIE's
  # `http_client`), a restarted server reclaims no claim until it has been up
  # for the heartbeat timeout (`scheduler::reclaim_lapsed`), and the
  # dashboard's stream reconnects. The alternative is an invariant that
  # silently does not hold exactly when the code changes. Pages keep loading
  # through it: they are the frontend's service, which this does not touch.
  deployment_minimum_healthy_percent = 0
  deployment_maximum_percent         = 100

  # A release whose task cannot start -- a refused configuration, its own
  # MAGPIE below `min_magpie_version`, a missing SSM parameter, a migration
  # that fails -- was retried forever with nothing serving, since the old task
  # is already gone (above). After three failed launches (ECS's least, for one
  # task) the deployment fails and ECS goes back to the last task definition
  # that reached a steady state; the previous image starts on the migrated
  # schema because migrations after release are additive. Terraform's state
  # still names the new revision, so the next apply redeploys it until
  # prod.tfvars is changed: RUNBOOK.md, "Rolling back a deploy". It does not
  # help when what fails is shared by both revisions -- an SSM value, the
  # database -- and a first deployment has nothing to go back to.
  deployment_circuit_breaker {
    enable   = true
    rollback = true
  }

  # The backend migrates before it binds, and the load balancer's checks
  # (3 x 10 s) fail until it does. Without a grace period ECS replaced a task
  # whose migration took more than about thirty seconds -- killed mid-way,
  # rolled back, retried forever, with nothing serving.
  health_check_grace_period_seconds = 600

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

  depends_on = [aws_lb_listener.https]
  tags       = local.tags
}

# Nginx and the static build, and nothing else: no secrets, no task role, no
# database. A quarter of a vCPU serves static files with room to spare.
resource "aws_ecs_task_definition" "frontend" {
  family                   = "${local.name}-frontend"
  requires_compatibilities = ["FARGATE"]
  network_mode             = "awsvpc"
  cpu                      = 256
  memory                   = 512
  execution_role_arn       = aws_iam_role.execution.arn

  container_definitions = jsonencode([
    {
      name         = "frontend"
      image        = var.frontend_image
      essential    = true
      portMappings = [{ containerPort = 80, protocol = "tcp" }]
      # Nothing here proxies to the backend: the load balancer sends /api/*
      # and /health to the backend's target group before a request reaches
      # this task (`aws_lb_listener_rule.api`). Nginx still loads the /api/
      # location compose uses, and resolves its upstream as it loads: a name
      # that resolves to nothing stops it starting, so this is an address,
      # one where nothing listens -- a request that did arrive would be
      # refused at once (502), not hang.
      environment = [
        { name = "BACKEND_UPSTREAM", value = "127.0.0.1:1" },
      ]
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

# Stateless, so ECS's ordinary rolling deploy: the new task starts and passes
# its health checks before the old one drains, and pages never stop. One task,
# as many as the backend: a second would keep pages up while a lone task is
# replaced, but every page is a client of the backend's API, which has the
# same single task and the same gap, so it would buy nothing a visitor could
# use. The same `desired_count`: 0 while the stack is built, 1 after.
resource "aws_ecs_service" "frontend" {
  name            = "${local.name}-frontend"
  cluster         = aws_ecs_cluster.main.id
  task_definition = aws_ecs_task_definition.frontend.arn
  desired_count   = var.desired_count
  launch_type     = "FARGATE"

  deployment_minimum_healthy_percent = 100
  deployment_maximum_percent         = 200

  # A release whose Nginx does not start never takes traffic (the old task
  # serves on); after three failed launches ECS gives up on it and the alert
  # below says so, as for the backend.
  deployment_circuit_breaker {
    enable   = true
    rollback = true
  }

  # Nginx serves within seconds of starting; this only covers the image pull.
  health_check_grace_period_seconds = 60

  network_configuration {
    subnets          = aws_subnet.public[*].id
    security_groups  = [aws_security_group.frontend.id]
    assign_public_ip = true
  }

  load_balancer {
    target_group_arn = aws_lb_target_group.frontend.arn
    container_name   = "frontend"
    container_port   = 80
  }

  depends_on = [aws_lb_listener.https]
  tags       = local.tags
}

# The circuit breaker's rollback, said out loud. Three failed launches usually
# finish inside the `-down` alarms' ten minutes and the old revision is healthy
# again, and `apply` has already returned, so without this nothing says the
# release is not live -- while Terraform's state still names it (the next apply
# of any kind redeploys it), the derived-data builder's schedule runs it (the
# family's latest revision, derived.tf), and the nightly dumps' manifests name
# its image. ECS sends SERVICE_DEPLOYMENT_FAILED when the breaker trips, with
# the service's ARN (`id`) as the event's resource: either service's. A local,
# not inline, so that S-TF-3 can read the pattern at plan time: the ARNs are
# not known until apply, and an encoded pattern holding them is unknown as a
# whole.
locals {
  deploy_failed_pattern = {
    source      = ["aws.ecs"]
    detail-type = ["ECS Deployment State Change"]
    resources   = [aws_ecs_service.backend.id, aws_ecs_service.frontend.id]
    detail = {
      eventName = ["SERVICE_DEPLOYMENT_FAILED"]
    }
  }
}

resource "aws_cloudwatch_event_rule" "deploy_failed" {
  name        = "${local.name}-deploy-failed"
  description = "A deployment of the backend or frontend service failed and ECS rolled it back (circuit breaker)"

  event_pattern = jsonencode(local.deploy_failed_pattern)

  tags = local.tags
}

resource "aws_cloudwatch_event_target" "deploy_failed" {
  rule      = aws_cloudwatch_event_rule.deploy_failed.name
  target_id = "sns"
  arn       = aws_sns_topic.alerts.arn

  input_transformer {
    input_paths = {
      service    = "$.resources[0]"
      reason     = "$.detail.reason"
      deployment = "$.detail.deploymentId"
    }
    input_template = "\"birdtest deploy FAILED and ECS rolled <service> back to the previous release: <reason> (<deployment>). Terraform's state still names the failed release, so the next apply deploys it again (and, for the backend, the derived-data builder runs it until then): follow RUNBOOK.md, Rolling back a deploy, from step 2, before any other apply. The stopped tasks and the CloudWatch log group ${aws_cloudwatch_log_group.main.name} say what failed.\""
  }
}
