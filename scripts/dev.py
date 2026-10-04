#!/usr/bin/env python3
"""Bring up birdtest locally with real MAGPIE contributors, and open a browser.

One command: start the stack, wait for it, seed it, launch N `magpie
contribute` processes -- in the background, or with --worker-windows each in
its own terminal window, so one can be stopped and restarted by hand -- and
open the site.

It starts with no job. Each job flag adds one, and they stack:

    scripts/dev.py --leavegen-job --pairs-job

Every job runs on --lexicon and the english distribution. Append `_ab` to a
job flag (--leavegen-job_ab, say) to run that job on MAGPIE's two-letter test
data instead (`english_ab`, eight possible full racks), where it finishes in
minutes. It is tier 6's setup with the assertions removed, and it calls the
same `scripts/seed.py`,
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
import fcntl
import json
import os
import re
import shlex
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

    # The data of the checkout the binary is in, unless named: pointed at
    # another MAGPIE, the data went on being the default one's.
    data = (Path(args.magpie_data).expanduser() if args.magpie_data
            else binary.resolve().parent.parent / "data")
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


# Where dev.py serves MAGPIE's two-letter test data from (start_data_standin).
DATA_STANDIN_PORT = 8482


def docker_bridge_address() -> Optional[str]:
    """The host's address on Docker's default bridge: what a container reaches
    as `host.docker.internal` (docker-compose.yml's `host-gateway`), and not
    reachable from the rest of the network the way 0.0.0.0 would be."""
    result = subprocess.run(
        ["docker", "network", "inspect", "bridge", "--format",
         "{{(index .IPAM.Config 0).Gateway}}"],
        capture_output=True, text=True)
    address = result.stdout.strip()
    return address if result.returncode == 0 and address else None


def install_small_data(magpie_root: Path, data: Path) -> None:
    """Copies the two-letter test files into the data directory every worker
    shares, where MAGPIE looks for a job's lexicon and distribution by name.
    Three files of a few kilobytes, under names no MAGPIE-DATA release uses;
    an existing file is left alone."""
    import e2e_magpie
    for inside, source in e2e_magpie.SMALL_FILES:
        target = data / inside
        if not target.exists():
            shutil.copyfile(magpie_root / source, target)
            log(f"copied {source} into {target.parent} for the two-letter test data")


def start_data_standin(magpie_root: Path) -> None:
    """Serves MAGPIE's two-letter test data (`english_ab`, `CSW21_ab`: 8 full
    racks) as a MAGPIE-DATA version the admin import can fetch, by standing in
    for GitHub: the one version and branch below are answered here, and every
    other request goes on to GitHub, so the real import still works. It is
    tier 6's stand-in (`e2e_magpie.GitHubStandIn`), and it is what makes a
    leave-generation job that finishes in minutes possible locally --
    JOURNEYS.md walks through one. Lives as long as this process."""
    import e2e_magpie
    try:
        tarball = e2e_magpie.small_tarball(magpie_root)
    except e2e_magpie.Failure as err:
        log(f"no two-letter test data to serve ({err}); imports go straight to GitHub")
        return
    address = docker_bridge_address()
    if not address:
        log("could not find Docker's bridge address; imports go straight to GitHub")
        return
    try:
        e2e_magpie.GitHubStandIn((address, DATA_STANDIN_PORT), tarball)
    except OSError as err:
        log(f"could not serve the two-letter test data on {address}:{DATA_STANDIN_PORT} "
            f"({err}); imports go straight to GitHub")
        return
    base = f"http://host.docker.internal:{DATA_STANDIN_PORT}"
    os.environ["BIRDTEST_GITHUB_API_URL"] = f"{base}/api"
    os.environ["BIRDTEST_GITHUB_RAW_URL"] = f"{base}/raw"
    log(f"MAGPIE's two-letter test data can be imported as version {e2e_magpie.SMALL_DATE}, "
        f"branch {e2e_magpie.SMALL_REF} (Input data, while dev.py runs)")


# Every worker maps its rack info table rather than reading it in: the workers
# share one data directory, so mapped they share one ~1.9 GB copy in the page
# cache instead of holding one each. (Newer MAGPIE does this unasked; named
# here for a checkout that predates that.)
CONTRIBUTE_FLAGS = ["-ritmmap", "true"]

# In the worker directory; see lock_workdir.
LOCK_FILE = ".dev.lock"

# How often a running dev.py looks for queued derived files.
DERIVED_CHECK_SECS = 15


def queued_derived_files() -> Optional[int]:
    """How many wordmaps and rack info tables are waiting to be built, or None
    when the stack's database cannot be asked."""
    result = subprocess.run(
        ["docker", "compose", "exec", "-T", "postgres", "psql", "-U", "birdtest", "-d", "birdtest",
         "-Atq", "-c", "SELECT COUNT(*) FROM derived_data WHERE state = 'pending'"],
        cwd=REPO_ROOT, capture_output=True, text=True)
    if result.returncode != 0:
        return None
    try:
        return int(result.stdout.strip())
    except ValueError:
        return None


