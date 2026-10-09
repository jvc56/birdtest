#!/usr/bin/env python3
"""Stand-ins for aws, terraform, gh, docker and curl, for
scripts/ops-scripts-check.sh: the operator scripts run against these, never
against AWS. Installed as symlinks named after each tool; FAKE_DIR holds their
state:

  calls.log          one line per call, in order ("aws ecs update-service ...")
  s3/<bucket>/<key>  the objects `aws s3 cp` reads and writes
  s3v/<bucket>.json  a versioned bucket's versions and delete markers, as
                     [{"Key", "VersionId", "DeleteMarker"}], for
                     list-object-versions and delete-objects (a bucket with no
                     file answers the older two-version listing)
  ecr/<repo>:<tag>   the images ECR holds (`docker push` adds one)
  desired            the backend's service's desired count
  desired_frontend   the frontend's service's
  running_image      the backend image the backend's service runs (set by
                     apply)
  running_frontend_image   the frontend image the frontend's service runs
  runtask/<n>.json   each `ecs run-task`'s --overrides
  rds/<name>         an instance, holding its DbiResourceId
  ssm/<name>         a parameter's value
  tfstate_db         the DbiResourceId Terraform's state holds
  plan.json          what `terraform show -json <plan>` prints

Behaviour switches, from the environment: FAKE_STS_FAIL, FAKE_PLAN_RC (2),
FAKE_APPLY_RC (0), FAKE_APPLY_TOUCH_S3 (someone else uploads prod.tfvars
during the apply), FAKE_ROLLBACK (backend, frontend or 1 for the backend:
that service keeps running the old image), FAKE_NO_FRONTEND_SERVICE (a stack
from before the frontend had a service of its own, until an apply makes it:
frontend_service_made), FAKE_CI ("completed
success <url>"), FAKE_WORKSPACE (default).
"""
import base64
import io
import json
import os
import re
import sys
import tarfile

D = os.environ["FAKE_DIR"]


def path(*p):
    return os.path.join(D, *p)


def read(p, default=None):
    try:
        with open(p) as f:
            return f.read()
    except FileNotFoundError:
        return default


def write(p, s):
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "w") as f:
        f.write(s)


def log(tool, args):
    with open(path("calls.log"), "a") as f:
        f.write(" ".join([tool] + [a.replace("\n", "\\n") for a in args]) + "\n")


FLAGS = {
    "--interactive", "--no-publicly-accessible", "--apply-immediately", "--deletion-protection",
    "--no-deletion-protection", "--start-from-head", "--only-show-errors", "--with-decryption",
    "--overwrite", "--enable-execute-command", "--force-new-deployment", "--no-paginate",
    "--bypass-governance-retention",
}


def parse(args):
    pos, opt, i = [], {}, 0
    while i < len(args):
        a = args[i]
        if a.startswith("--") and a not in FLAGS and i + 1 < len(args):
            opt[a] = args[i + 1]
            i += 2
        elif a.startswith("--"):
            opt[a] = True
            i += 1
        else:
            pos.append(a)
            i += 1
    return pos, opt


def out(s):
    sys.stdout.write(s if s.endswith("\n") or s == "" else s + "\n")


def tfvar(text, key):
    m = re.search(r'^\s*' + key + r'\s*=\s*"?([^"\n]*)"?\s*$', text, re.M)
    return m.group(1) if m else ""


def runtask_count():
    return len(os.listdir(path("runtask"))) if os.path.isdir(path("runtask")) else 0


