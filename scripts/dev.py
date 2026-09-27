#!/usr/bin/env python3
"""Bring up birdtest locally with real MAGPIE contributors, and open a browser.

One command: start the stack, wait for it, seed it, launch N `magpie
contribute` processes, and open the site. It is tier 6's setup with the
assertions and the teardown removed, and it calls the same `scripts/seed.py`,
so the development environment cannot drift from what the tests exercise.

**Contributors are always real MAGPIE.** There is no fake-worker mode here.
`worker/fake_worker.py` belongs to the end-to-end suite, where a browser
journey needs contributions to arrive on cue at predictable values without a C
toolchain in the loop — that is a property of an assertion harness, not of a
place you develop. Watching synthetic results move a dashboard tells you
nothing about what your change did.

The cost is that this needs a built MAGPIE and a real MAGPIE-DATA install, so
bringing up birdtest is no longer a Docker-only operation. It fails naming both
when either is missing rather than starting something that cannot work.

Each contributor gets its own directory, because `magpie contribute` writes the
identity it is issued into the `contribute.txt` in its working directory:
sharing one would collapse every worker onto a single identity. The MAGPIE data
directory is symlinked rather than copied (MAGPIE loads its board from `./data`
before anything else), so N workers cost nothing but their own settings files.
"""

import argparse
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time
import webbrowser
from pathlib import Path
from typing import List, Optional
from urllib.parse import quote

import requests

REPO_ROOT = Path(__file__).resolve().parent.parent


def load_env_file() -> None:
    """The repository's `.env`, as docker compose reads it, into the
    environment: the defaults below (WEB_PORT, BACKEND_PORT, ...) come from
    it. Set there, a port applied to compose but not to this script, which
    passed compose its own default and so overrode the file. A variable
    already in the environment wins, as it does for compose."""
    path = REPO_ROOT / ".env"
    if not path.is_file():
        return
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        if key.startswith("export "):
            key = key[len("export "):].strip()
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in "'\"":
            value = value[1:-1]
        os.environ.setdefault(key, value)


def log(message: str) -> None:
    print(f"[dev] {message}", flush=True)


def fail(message: str) -> "NoReturn":  # type: ignore[valid-type]
    print(f"[dev] {message}", file=sys.stderr)
    sys.exit(1)


# --- what has to exist before anything starts -------------------------------


def resolve_magpie(args) -> tuple:
    """The binary and the data directory, or a message naming what is missing.

    Checked up front rather than at first use: a stack that comes up, seeds,
    and only then discovers it has no engine has wasted a couple of minutes to
    tell you something it knew at the start.
    """
    binary = Path(args.magpie).expanduser()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        fail(
            f"no MAGPIE binary at {binary}.\n"
            "      Build one (`make magpie BUILD=portable_release` in your MAGPIE checkout, on\n"
            "      a host whose glibc is no newer than Debian bookworm's 2.36, since the backend\n"
            "      container runs it) and pass --magpie, or set MAGPIE_BIN.\n"
            "      Contributors here are always real MAGPIE; there is no fake-worker mode."
        )

    data = Path(args.magpie_data).expanduser()
    if not data.is_dir():
        fail(
            f"no MAGPIE data directory at {data}.\n"
            "      Run `./download_data.sh` in your MAGPIE checkout and pass --magpie-data, "
            "or set MAGPIE_DATA_PATH."
        )
    if data.name != "data":
        # MAGPIE resolves its data from `./data` relative to the working
        # directory, and each worker directory symlinks this in under that
        # name, so a directory called something else still works.
        log(f"note: {data} is not named 'data'; it will be linked as 'data' in each worker dir")

    return binary.resolve(), data.resolve()