def build_derived_files(args) -> None:
    """Builds what the derived-file queue holds, as production's scheduled
    task does. Nothing else in the stack builds a wordmap, so without this a
    job whose players use one -- the default -- is never dispatched."""
    queued = queued_derived_files()
    if not queued:
        return
    log(f"building {queued} queued wordmap / rack info table file(s)")
    run = ["run", "--rm"]
    if args.rebuild:
        run.append("--build")
    if compose([*run, "derived-builder"], check=False).returncode != 0:
        log("the derived-file builder failed; jobs that need its files wait until "
            "`docker compose run --rm derived-builder` succeeds (see /admin/derived-data)")


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


# The terminals a worker's window can be opened in, tried in order, and how
# each is told a window title and a command. `x-terminal-emulator` is Debian's
# alternatives link to whichever is installed, and takes xterm's flags.
TERMINALS = (
    ("gnome-terminal", lambda title, command: ["--title", title, "--", *command]),
    ("konsole", lambda title, command: ["-p", f"tabtitle={title}", "-e", *command]),
    ("xfce4-terminal", lambda title, command: ["--title", title, "-x", *command]),
    ("kitty", lambda title, command: ["--title", title, *command]),
    ("alacritty", lambda title, command: ["--title", title, "-e", *command]),
    ("x-terminal-emulator", lambda title, command: ["-T", title, "-e", *command]),
    ("xterm", lambda title, command: ["-T", title, "-e", *command]),
)

# Run in each worker's window. It loops so a stopped worker can be started
# again from the same window, keeping its identity: Ctrl-C stops MAGPIE (the
# trap keeps the script alive for the prompt), Enter restarts it, and a second
# Ctrl-C or closing the window leaves it stopped -- the prompt has its own trap,
# since bash resumes a `read` a trap interrupted. The pid it records is how
# dev.py follows, and stops, a window whose terminal launcher returned at once.
WORKER_SCRIPT = """#!/usr/bin/env bash
cd "$(dirname "$0")"
echo $$ > window.pid
trap 'rm -f window.pid' EXIT
trap 'echo' INT
title={title}
printf '\\033]0;%s\\007' "$title"
while true; do
    echo "--- started $(date '+%Y-%m-%d %H:%M:%S') ---" >> contribute.log
    echo "$title: Ctrl-C stops it"
    {binary} contribute contribute.txt {flags} 2>&1 | tee -a contribute.log
    status=${{PIPESTATUS[0]}}
    echo
    trap 'exit 0' INT
    read -r -p "MAGPIE exited ($status). Enter restarts it; Ctrl-C closes this window. " || break
    trap 'echo' INT
done
"""


def lock_workdir(workdir: Path):
    """Held for the whole run: a second dev.py on the same worker directories
    ran a second MAGPIE in each, under the same identities, beside the first's.
    The lock goes with the process, however it ends."""
    workdir.mkdir(parents=True, exist_ok=True)
    handle = (workdir / LOCK_FILE).open("a+")
    try:
        fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        handle.seek(0)
        holder = handle.read().strip() or "unknown"
        fail(f"another dev.py (pid {holder}) is running workers in {workdir}; stop it first, "
             "or give this one its own --workdir")
    handle.seek(0)
    handle.truncate()
    handle.write(f"{os.getpid()}\n")
    handle.flush()
    return handle


def find_terminal() -> Optional[tuple]:
    """The first terminal emulator on PATH, or None -- including when there is
    no display to open a window on (an SSH session)."""
    if not (os.environ.get("DISPLAY") or os.environ.get("WAYLAND_DISPLAY")):
        return None
    for name, flags in TERMINALS:
        path = shutil.which(name)
        if path:
            return path, flags
    return None