# --------------------------------------------------------------------------
def aws(args):
    log("aws", args)
    svc, op = args[0], args[1] if len(args) > 1 else ""
    pos, opt = parse(args[2:])
    q = opt.get("--query", "")
    if svc == "sts":
        if os.environ.get("FAKE_STS_FAIL"):
            sys.stderr.write("An error occurred (ExpiredToken): the SSO session has expired\n")
            sys.exit(255)
        out("123456789012\tarn:aws:sts::123456789012:assumed-role/AdministratorAccess/tester")
    elif svc == "configure":
        out("birdtest")
    elif svc == "s3" and op == "cp":
        src, dst = pos
        if src.startswith("s3://"):
            data = read(path("s3", src[5:]))
            if data is None:
                sys.stderr.write("fatal error: An error occurred (404) when calling the HeadObject operation\n")
                sys.exit(1)
            if dst == "-":
                sys.stdout.write(data)
            else:
                write(dst, data)
        else:
            write(path("s3", dst[5:]), read(src))
    elif svc == "s3" and op == "ls":
        out("2026-10-01 03:10:00        512 2026-10-01T03-00-00Z.manifest.json")
    elif svc == "s3api" and op == "head-bucket":
        pass
    elif svc == "s3api" and op == "get-bucket-location":
        out("eu-west-1" if "-dr-" in opt["--bucket"] else "None")
    elif svc == "s3api" and op in ("list-object-versions", "delete-objects") \
            and os.path.exists(path("s3v", opt["--bucket"] + ".json")):
        s3_versions(op, opt)
    elif svc == "s3api" and op == "list-object-versions":
        key = opt["--prefix"]
        out(json.dumps({"Versions": [
            {"Key": key, "VersionId": "v-new", "LastModified": "2026-10-05T00:00:00Z", "Size": 10, "ETag": '"b"', "IsLatest": True},
            {"Key": key, "VersionId": "v-old", "LastModified": "2026-10-01T00:00:00Z", "Size": 10, "ETag": '"a"', "IsLatest": False},
        ]}))
    elif svc == "s3api" and op == "copy-object":
        out("v-restored")
    elif svc == "ecr" and op == "describe-images":
        tag = opt["--image-ids"].split("=", 1)[1]
        if not os.path.exists(path("ecr", opt["--repository-name"] + ":" + tag)):
            sys.stderr.write("An error occurred (ImageNotFoundException)\n")
            sys.exit(254)
        out("{}")
    elif svc == "ecr" and op == "get-login-password":
        out("password")
    elif svc == "ecs":
        ecs(op, opt, q)
    elif svc == "logs" and op == "get-log-events":
        token = opt.get("--next-token")
        if token:
            out(json.dumps({"events": [], "nextForwardToken": token}))
            return
        n = runtask_count()
        overrides = json.loads(read(path("runtask", "%d.json" % n), "{}"))
        env = {e["name"]: e["value"] for c in overrides.get("containerOverrides", [])
               for e in c.get("environment", [])}
        if "BIRDTEST_PAYLOAD" in env:
            tar = tarfile.open(fileobj=io.BytesIO(base64.b64decode(env["BIRDTEST_PAYLOAD"])))
            run = tar.extractfile("./RUN").read().decode()
            msgs = ["__birdtest_run %s start" % run, "psql printed this", "__birdtest_run %s exit 0" % run]
        else:
            msgs = ["psql printed this"]
        out(json.dumps({"events": [{"message": m} for m in msgs], "nextForwardToken": "f/1"}))
    elif svc == "elbv2":
        if op == "describe-target-groups":
            out("arn:aws:elasticloadbalancing:us-east-1:123456789012:targetgroup/" + opt["--names"])
    elif svc == "rds":
        rds(op, opt, q)
    elif svc == "ssm":
        name = opt.get("--name", "")
        p = path("ssm", name.strip("/").replace("/", "_"))
        if op == "get-parameter":
            if q == "Parameter.ARN":
                out("arn:aws:ssm:us-east-1:123456789012:parameter" + name)
            else:
                v = read(p)
                if v is None:
                    sys.exit(254)
                out(v)
        elif op == "put-parameter":
            v = opt["--value"]
            if v.startswith("file://"):
                v = read(v[7:])
            write(p, v)
    elif svc == "cloudwatch":
        if op == "get-metric-statistics":
            out("1.0" if opt.get("--metric-name") == "TriggeredRules" else "0.0")
    elif svc == "events":
        if op == "describe-rule":
            out('{"source": ["aws.ecs"]}')
        elif op == "test-event-pattern":
            out("True")
    else:
        sys.stderr.write("fake aws: no such call: %s\n" % " ".join(args))
        sys.exit(2)


