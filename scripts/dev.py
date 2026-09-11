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

Each contributor gets its own directory, because `magpie contribute` reads and
writes `settings.txt` and `contribute.txt` in its working directory: sharing
one would race on both files and collapse every worker onto a single identity.
The MAGPIE data directory is symlinked rather than copied, so N workers cost
nothing but their own settings files.
"""

import argparse
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

import requests

REPO_ROOT = Path(__file__).resolve().parent.parent


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
            "      Build one (`make magpie` in your MAGPIE checkout) and pass --magpie, "
            "or set MAGPIE_BIN.\n"
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
    and production sets it to a real release (0.1.0 by default). Locally the
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


def write_contribute_settings(directory: Path, args, api_url: str) -> Path:
    """One `contribute.txt` per worker.

    Settings live in a file rather than on the command line so an API key stays
    out of shell history and `ps` output. MAGPIE appends the server-minted
    `uuid` to this file on first contact, which is why each worker needs its
    own: they would otherwise overwrite each other's identity.
    """
    settings = directory / "contribute.txt"
    if settings.exists() and not args.reset_workers:
        return settings

    lines = [
        "# Written by scripts/dev.py. Delete this file (or pass --reset-workers)",
        "# to make this worker forget the identity the server assigned it.",
        f"server   {api_url}",
        f"threads  {args.threads}",
        f"maxtasks {args.max_tasks}",
        f"idlewait {args.idle_wait}",
    ]
    if args.api_key:
        lines.append(f"apikey   {args.api_key}")
    settings.write_text("\n".join(lines) + "\n")
    return settings


def start_contributors(args, binary: Path, data: Path, api_url: str) -> List[subprocess.Popen]:
    workdir = Path(args.workdir).expanduser().resolve()
    if args.reset_workers and workdir.exists():
        shutil.rmtree(workdir)
    workdir.mkdir(parents=True, exist_ok=True)

    processes = []
    for index in range(1, args.workers + 1):
        directory = workdir / f"worker-{index:02d}"
        directory.mkdir(exist_ok=True)

        # Symlinked, not copied: the data directory is gigabytes and every
        # worker reads the same bytes.
        link = directory / "data"
        if link.is_symlink() or link.exists():
            link.unlink()
        link.symlink_to(data)

        settings = write_contribute_settings(directory, args, api_url)
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
        log(f"worker {index}: {directory} (log: {log_path})")

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
    if subprocess.run(command, cwd=REPO_ROOT).returncode != 0:
        fail("seeding failed; the stack is still up, so fix and re-run with --no-up")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )

    contributors = parser.add_argument_group("contributors (always real MAGPIE)")
    contributors.add_argument("-w", "--workers", type=int, default=2,
                              help="MAGPIE contributor processes to run (default: %(default)s)")
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
    contributors.add_argument("--workdir", default=".dev-workers",
                              help="where per-worker directories live (default: %(default)s)")
    contributors.add_argument("--reset-workers", action="store_true",
                              help="delete worker directories first, so each starts as a "
                                   "brand-new anonymous contributor")

    stack = parser.add_argument_group("the stack")
    stack.add_argument("--web-port", type=int, default=int(os.environ.get("WEB_PORT", 5173)))
    stack.add_argument("--backend-port", type=int, default=int(os.environ.get("BACKEND_PORT", 8080)))
    stack.add_argument("--no-up", action="store_true",
                       help="assume the stack is already running")
    stack.add_argument("--rebuild", action="store_true",
                       help="rebuild images before starting")
    stack.add_argument("--hot-reload", action="store_true",
                       help="also run the Vite dev server (compose profile 'dev')")
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
    seeding.add_argument("--lexicon", default=None, help="lexicon for the seeded job")
    seeding.add_argument("--variant", default=None, choices=["classic", "wordsmog"])
    seeding.add_argument("--tarball-date", default=None,
                         help="MAGPIE-DATA tarball YYYYMMDD (default: the DATA_VERSION your "
                              "MAGPIE checkout installed, so the server's digests match "
                              "the bytes your workers actually have)")
    seeding.add_argument("--git-ref", default=None, help="ref to resolve the tarball at")
    seeding.add_argument("--username", default="dev")
    seeding.add_argument("--password", default="devpassword123!")
    seeding.add_argument("--email", default="dev@example.invalid")

    parser.add_argument("--no-browser", action="store_true",
                        help="do not open a browser (for SSH sessions and CI)")
    return parser


def main() -> int:
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
    })
    log(f"version floor {floor} (your MAGPIE build reports "
        f"{magpie_version(magpie_root) or 'an unknown version'})")

    if not args.no_up:
        up = ["up", "-d"]
        if args.hot_reload:
            up = ["--profile", "dev", *up]
        if args.rebuild:
            up.append("--build")
        log("starting the stack")
        compose(up)

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
    log(f"{args.workers} MAGPIE contributor(s) running against {api_url}")

    if not args.no_browser:
        webbrowser.open(site_url)
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
                    log(f"worker {index} exited with status {code}")
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