class WorkerWindow:
    """A worker in its own terminal window, standing in for the Popen a
    background worker has: poll, terminate, kill and wait, by the pid the
    window's script records."""

    def __init__(self, pid_file: Path):
        self.pid_file = pid_file
        self.pid: Optional[int] = None
        self.returncode: Optional[int] = None

    def started(self, timeout: float) -> bool:
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                self.pid = int(self.pid_file.read_text().strip())
                return True
            except (OSError, ValueError):
                time.sleep(0.2)
        return False

    def poll(self) -> Optional[int]:
        if self.returncode is None and self.pid is not None:
            try:
                os.kill(self.pid, 0)
            except ProcessLookupError:
                self.returncode = 0
            except PermissionError:
                pass
        return self.returncode

    def _signal(self, sig: int) -> None:
        if self.poll() is not None:
            return
        try:
            # The window's whole process group -- the script, MAGPIE and tee --
            # unless the terminal left it in ours, which would signal this
            # script too.
            group = os.getpgid(self.pid)
            if group == os.getpgrp():
                os.kill(self.pid, sig)
            else:
                os.killpg(group, sig)
        except ProcessLookupError:
            pass

    def terminate(self) -> None:
        self._signal(signal.SIGTERM)

    def kill(self) -> None:
        self._signal(signal.SIGKILL)

    def wait(self, timeout: float) -> int:
        deadline = time.time() + timeout
        while self.poll() is None:
            if time.time() >= deadline:
                raise subprocess.TimeoutExpired(str(self.pid_file), timeout)
            time.sleep(0.2)
        return self.returncode


def open_worker_window(terminal: tuple, directory: Path, binary: Path, title: str) -> WorkerWindow:
    script = directory / "run.sh"
    script.write_text(WORKER_SCRIPT.format(title=shlex.quote(title),
                                           binary=shlex.quote(str(binary)),
                                           flags=shlex.join(CONTRIBUTE_FLAGS)))
    script.chmod(0o755)
    pid_file = directory / "window.pid"
    pid_file.unlink(missing_ok=True)
    path, flags = terminal
    subprocess.Popen([path, *flags(title, ["bash", str(script)])], cwd=directory,
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                     stderr=subprocess.DEVNULL, start_new_session=True)
    return WorkerWindow(pid_file)


def start_contributors(args, binary: Path, data: Path, api_url: str) -> list:
    workdir = Path(args.workdir).expanduser().resolve()
    workdir.mkdir(parents=True, exist_ok=True)

    # Each worker's issued identity, kept only while the database still knows
    # it: after --reset-db, or a restore, the server answers an identity it
    # never issued with 401, and MAGPIE gives up at once -- with status 0.
    directories = [workdir / f"worker-{index:02d}" for index in range(1, args.workers + 1)]
    issued = {d: issued_uuid(d / "contribute.txt") for d in directories}
    known = uuids_the_database_knows([u for u in issued.values() if u])

    terminal = find_terminal() if args.worker_windows else None
    if args.worker_windows and terminal is None:
        log("no display or terminal emulator found: workers run in the background "
            "(their output goes to each contribute.log)")

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
        kind = "under an API key" if (args.api_key or key) else "anonymous"
        log_path = directory / "contribute.log"
        if terminal:
            window = open_worker_window(terminal, directory, binary,
                                        f"birdtest worker {index} ({kind})")
            if not window.started(timeout=15):
                fail(f"worker {index}'s window did not start ({terminal[0]}); "
                     "re-run without --worker-windows")
            processes.append(window)
            log(f"worker {index} ({kind}): its own window, in {directory}")
            continue
        handle = log_path.open("a", buffering=1)
        handle.write(f"\n--- started {time.strftime('%Y-%m-%d %H:%M:%S')} ---\n")

        processes.append(
            subprocess.Popen(
                [str(binary), "contribute", str(settings.name), *CONTRIBUTE_FLAGS],
                cwd=directory,
                stdout=handle,
                stderr=subprocess.STDOUT,
            )
        )
        log(f"worker {index} ({kind}): {directory} (log: {log_path})")

    return processes


def stop_contributors(processes: list) -> None:
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