def s3_versions(op, opt):
    """A versioned bucket under a governance Object Lock: every version but a
    delete marker is locked, so deleting one needs the bypass flag. Called in
    the wrong region, S3 redirects; one DeleteObjects takes 1000 keys at most.
    FAKE_DUMP_DELETE_FAIL refuses every version deleted."""
    bucket = opt["--bucket"]
    p = path("s3v", bucket + ".json")
    entries = json.loads(read(p))
    want = "eu-west-1" if "-dr-" in bucket else "us-east-1"
    if opt.get("--region") != want:
        sys.stderr.write("An error occurred (PermanentRedirect): the bucket is in %s\n" % want)
        sys.exit(254)
    if op == "list-object-versions":
        prefix = opt.get("--prefix", "")
        found = [e for e in entries if e["Key"].startswith(prefix)]
        if "--no-paginate" in opt:
            # One ListObjectVersions response: MaxKeys 1000, versions and
            # delete markers together.
            page = found[:1000]
            res = {"IsTruncated": len(found) > 1000, "Prefix": prefix}
        else:
            page = found
            res = {}
            if not page:
                return  # the CLI, having followed every page, printed nothing
        v = [{"Key": e["Key"], "VersionId": e["VersionId"], "IsLatest": False}
             for e in page if not e.get("DeleteMarker")]
        m = [{"Key": e["Key"], "VersionId": e["VersionId"], "IsLatest": True}
             for e in page if e.get("DeleteMarker")]
        if v:
            res["Versions"] = v
        if m:
            res["DeleteMarkers"] = m
        out(json.dumps(res))
        return
    spec = opt["--delete"]
    assert spec.startswith("file://"), spec
    objects = json.loads(read(spec[7:]))["Objects"]
    if len(objects) > 1000:
        sys.stderr.write("An error occurred (MalformedXML) when calling the DeleteObjects operation\n")
        sys.exit(254)
    by_id = {(e["Key"], e["VersionId"]): e for e in entries}
    errors, gone = [], set()
    for o in objects:
        k = (o["Key"], o["VersionId"])
        e = by_id.get(k)
        locked = e is not None and not e.get("DeleteMarker")
        if locked and (os.environ.get("FAKE_DUMP_DELETE_FAIL")
                       or "--bypass-governance-retention" not in opt):
            errors.append({"Key": k[0], "VersionId": k[1], "Code": "AccessDenied",
                           "Message": "Access Denied because object protected by object lock."})
        else:
            gone.add(k)
    write(p, json.dumps([e for e in entries if (e["Key"], e["VersionId"]) not in gone]))
    out(json.dumps({"Errors": errors}) if errors else "{}")


def ecs(op, opt, q):
    # The backend's service is the cluster's name; the frontend's has -frontend.
    svc = opt.get("--service") or opt.get("--services") or "birdtest"
    front = svc.endswith("-frontend")
    desired_file = path("desired_frontend" if front else "desired")
    desired = read(desired_file, "1").strip()
    if op == "update-service":
        if "--desired-count" in opt:
            write(desired_file, opt["--desired-count"])
        out(opt.get("--desired-count", svc))
    elif op == "describe-services":
        if front and os.environ.get("FAKE_NO_FRONTEND_SERVICE") \
                and not os.path.exists(path("frontend_service_made")):
            out("None")  # services[0] of a MISSING service, as text
        elif "desiredCount" in q or "runningCount" in q:
            out(desired)
        elif "rolloutState" in q:
            out("PRIMARY\tCOMPLETED")
        elif "taskDefinition" in q:
            out("arn:aws:ecs:us-east-1:123456789012:task-definition/%s:7" % svc)
        elif "events" in q:
            out("2026-10-06T00:00:00Z\t(service %s) has started 1 tasks" % svc)
        elif "serviceArn" in q:
            out("arn:aws:ecs:us-east-1:123456789012:service/birdtest/%s" % svc)
        else:
            out("{}")
    elif op == "describe-task-definition":
        front_td = opt.get("--task-definition", "").split("/")[-1].startswith("birdtest-frontend")
        out(read(path("running_frontend_image" if front_td else "running_image"), "None").strip())
    elif op == "run-task":
        n = runtask_count() + 1
        write(path("runtask", "%d.json" % n), opt.get("--overrides", "{}"))
        arn = "arn:aws:ecs:us-east-1:123456789012:task/birdtest/task%d" % n
        if q:
            out(arn)
        else:
            out(json.dumps({"tasks": [{"taskArn": arn}], "failures": []}))
    elif op == "describe-tasks":
        if "lastStatus" in q:
            # A shell's task (prod-psql.sh --task) runs on; a one-off stops.
            out("RUNNING" if "shelltask" in opt.get("--tasks", "") else "STOPPED")
        elif "exitCode" in q and "stoppedReason" not in q:
            out("0")
        else:
            out("Essential container in task exited\t0")
    elif op in ("execute-command", "stop-task", "wait"):
        pass


def rds(op, opt, q):
    ident = opt.get("--db-instance-identifier")
    p = path("rds", ident) if ident else None
    if op == "describe-db-instances":
        if ident is None:
            m = re.search(r"starts_with\(DBInstanceIdentifier, '([^']*)'\)", q)
            prefix = m.group(1) if m else ""
            names = sorted(n for n in os.listdir(path("rds")) if n.startswith(prefix)) if os.path.isdir(path("rds")) else []
            out("\t".join(names))
            return
        if not os.path.exists(p):
            sys.stderr.write("An error occurred (DBInstanceNotFound)\n")
            sys.exit(254)
        answers = {
            "DbiResourceId": read(p).strip(), "DBInstanceStatus": "available",
            "Endpoint.Address": ident + ".abc.us-east-1.rds.amazonaws.com",
            "LatestRestorableTime": "2026-10-06T11:55:00+00:00", "DBParameterGroups": "birdtest-pg-1",
            "DBInstanceClass": "db.t3.micro", "AllocatedStorage": "20\t100", "MasterUsername": "birdtest",
        }
        for k, v in answers.items():
            if k in q:
                out(v)
                return
        out("{}")
    elif op == "modify-db-instance":
        new = opt.get("--new-db-instance-identifier")
        if new:
            os.rename(p, path("rds", new))
        out(new or ident)
    elif op == "restore-db-instance-to-point-in-time":
        write(path("rds", opt["--target-db-instance-identifier"]), "db-RESTORED\n")
        out(opt["--target-db-instance-identifier"])
    elif op == "wait":
        if not os.path.exists(path("rds", opt["--db-instance-identifier"])):
            sys.exit(255)
    elif op == "delete-db-instance":
        os.remove(p)
        out("deleting")
    elif op == "describe-event-subscriptions":
        out("active")