def magpie_version(magpie_root: Path) -> Optional[str]:
    """`MAGPIE_VERSION` out of the checkout's source.

    The floor exists to keep a fleet off a build too old to speak the protocol,
    and production sets it to a real release (0.1.1 by default). Locally the
    contributor is whatever checkout you built, which may be older -- and then
    every task is declined with "update MAGPIE" and nothing ever runs. Reading
    the constant the binary was built from is exact.
    """
    source = magpie_root / "src" / "impl" / "config.c"
    if not source.is_file():
        return None
    match = re.search(r'#define\s+MAGPIE_VERSION\s+"([0-9.]+)"', source.read_text())
    return match.group(1) if match else None


def compose(args: List[str], check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(["docker", "compose", *args], cwd=REPO_ROOT, check=check)


# What the backend says when the database holds an older edit of the one
# migration (PLAN.md, "Resetting the database after a schema change").
SCHEMA_CHANGED = "was previously applied but has been modified"


def reset_database() -> None:
    """Drop the schema and let the backend rebuild it on its next start.

    The backend is stopped first, so the `up` after this starts it afresh and
    its migrations run on the empty schema. Left running -- as it was unless
    --rebuild recreated it -- it went on serving a database with no tables,
    and `up` saw nothing to change."""
    log("resetting the database (--reset-db)")
    compose(["stop", "backend"], check=False)
    compose(["up", "-d", "--wait", "postgres"])
    compose(["exec", "-T", "postgres", "psql", "-U", "birdtest", "-d", "birdtest", "-q",
             "-v", "ON_ERROR_STOP=1", "-c", "DROP SCHEMA public CASCADE; CREATE SCHEMA public;"])


def explain_failed_start() -> None:
    """Why the stack did not come up, from the backend's own last words."""
    logs = subprocess.run(["docker", "compose", "logs", "--no-color", "--tail", "40", "backend"],
                          cwd=REPO_ROOT, capture_output=True, text=True).stdout
    if SCHEMA_CHANGED in logs:
        log("the backend refused the database: it was made by an older edit of the schema's "
            "one migration, which changes in place until release")
        log("run again with --reset-db to drop the schema and rebuild it (the database's data "
            "goes; `scripts/dev-dump.sh` snapshots it first), with --rebuild too if the images "
            "predate the change")
    else:
        # Any service can fail the start -- a host port another project
        # holds, most often -- so compose's own error, printed above, is the
        # reason; the backend's log only when the backend is what is down.
        log("the stack did not start: docker compose's error is above")
        running = subprocess.run(["docker", "compose", "ps", "-q", "--status", "running", "backend"],
                                 cwd=REPO_ROOT, capture_output=True, text=True).stdout.strip()
        if not running:
            log("the backend is not running; its last lines:")
            print(logs.rstrip() or "(no output)")
        log("a host port in use? --web-port and --backend-port move the stack's ports, "
            "or set WEB_PORT and BACKEND_PORT in .env")


def wait_for_health(url: str, timeout: int) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            if requests.get(url, timeout=5).ok:
                return
        except requests.RequestException:
            pass
        time.sleep(1)
    fail(f"{url} did not come up within {timeout}s. Try `docker compose logs backend`.")


# --- contributors -----------------------------------------------------------


UUID_LINE = re.compile(r"^uuid\s+(\S+)\s*$", re.MULTILINE)
KEY_LINE = re.compile(r"^apikey\s+(\S+)\s*$", re.MULTILINE)

# The contributor accounts --reset-db makes are run by these workers, under
# their API keys; the others run anonymously.
KEYED_WORKERS = (3, 4)


def worker_key(settings: Path) -> Optional[str]:
    """The API key a worker runs under, kept in its settings between runs."""
    if not settings.is_file():
        return None
    match = KEY_LINE.search(settings.read_text())
    return match.group(1) if match else None


def issued_uuid(settings: Path) -> Optional[str]:
    """The identity the server issued this worker, which MAGPIE appends to its
    settings file on first contact."""
    if not settings.is_file():
        return None
    match = UUID_LINE.search(settings.read_text())
    return match.group(1) if match else None


def uuids_the_database_knows(uuids: List[str]) -> Optional[set]:
    """Which of `uuids` the stack's database has, or None when it cannot be
    asked (a stack not run by this compose file)."""
    if not uuids:
        return set()
    listed = ",".join(f"'{u}'" for u in uuids if re.fullmatch(r"[0-9a-fA-F-]{36}", u))
    if not listed:
        return set()
    result = subprocess.run(
        ["docker", "compose", "exec", "-T", "postgres", "psql", "-U", "birdtest", "-d", "birdtest",
         "-Atq", "-c", f"SELECT uuid FROM anonymous_workers WHERE uuid IN ({listed})"],
        cwd=REPO_ROOT, capture_output=True, text=True)
    if result.returncode != 0:
        return None
    return {line.strip() for line in result.stdout.splitlines() if line.strip()}


def write_contribute_settings(directory: Path, args, api_url: str, uuid: Optional[str],
                              api_key: Optional[str] = None) -> Path:
    """One `contribute.txt` per worker, written from this run's flags.

    Settings live in a file rather than on the command line so an API key stays
    out of shell history and `ps` output. MAGPIE appends the server-minted
    `uuid` to this file on first contact, which is why each worker needs its
    own: they would otherwise overwrite each other's identity. That identity
    is the one thing carried over from an earlier run (`uuid`, when given);
    the rest is rewritten every time -- written once and kept, the file
    ignored every later --threads, --max-tasks, --idle-wait and --api-key.
    """
    settings = directory / "contribute.txt"
    lines = [
        "# Written by scripts/dev.py on every run; only the uuid MAGPIE appends and",
        "# the apikey --reset-db gives workers 3 and 4 are kept. Delete this file",
        "# (or pass --reset-workers) to make this worker forget both.",
        f"server   {api_url}",
        f"threads  {args.threads}",
        f"maxtasks {args.max_tasks}",
        f"idlewait {args.idle_wait}",
    ]
    key = args.api_key or api_key
    if key:
        lines.append(f"apikey   {key}")
    if uuid:
        lines.append(f"uuid {uuid}")
    # It may hold an API key: readable by its owner only, from the start --
    # written first and chmod'd after, it was world-readable in between.
    settings.unlink(missing_ok=True)
    fd = os.open(settings, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as f:
        f.write("\n".join(lines) + "\n")
    return settings


def start_contributors(args, binary: Path, data: Path, api_url: str) -> List[subprocess.Popen]:
    workdir = Path(args.workdir).expanduser().resolve()
    workdir.mkdir(parents=True, exist_ok=True)

    # Each worker's issued identity, kept only while the database still knows
    # it: after --reset-db, or a restore, the server answers an identity it
    # never issued with 401, and MAGPIE gives up at once -- with status 0.
    directories = [workdir / f"worker-{index:02d}" for index in range(1, args.workers + 1)]
    issued = {d: issued_uuid(d / "contribute.txt") for d in directories}
    known = uuids_the_database_knows([u for u in issued.values() if u])

    processes = []
    for index, directory in enumerate(directories, start=1):
        directory.mkdir(exist_ok=True)
        uuid = issued[directory]
        if uuid and known is not None and uuid not in known:
            log(f"worker {index}: its identity {uuid} is not in this database (reset or "
                f"restored since); it will be issued a new one")
            uuid = None

        # Symlinked, not copied: the data directory is gigabytes and every
        # worker reads the same bytes.
        link = directory / "data"
        if link.is_symlink() or link.exists():
            link.unlink()
        link.symlink_to(data)

        key = worker_key(directory / "contribute.txt")
        settings = write_contribute_settings(directory, args, api_url, uuid, key)
        log_path = directory / "contribute.log"
        handle = log_path.open("a", buffering=1)
        handle.write(f"\n--- started {time.strftime('%Y-%m-%d %H:%M:%S')} ---\n")

        processes.append(
            subprocess.Popen(
                [str(binary), "contribute", str(settings.name)],
                cwd=directory,
                stdout=handle,
                stderr=subprocess.STDOUT,
            )
        )
        kind = "under an API key" if (args.api_key or key) else "anonymous"
        log(f"worker {index} ({kind}): {directory} (log: {log_path})")

    return processes


def stop_contributors(processes: List[subprocess.Popen]) -> None:
    for process in processes:
        if process.poll() is None:
            process.terminate()
    deadline = time.time() + 10
    for process in processes:
        remaining = max(0.0, deadline - time.time())
        try:
            process.wait(timeout=remaining)
        except subprocess.TimeoutExpired:
            process.kill()


# --- the run ----------------------------------------------------------------


def run_seed(args, api_url: str, magpie_root: Path, floor: str) -> None:
    command = [
        sys.executable, str(REPO_ROOT / "scripts" / "seed.py"),
        "--api", api_url,
        "--job-type", args.job_type,
        "--job-name", f"dev {args.job_type.replace('_', ' ')}",
        "--magpie-root", str(magpie_root),
        "--username", args.username,
        "--password", args.password,
        "--email", args.email,
        # The job's own floor, not just the server's: a job stores the floor it
        # was created under, so leaving it implicit would let one created
        # earlier keep declining this build.
        "--min-magpie-version", floor,
    ]
    for flag, value in (("--tarball-date", args.tarball_date), ("--git-ref", args.git_ref),
                        ("--lexicon", args.lexicon), ("--variant", args.variant)):
        if value:
            command += [flag, str(value)]
    # A fresh database gets the whole set: a job of every type at equal
    # shares, and contributor accounts whose keys the keyed workers run under.
    keys_file = Path(args.workdir).expanduser().resolve() / ".contributor-keys.json"
    if args.reset_db:
        # Six jobs at equal shares; a cap every pairs job reaches in a dev
        # session, so all three finish and their players can be rated.
        command += ["--all-job-types", "--allocation", str(100 // 6), "--max-units", "2000",
                    "--contributors", str(len(KEYED_WORKERS)), "--keys-out", str(keys_file)]
    if subprocess.run(command, cwd=REPO_ROOT).returncode != 0:
        fail("seeding failed; the stack is still up, so fix and re-run with --no-up")
    if args.reset_db:
        give_workers_keys(args, api_url, keys_file)


def give_workers_keys(args, api_url: str, keys_file: Path) -> None:
    """Each keyed worker's settings get a contributor's new key; the file the
    seed wrote them to is removed once they are in place."""
    keys = json.loads(keys_file.read_text())
    keys_file.unlink()
    workdir = keys_file.parent
    for index, entry in zip(KEYED_WORKERS, keys):
        directory = workdir / f"worker-{index:02d}"
        directory.mkdir(parents=True, exist_ok=True)
        write_contribute_settings(directory, args, api_url, None, entry["key"])
        log(f"worker {index} runs as {entry['username']}")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )

    contributors = parser.add_argument_group("contributors (always real MAGPIE)")
    contributors.add_argument("-w", "--workers", type=int, default=4,
                              help="MAGPIE contributor processes to run (default: %(default)s). "
                                   "Workers 1 and 2 are anonymous; 3 and 4 run as the contributor "
                                   "accounts --reset-db makes, under their API keys (anonymous "
                                   "until a reset has made them)")
    contributors.add_argument("--threads", type=int, default=4,
                              help="threads per contributor (default: %(default)s)")
    contributors.add_argument("--max-tasks", type=int, default=0,
                              help="tasks each contributor runs before exiting; "
                                   "0 runs until stopped (default: %(default)s)")
    contributors.add_argument("--idle-wait", type=int, default=5,
                              help="seconds a contributor waits when there is no work "
                                   "(default: %(default)s)")
    contributors.add_argument("--api-key", default=os.environ.get("BIRDTEST_API_KEY"),
                              help="contribute under an account instead of anonymously")
    contributors.add_argument("--magpie", default=os.environ.get("MAGPIE_BIN", "../MAGPIE/bin/magpie"),
                              help="MAGPIE binary (default: %(default)s, or $MAGPIE_BIN)")
    contributors.add_argument("--magpie-data",
                              default=os.environ.get("MAGPIE_DATA_PATH", "../MAGPIE/data"),
                              help="MAGPIE data directory (default: %(default)s, "
                                   "or $MAGPIE_DATA_PATH)")
    # Under the repo root whatever the working directory: `.gitignore` covers
    # only the root's `.dev-workers/`, and a contribute.txt holding an API key
    # written under frontend/ was not ignored (the audit's pass 23).
    contributors.add_argument("--workdir", default=str(REPO_ROOT / ".dev-workers"),
                              help="where per-worker directories live (default: %(default)s)")
    contributors.add_argument("--reset-workers", action="store_true",
                              help="delete worker directories first, so each starts as a "
                                   "brand-new anonymous contributor (the keyed workers lose their "
                                   "keys until the next --reset-db)")

    stack = parser.add_argument_group("the stack")
    stack.add_argument("--web-port", type=int, default=int(os.environ.get("WEB_PORT", 5173)))
    stack.add_argument("--backend-port", type=int, default=int(os.environ.get("BACKEND_PORT", 8080)))
    stack.add_argument("--no-up", action="store_true",
                       help="assume the stack is already running")
    stack.add_argument("--rebuild", action="store_true",
                       help="rebuild images before starting")
    stack.add_argument("--hot-reload", action="store_true",
                       help="also run the Vite dev server (compose profile 'dev')")
    stack.add_argument("--reset-db", action="store_true",
                       help="drop the database's schema first, and let the backend rebuild it: "
                            "needed after a schema change, since the one migration is edited "
                            "in place until release (its data goes; the MinIO bucket is kept). "
                            "The fresh database is seeded, on --lexicon, with six jobs at equal "
                            "shares -- games, opening racks, leave generation and three game-pairs "
                            "jobs among three players, which a rating pool can rate once they are "
                            "done -- and two contributor accounts whose keys workers 3 and 4 run "
                            "under")
    stack.add_argument("--down", action="store_true",
                       help="stop the stack on exit instead of leaving it up")
    stack.add_argument("--health-timeout", type=int, default=180,
                       help="seconds to wait for the backend (default: %(default)s)")
    stack.add_argument("--min-magpie-version", default=os.environ.get("MIN_MAGPIE_VERSION"),
                       help="fleet-wide version floor (default: the version your MAGPIE "
                            "checkout reports, so your own build can contribute)")

    seeding = parser.add_argument_group("seeding")
    seeding.add_argument("--no-seed", action="store_true",
                         help="skip seeding; use when the stack already has an active job")
    seeding.add_argument("--job-type", default="game_pairs",
                         choices=["game_pairs", "games", "opening_rack"],
                         help="job to create and activate (default: %(default)s)")
    seeding.add_argument("--lexicon", default="CSW24",
                         help="lexicon for the seeded jobs and their players (default: %(default)s)")
    seeding.add_argument("--variant", default=None, choices=["classic", "wordsmog"])
    seeding.add_argument("--tarball-date", default=None,
                         help="MAGPIE-DATA tarball YYYYMMDD (default: the DATA_VERSION your "
                              "MAGPIE checkout installed, so the server's digests match "
                              "the bytes your workers actually have)")
    seeding.add_argument("--git-ref", default=None, help="ref to resolve the tarball at")
    seeding.add_argument("--username", default="dev")
    seeding.add_argument("--password", default="devpassword123!")
    seeding.add_argument("--email", default="dev@example.invalid")

    signin = parser.add_argument_group("signing in")
    signin.add_argument("--login-as", default=None, metavar="USER",
                        help="open the site signed in as this account (default: the seeded "
                             "--username, the admin)")
    signin.add_argument("--no-login", action="store_true",
                        help="open the site signed out")
    parser.add_argument("--no-browser", action="store_true",
                        help="do not open a browser (for SSH sessions and CI)")
    return parser


def main() -> int:
    load_env_file()
    args = build_parser().parse_args()
    binary, data = resolve_magpie(args)
    magpie_root = binary.parent.parent

    site_url = f"http://localhost:{args.web_port}"
    api_url = f"http://localhost:{args.backend_port}"

    floor = args.min_magpie_version or magpie_version(magpie_root) or "0.0.0"
    os.environ.update({
        "WEB_PORT": str(args.web_port),
        "BACKEND_PORT": str(args.backend_port),
        # Compose recreates the backend when this changes, so switching floors
        # between runs takes effect without a manual `down`.
        "MIN_MAGPIE_VERSION": floor,
        # The backend runs this same MAGPIE for every wordmap, rack info table
        # and leave-generation KLV, and refuses to start without one. Mounting
        # the checkout the contributors run from is what keeps the server's
        # builder and the fleet's the same binary -- which is the whole point:
        # a hash built by one and checked by the other has to come from the
        # same build.
        "MAGPIE_ROOT": str(magpie_root),
    })
    log(f"version floor {floor} (your MAGPIE build reports "
        f"{magpie_version(magpie_root) or 'an unknown version'})")

    if args.reset_db:
        reset_database()
    # Before seeding, which may give workers keys to keep.
    workdir = Path(args.workdir).expanduser().resolve()
    if args.reset_workers and workdir.exists():
        shutil.rmtree(workdir)
    if not args.no_up:
        # --remove-orphans: containers of services the compose file no longer
        # has (a `worker` service, once) are removed rather than warned about.
        up = ["up", "-d", "--remove-orphans"]
        if args.hot_reload:
            up = ["--profile", "dev", *up]
        if args.rebuild:
            up.append("--build")
        log("starting the stack")
        if compose(up, check=False).returncode != 0:
            explain_failed_start()
            return 1

    log(f"waiting for {api_url}/health")
    wait_for_health(f"{api_url}/health", args.health_timeout)

    if args.no_seed:
        log("skipping seeding")
    else:
        run_seed(args, api_url, magpie_root, floor)

    processes = start_contributors(args, binary, data, api_url)
    # Keyed by the worker number shown at startup, so an exit report names the
    # directory whose log to read rather than a shifting position in a list.
    labelled = {index: process for index, process in enumerate(processes, start=1)}

    def last_line(index: int) -> str:
        log_path = Path(args.workdir).expanduser().resolve() / f"worker-{index:02d}" / "contribute.log"
        try:
            lines = [line for line in log_path.read_text().splitlines() if line.strip()]
        except OSError:
            return "(no log)"
        return lines[-1] if lines else "(no output)"
    log(f"{args.workers} MAGPIE contributor(s) running against {api_url}")

    # Signed in through the stack's dev-only sign-in (DEV_LOGIN in
    # docker-compose.yml), which sets the session and sends the browser on.
    open_url = site_url
    if not args.no_login:
        user = args.login_as or args.username
        open_url = f"{site_url}/api/dev/login?username={quote(user)}&next=/"
        log(f"signed in as {user}: {open_url}")
    if not args.no_browser:
        webbrowser.open(open_url)
    log(f"birdtest is at {site_url} — Ctrl-C to stop the contributors")

    stopping = False

    def handle_signal(_signum, _frame):
        nonlocal stopping
        stopping = True

    signal.signal(signal.SIGINT, handle_signal)
    signal.signal(signal.SIGTERM, handle_signal)

    try:
        while not stopping:
            # A contributor that exits on its own (--max-tasks, or a fatal
            # error) is worth surfacing rather than silently leaving a smaller
            # fleet running.
            for index, process in list(labelled.items()):
                code = process.poll()
                if code is not None:
                    # MAGPIE exits 0 on errors too: its last line says why.
                    log(f"worker {index} exited with status {code}: {last_line(index)}")
                    del labelled[index]
                    processes.remove(process)
            if not processes:
                log("no contributors left running")
                break
            time.sleep(1)
    finally:
        stop_contributors(processes)
        log("contributors stopped")
        if args.down:
            log("stopping the stack")
            compose(["down"], check=False)
        else:
            log(f"the stack is still up ({site_url}); `docker compose down` when you are done")
    return 0


if __name__ == "__main__":
    sys.exit(main())