# The jobs dev.py can start with: its flag, seed.py's name for the job, and
# what the job is. Each flag also comes with `_ab` appended, which runs the job
# on the two-letter test data rather than the main data.
DEV_JOBS = (
    ("--leavegen-job", "leave_generation",
     "a leave-generation job of six generations, targets 100, 200, 500, 1000, 1000, 1000"),
    ("--opening-rack-job", "opening_rack", "an opening-rack job, every play of every rack ranked"),
    ("--games-job", "games",
     "a games job, static equity against static score, saving every position"),
    ("--pairs-job", "game_pairs",
     "a game-pairs job, static equity against static score, saving the positions where each "
     "pair first diverges"),
    ("--sim-games-job", "games_sim",
     "a games job, a 2-ply simmer against a 1-ply one, saving every position"),
    ("--sim-pairs-job", "game_pairs_sim",
     "a game-pairs job, a 2-ply simmer against a 1-ply one, saving the positions where each "
     "pair first diverges"),
)
# (dest, seed.py's job name, on the two-letter data?) for every job flag.
DEV_JOB_FLAGS = tuple(
    (flag[2:].replace("-", "_") + suffix, job + suffix, bool(suffix))
    for flag, job, _ in DEV_JOBS for suffix in ("", "_ab")
)


def requested_jobs(args) -> List[tuple]:
    """(seed.py's job name, on the two-letter data?) for each job flag given."""
    return [(job, small) for flag, job, small in DEV_JOB_FLAGS if getattr(args, flag)]