# --------------------------------------------------------------------------
def terraform(args):
    log("terraform", args)
    chdir = "."
    if args and args[0].startswith("-chdir="):
        chdir, args = args[0][len("-chdir="):], args[1:]
    cmd = args[0]
    if cmd == "output":
        raw = {"region": "us-east-1", "cluster_name": "birdtest", "ops_task_definition": "birdtest-ops",
               "log_group_name": "/ecs/birdtest", "service_security_group_id": "sg-service",
               "db_security_group_id": "sg-db", "backups_bucket": "birdtest-backups-123",
               "backups_dr_bucket": "birdtest-backups-dr-123",
               "artifacts_bucket": "birdtest-artifacts-123", "backup_task_definition": "birdtest-backup"}
        js = {"service_subnet_ids": ["subnet-1", "subnet-2"],
              "ssm_parameter_names": ["/birdtest/DATABASE_URL", "/birdtest/SESSION_SIGNING_KEY"]}
        name = args[-1]
        if "-json" in args:
            out(json.dumps(js[name]))
        else:
            sys.stdout.write(raw[name])
    elif cmd == "workspace":
        out(os.environ.get("FAKE_WORKSPACE", "default"))
    elif cmd == "plan":
        assert "-var-file=prod.tfvars" in args, "plan without prod.tfvars"
        assert os.path.exists(os.path.join(chdir, "prod.tfvars")), "no prod.tfvars to plan with"
        rc = int(os.environ.get("FAKE_PLAN_RC", "2"))
        for a in args:
            if a.startswith("-out="):
                write(a[5:], "plan")
        out("Plan: 1 to add, 1 to change, 1 to destroy." if rc == 2 else "No changes.")
        sys.exit(rc)
    elif cmd == "show":
        if len(args) > 2:
            sys.stdout.write(read(path("plan.json")))
        else:
            rid = read(path("tfstate_db"), "").strip()
            res = [{"address": "aws_db_instance.main", "values": {"resource_id": rid}}] if rid else []
            out(json.dumps({"values": {"root_module": {"resources": res}}}))
    elif cmd == "apply":
        rc = int(os.environ.get("FAKE_APPLY_RC", "0"))
        if rc:
            sys.exit(rc)
        tfvars = read(os.path.join(chdir, "prod.tfvars"))
        rolled_back = os.environ.get("FAKE_ROLLBACK", "")
        if rolled_back not in ("1", "backend"):
            write(path("running_image"), tfvar(tfvars, "backend_image"))
        if rolled_back != "frontend":
            write(path("running_frontend_image"), tfvar(tfvars, "frontend_image"))
        write(path("frontend_service_made"), "")
        if os.environ.get("FAKE_APPLY_TOUCH_S3"):
            p = path("s3", "state-bucket", "birdtest", "prod.tfvars")
            write(p, read(p) + "# someone else\n")
        out("Apply complete!")
    elif cmd == "state" and args[1] == "rm":
        os.remove(path("tfstate_db"))
    elif cmd == "import":
        write(path("tfstate_db"), read(path("rds", args[-1])))
    elif cmd == "version":
        out('{"terraform_version": "1.16.5"}')
    elif cmd == "init":
        pass
    else:
        sys.stderr.write("fake terraform: no such call: %s\n" % " ".join(args))
        sys.exit(2)


def gh(args):
    log("gh", args)
    out(os.environ.get("FAKE_CI", "completed success https://github.com/x/actions/runs/1"))


def docker(args):
    log("docker", args)
    if args[0] == "login":
        sys.stdin.read()
    elif args[0] == "push":
        repo, tag = args[1].rsplit("/", 1)[1].split(":")
        write(path("ecr", repo + ":" + tag), "")


def curl(args):
    log("curl", args)


tool = os.path.basename(sys.argv[0])
{"aws": aws, "terraform": terraform, "gh": gh, "docker": docker, "curl": curl,
 "session-manager-plugin": lambda a: None}[tool](sys.argv[1:])