def run_seed(args, api_url: str, magpie_root: Path, floor: str) -> None:
    command = [
        sys.executable, str(REPO_ROOT / "scripts" / "seed.py"),
        "--api", api_url,
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
    # Only the jobs asked for; new ones share the allocation the active jobs
    # leave free.
    jobs = requested_jobs(args)
    if not jobs:
        command.append("--no-job")
    for job, _ in jobs:
        command += ["--dev-job", job]
    if any(small for _, small in jobs):
        import e2e_magpie
        command += ["--small-tarball-date", e2e_magpie.SMALL_DATE,
                    "--small-git-ref", e2e_magpie.SMALL_REF]
    # A fresh database gets contributor accounts whose keys the keyed workers
    # run under.
    keys_file = Path(args.workdir).expanduser().resolve() / ".contributor-keys.json"
    if args.reset_db:
        command += ["--contributors", str(len(KEYED_WORKERS)), "--keys-out", str(keys_file)]
    if args.no_rit:
        command.append("--no-rit")
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
    contributors.add_argument("--worker-windows", action="store_true",
                              help="run each worker in its own terminal window, where Ctrl-C "
                                   "stops it and Enter restarts it, instead of in the background "
                                   "logging to its contribute.log (ignored with no display)")
    contributors.add_argument("--threads", type=int, default=2,
                              help="threads per contributor (default: %(default)s)")
    contributors.add_argument("--max-tasks", type=int, default=0,
                              help="tasks each contributor runs before exiting; "
                                   "0 runs until stopped (default: %(default)s)")
    contributors.add_argument("--idle-wait", type=int, default=5,
                              help="seconds a contributor waits when there is no work "
                                   "(default: %(default)s)")
    contributors.add_argument("--api-key", default=os.environ.get("BIRDTEST_API_KEY"),
                              help="contribute under an account instead of anonymously")
    contributors.add_argument("--magpie", default=os.environ.get("MAGPIE_BIN", "~/MAGPIE/bin/magpie"),
                              help="MAGPIE binary, in the checkout the backend runs too "
                                   "(default: $MAGPIE_BIN, or %(default)s)")
    contributors.add_argument("--magpie-data", default=os.environ.get("MAGPIE_DATA_PATH"),
                              help="MAGPIE data directory (default: $MAGPIE_DATA_PATH, or the "
                                   "data/ of the --magpie checkout)")
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
    stack.add_argument("--build-threads", type=int,
                       default=int(os.environ.get("MAGPIE_THREADS") or os.cpu_count() or 1),
                       help="threads the server's wordmap / rack info table builder gives "
                            "MAGPIE (default: $MAGPIE_THREADS, or every core: %(default)s)")
    stack.add_argument("--hot-reload", action="store_true",
                       help="also run the Vite dev server (compose profile 'dev')")
    stack.add_argument("--reset-db", action="store_true",
                       help="drop the database's schema first, and let the backend rebuild it: "
                            "needed after a schema change, since the one migration is edited "
                            "in place until release (its data goes; the MinIO bucket is kept). "
                            "The fresh database is seeded with the admin, the input data, the "
                            "jobs the job flags ask for (none without one), and two "
                            "contributor accounts whose keys workers 3 and 4 run under")
    stack.add_argument("--fresh", action="store_true",
                       help="start birdtest as a new deployment does: --reset-db and "
                            "--reset-workers, then no seeding -- no accounts, no data imports, "
                            "no jobs -- and the site opened signed out. The workers run "
                            "anonymously, waiting for a job; how to register and make yourself "
                            "the first admin is printed")
    stack.add_argument("--keep-up", action="store_true",
                       help="leave the stack running on exit instead of stopping it (a stack "
                            "attached to with --no-up is always left running). The database "
                            "and MinIO volumes are kept either way")
    stack.add_argument("--health-timeout", type=int, default=180,
                       help="seconds to wait for the backend (default: %(default)s)")
    stack.add_argument("--min-magpie-version", default=os.environ.get("MIN_MAGPIE_VERSION"),
                       help="fleet-wide version floor (default: the version your MAGPIE "
                            "checkout reports, so your own build can contribute)")

    jobs = parser.add_argument_group(
        "jobs",
        "dev.py starts with no job; each of these adds one, created and activated (or, when an "
        "active one of its name is already running, reused), and they stack. New jobs share "
        "the allocation the active ones leave free")
    for flag, _, what in DEV_JOBS:
        jobs.add_argument(flag, action="store_true",
                          help=f"{what}, on --lexicon and the english distribution")
        # Spelled with a hyphen as well, as every other flag here is.
        jobs.add_argument(f"{flag}_ab", f"{flag}-ab", action="store_true",
                          help="the same job on the two-letter english_ab data (eight "
                               "possible full racks)")

    seeding = parser.add_argument_group("seeding")
    seeding.add_argument("--no-seed", action="store_true",
                         help="skip seeding (the admin, the input data and any job flags)")
    seeding.add_argument("--lexicon", default="CSW24",
                         help="lexicon for the jobs not on english_ab, and their players "
                              "(default: %(default)s)")
    seeding.add_argument("--variant", default=None, choices=["classic", "wordsmog"])
    seeding.add_argument("--no-rit", action="store_true",
                         help="seed the jobs' players without a rack info "
                              "table (~1.9 GB, which the workers map and share, and a few "
                              "minutes' build per worker data directory)")
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


def first_admin_steps(site_url: str) -> str:
    """How to make the first admin on a --fresh site, and how the live site
    differs. There is deliberately no page or endpoint for it."""
    confirm = f"{site_url}/confirm-email?code="
    return f"""
============================================================================
A fresh site has no accounts. To make yourself its first admin:

 1. In the browser, register at {site_url}/register
    Any username; the email can be made up, since nothing is sent locally.

 Leave dev.py running in this terminal: Ctrl-C here stops the whole site.
 For steps 2 and 3, open a NEW terminal window, in the birdtest folder:

      cd {REPO_ROOT}

 2. Confirm the email. Locally it is written to the backend's log instead of
    being sent, and this prints the link it holds; open that in the browser:

      docker compose logs backend | grep -o '{confirm}[A-Za-z0-9%]*' | tail -1

 3. Make the account an admin, with YOUR_NAME replaced by your username
    (keep the quotes):

      docker compose exec postgres psql -U birdtest -d birdtest -c "UPDATE users SET is_admin = true WHERE lower(username) = lower('YOUR_NAME')"

    It prints UPDATE 1 (UPDATE 0: no account has that name). Sign in, or
    reload if you already have: an Admin link appears in the header.

 On the live site:
  - Step 1 is the same, at the site's own address.
  - Step 2: the email really arrives; open the link in it.
  - Step 3 runs from the machine you deploy from, with the stack's AWS
    credentials and Terraform state (README, "Deploying", step 7):

      scripts/prod-sql.sh "UPDATE users SET is_admin = true WHERE lower(username) = lower('YOUR_NAME') RETURNING username"
============================================================================"""


def main() -> int:
    load_env_file()
    parser = build_parser()
    args = parser.parse_args()
    asked = [f"--{flag.replace('_', '-')}" for flag, _, _ in DEV_JOB_FLAGS if getattr(args, flag)]
    if asked and (args.fresh or args.no_seed):
        parser.error(f"{' '.join(asked)} {'is' if len(asked) == 1 else 'are'} seeded, and "
                     f"--{'fresh' if args.fresh else 'no-seed'} seeds nothing")
    if args.fresh:
        if args.login_as:
            parser.error("--fresh makes no accounts, so there is nobody to --login-as")
        # Everything a new deployment has not got: its database, the
        # identities and keys workers kept from earlier runs, and the seed.
        args.reset_db = args.reset_workers = args.no_seed = args.no_login = True
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
        # MAGPIE's conversions run on as many threads as they are given, and
        # the builder runs alone before any job that needs its files is handed
        # out: every core finishes it soonest. (Compose's default is 2.)
        "MAGPIE_THREADS": str(max(1, args.build_threads)),
    })
    log(f"version floor {floor} (your MAGPIE build reports "
        f"{magpie_version(magpie_root) or 'an unknown version'})")

    # Before anything touches the stack: a second run's --reset-db would drop
    # the database under the first one's workers too.
    workdir = Path(args.workdir).expanduser().resolve()
    lock = lock_workdir(workdir)  # noqa: F841 -- held until exit

    # Before the stack starts: the backend is pointed at it through the
    # environment compose reads.
    if not args.no_up:
        start_data_standin(magpie_root)
    small_jobs = any(small for _, small in requested_jobs(args))
    if os.environ.get("BIRDTEST_GITHUB_API_URL") or small_jobs:
        install_small_data(magpie_root, data)
    if small_jobs and args.no_up:
        log("the english_ab jobs import the two-letter test data through the stand-in of the "
            "dev.py that started the stack; if that one has stopped and the data is not "
            "imported yet, seeding fails: restart without --no-up")

    if args.reset_db:
        reset_database()
    # Before seeding, which may give workers keys to keep. Everything but the
    # lock, which this run holds.
    if args.reset_workers:
        for entry in workdir.iterdir():
            if entry.name == LOCK_FILE:
                continue
            if entry.is_dir() and not entry.is_symlink():
                shutil.rmtree(entry)
            else:
                entry.unlink()
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

    if args.fresh:
        log("fresh start: no accounts, data imports or jobs")
    elif args.no_seed:
        log("skipping seeding")
    else:
        run_seed(args, api_url, magpie_root, floor)
    build_derived_files(args)

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
    log(f"birdtest is at {site_url} — Ctrl-C to stop the contributors"
        + ("" if args.keep_up or args.no_up else " and the stack"))
    if args.fresh:
        # Last, so it is what the terminal is left showing.
        # Unprefixed, so a command copied whole is only the command.
        print(first_admin_steps(site_url), flush=True)

    stopping = False

    def handle_signal(_signum, _frame):
        nonlocal stopping
        stopping = True

    signal.signal(signal.SIGINT, handle_signal)
    signal.signal(signal.SIGTERM, handle_signal)

    # A job activated from the admin pages while this runs queues its files
    # too, so the queue is drained as long as the stack is being run.
    next_derived_check = time.monotonic() + DERIVED_CHECK_SECS
    try:
        while not stopping:
            if time.monotonic() >= next_derived_check:
                build_derived_files(args)
                next_derived_check = time.monotonic() + DERIVED_CHECK_SECS
            # A contributor that exits on its own (--max-tasks, or a fatal
            # error) is worth surfacing rather than silently leaving a smaller
            # fleet running.
            for index, process in list(labelled.items()):
                code = process.poll()
                if code is not None:
                    if isinstance(process, WorkerWindow):
                        log(f"worker {index}'s window closed")
                    else:
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
        # `down` without -v: the database and MinIO volumes stay, so the next
        # run picks up where this one left off.
        if args.keep_up or args.no_up:
            log(f"the stack is still up ({site_url}); `docker compose down` when you are done")
            if os.environ.get("BIRDTEST_GITHUB_API_URL") and not args.no_up:
                log("its data imports went through this dev.py, which has stopped: they fail "
                    "until the stack is restarted")
        else:
            log("stopping the stack (its data is kept; --keep-up leaves it running)")
            compose(["down"], check=False)
    return 0


if __name__ == "__main__":
    sys.exit(main())
