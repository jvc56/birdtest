#!/usr/bin/env python3
"""Tier 6 (TESTING.md): real `magpie contribute` tasks against a real stack.

Each case is selectable with `--cases` (default: every M case):

- `M-1` `games`: the result lands and is credited.
- `M-2` `game_pairs`: pairs are counted (the pentanomial invariants are
  enforced by the server's plausibility checks on every accepted result).
- `M-3` `opening_rack`, static and simming: a best move per rack, and the
  simulated statistics (win%, per-ply stats) are stored, not just move, score
  and equity. The static player asks for a wordmap, so its job waits for the
  derived-file builder, and MAGPIE builds its own copy and finds it agrees.
- `M-4` `leave_generation`: full-rack occurrences are staged, a merge folds
  them into the generation's progress, and nothing is written into MAGPIE's
  data directory. Real English, so creating the job seeds a 3.2M-rack
  universe: the slow one.
- `M-5` A worker whose data does not match the job's digests declines with
  `missing_data`, the gap reaches `worker_data_gaps`, and nothing is stored.
- `M-6` A worker below a job's version floor never runs it. The real server
  filters on the floor before dispatching, so the worker is told
  `magpie_too_old` and claims nothing; and against a server that hands the job
  out anyway (a proxy raising the floor on the way back), MAGPIE declines it
  with `magpie_version`.
- `M-7` `capture_positions` on a real game stores positions whose CGP loads
  back into MAGPIE.
- `M-9` Two contributors at once: every task has its own seed, the seeds tile
  the space with no gap, and each worker's claims are its own.
- `M-10` A `use_rit` job is not dispatched until its rack info table is
  built; then a real contributor builds a table whose SHA-256 is the one the
  server recorded, and plays with it. Runs on MAGPIE's two-letter test
  distribution (`CSW21_ab`, `english_ab`), imported from a tarball this script
  serves as a GitHub stand-in, so the table is small and quick.
- `M-11` A worker whose wordmap does not match the hash the server recorded
  declines with `derived_mismatch`, and both digests reach
  `worker_data_gaps`.

And one case that is not a test: `capture` runs one job of each type through
`scripts/capture_contract.py`'s recording proxy and writes the contract
fixtures into `--capture-out` (TESTING.md, "4. Contract").

Expects the compose stack (`postgres`, `minio`, `backend`) to be up, with
MAIL_BACKEND=console, and a built MAGPIE whose `data/` is a real
`download_data.sh` install. Seeding reuses `scripts/seed.py`, so the admin,
the input-data import and the player configs go through the real API.

    docker compose up -d --build --wait postgres minio minio-init backend
    python3 scripts/e2e_magpie.py --magpie ../MAGPIE/bin/magpie --magpie-root ../MAGPIE

On a machine without the images, `scripts/e2e_magpie_native.sh` brings up the
same stack natively and runs this against it.

`M-10` and `capture` need the backend's GITHUB_API_URL and GITHUB_RAW_URL to
point at `http://<host>:<--github-fixture-port>/api` and `/raw`: this script
serves the small tarball there, and forwards every other request to GitHub.
Without that they fail, naming the setting.

Only for a disposable stack: it creates and activates jobs, and deactivates
every job it did not create. Each case deletes the jobs it made and the
directories its workers used.
"""

import argparse
import hashlib
import io
import re
import shutil
import signal
import subprocess
import sys
import tarfile
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Callable, Dict, List, Optional

import requests

sys.path.insert(0, str(Path(__file__).resolve().parent))
import capture_contract  # noqa: E402
import seed  # noqa: E402

# A case over this is a failure, not a slow pass.
CASE_TIMEOUT = 600

# The small input data M-10 and the leave-generation capture run on: MAGPIE's
# own two-letter test lexicon and distribution. Served from a tarball under a
# date no real MAGPIE-DATA release has, at a ref only the stand-in resolves.
SMALL_DATE = "20000101"
SMALL_REF = "tier6-small"
SMALL_SHA = "7133e6a2b0c0ffee00000000000000000000ab01"
SMALL_FILES = (
    ("lexica/CSW21_ab.kwg", "testdata/lexica/CSW21_ab.kwg"),
    ("lexica/CSW21_ab.klv2", "testdata/lexica/CSW21_ab.klv2"),
    ("letterdistributions/english_ab.csv", "testdata/letterdistributions/english_ab.csv"),
)

# What MAGPIE prints when a task did not go through. It exits 0 even when it
# gives up after repeated failures, so the output is checked too.
FAILURE_SIGNS = ("task failed", "rejected the result", "gave up", "declining this task",
                 "declining a job", "Cannot contribute")


def log(message: str) -> None:
    print(f"[e2e] {message}", flush=True)


class Failure(RuntimeError):
    pass


def expect(condition: bool, message: str) -> None:
    if not condition:
        raise Failure(message)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


# --- a GitHub stand-in for the small tarball --------------------------------


def small_tarball(magpie_root: Path) -> bytes:
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
        for inside, source in SMALL_FILES:
            path = magpie_root / source
            if not path.is_file():
                raise Failure(f"no {path}: the MAGPIE checkout's testdata is needed "
                              "(its download_data.sh installs it)")
            archive.add(str(path), arcname=f"data/{inside}")
    return buffer.getvalue()


class GitHubStandIn(ThreadingHTTPServer):
    """Answers the two GitHub calls an input-data import makes.

    `SMALL_REF` resolves to `SMALL_SHA`, whose tarball is served from memory.
    Everything else is GitHub's: a ref resolution is forwarded (with the
    backend's token, if it sent one) and a tarball download redirected, so the
    real MAGPIE-DATA import the other cases rely on is still the real one.
    """

    daemon_threads = True

    def __init__(self, listen, tarball: bytes):
        self.tarball = tarball
        super().__init__(listen, _GitHubHandler)
        threading.Thread(target=self.serve_forever, daemon=True).start()


class _GitHubHandler(BaseHTTPRequestHandler):
    server: GitHubStandIn

    def log_message(self, *_args) -> None:
        pass

    def _send(self, status: int, body: bytes, headers=()) -> None:
        self.send_response(status)
        for key, value in headers:
            self.send_header(key, value)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:
        if self.path.startswith("/api/"):
            rest = self.path[len("/api"):]
            if re.fullmatch(r"/repos/[^/]+/[^/]+/commits/" + SMALL_REF, rest):
                return self._send(200, SMALL_SHA.encode())
            headers = {k: v for k, v in self.headers.items()
                       if k.lower() in ("accept", "authorization", "user-agent")}
            request = urllib.request.Request("https://api.github.com" + rest, headers=headers)
            try:
                with urllib.request.urlopen(request, timeout=60) as answer:
                    status, body, answer_headers = answer.status, answer.read(), answer.headers
            except urllib.error.HTTPError as err:
                status, body, answer_headers = err.code, err.read(), err.headers
            except urllib.error.URLError as err:
                return self._send(502, f"cannot reach GitHub: {err}".encode())
            passed = [(k, v) for k, v in answer_headers.items()
                      if k.lower().startswith("x-ratelimit") or k.lower() == "content-type"]
            return self._send(status, body, passed)
        if self.path.startswith("/raw/"):
            rest = self.path[len("/raw"):]
            parts = rest.split("/")
            if len(parts) > 3 and parts[3] == SMALL_SHA:
                if parts[-1] == f"data-{SMALL_DATE}.tgz":
                    return self._send(200, self.server.tarball)
                return self._send(404, b"not found")
            return self._send(302, b"", [("Location", "https://raw.githubusercontent.com" + rest)])
        self._send(404, b"not found")


# --- the run's shared state -------------------------------------------------


class Context:
    def __init__(self, args, client: seed.Client, seed_args):
        self.args = args
        self.client = client
        self.seed_args = seed_args
        self.data: Dict[str, str] = {}
        self.winpct: Optional[str] = None
        self.deadline = time.time() + CASE_TIMEOUT

    def remaining(self) -> float:
        left = self.deadline - time.time()
        if left <= 0:
            raise Failure(f"the case ran past its {CASE_TIMEOUT}s limit")
        return left

    def psql(self, sql: str) -> str:
        return seed.psql(self.args.compose_service, sql)

    def get(self, path: str, what: str):
        return self.client.json(self.client.get(path), what)

    def post(self, path: str, what: str, body: Optional[dict] = None):
        return self.client.json(self.client.post(path, body), what)

    def delete(self, path: str, what: str) -> None:
        response = self.client.session.delete(f"{self.client.api}{path}",
                                              headers=self.client._headers(), timeout=120)
        self.client.json(response, what)


def create_player(ctx: Context, data: dict, name: str, body: dict) -> str:
    for existing in ctx.get("/api/admin/player-configs", "list player configs"):
        if existing["name"] == name:
            return existing["id"]
    created = ctx.post("/api/admin/player-configs", f"create player config {name}", {
        "name": name, "recorder_type": "best", "sort_strategy": "equity",
        "kwg_id": data["kwg"], "klv_id": data["klv"], "num_plays_recorded": 5,
        **body,
    })
    return created["id"]


def ranked_moves(job_id_results: list) -> int:
    """How many moves the busiest rack in a results page came back with."""
    return max((r["num_moves"] for r in job_id_results), default=0)


def deactivate_everything(ctx: Context) -> None:
    page = ctx.get("/api/jobs?per_page=100", "list jobs")
    for job in page["items"]:
        if job["status"] == "active":
            ctx.post(f"/api/admin/jobs/{job['id']}/deactivate", "deactivate job")


def create_and_activate(ctx: Context, data: dict, body: dict) -> str:
    # Leave generation seeds every full rack at creation, which takes longer
    # than seed.Client's default request timeout.
    started = time.time()
    response = ctx.client.session.post(
        f"{ctx.client.api}/api/admin/jobs",
        json={"variant": "classic", "letterdist_id": data["letterdist"],
              "layout_id": data["layout"], **body},
        headers=ctx.client._headers(), timeout=1800,
    )
    job_id = ctx.client.json(response, f"create {body['job_type']} job")["job"]["id"]
    ctx.post(f"/api/admin/jobs/{job_id}/activate", "activate job", {"allocation": 100})
    log(f"created and activated {body['job_type']} job {job_id} "
        f"in {time.time() - started:.1f}s")
    return job_id


def delete_job(ctx: Context, job_id: str) -> None:
    ctx.delete(f"/api/admin/jobs/{job_id}", "delete job")


def build_derived(ctx: Context) -> None:
    """Drains the derived-file build queue once, as the scheduled task does.

    A job whose players ask for a wordmap or a rack info table is not dispatched
    until the server has built its reference copy and recorded the hash, and
    nothing in the compose stack does that on its own.
    """
    started = time.time()
    run = subprocess.run(
        ["docker", "compose", "run", "--rm", "--build", "derived-builder"],
        cwd=seed.REPO_ROOT, stdin=subprocess.DEVNULL, capture_output=True, text=True,
        timeout=min(1800, ctx.remaining()),
    )
    output = run.stdout + run.stderr
    expect(run.returncode == 0, f"the derived-file builder failed:\n{output[-3000:]}")
    unbuilt = ctx.psql("SELECT COUNT(*) FROM derived_data WHERE state <> 'built'")
    expect(unbuilt == "0", f"{unbuilt} derived files are still not built:\n{output[-3000:]}")
    log(f"derived files built in {time.time() - started:.0f}s")


def completed_claims(ctx: Context, job_id: str) -> int:
    return int(ctx.psql(
        "SELECT COUNT(*) FROM task_claims c JOIN tasks t ON t.id = c.task_id "
        f"WHERE t.job_id = '{job_id}' AND c.state = 'completed'"))


def claim_states(ctx: Context, job_id: str) -> List[str]:
    rows = ctx.psql(
        "SELECT c.state FROM task_claims c JOIN tasks t ON t.id = c.task_id "
        f"WHERE t.job_id = '{job_id}' ORDER BY c.claimed_at")
    return rows.split("\n") if rows else []


def decline_reasons(ctx: Context, job_id: str) -> List[str]:
    rows = ctx.psql("SELECT reason FROM audit_log WHERE action = 'task.declined' "
                    f"AND job_id = '{job_id}' ORDER BY id")
    return rows.split("\n") if rows else []


def data_gaps(ctx: Context, job_id: str) -> List[List[str]]:
    rows = ctx.psql("SELECT role, name, expected, COALESCE(actual, '') FROM worker_data_gaps "
                    f"WHERE job_id = '{job_id}' ORDER BY reported_at")
    return [row.split("|") for row in rows.split("\n")] if rows else []


def stored_results(ctx: Context, job_id: str) -> int:
    return int(ctx.psql(
        f"SELECT (SELECT COUNT(*) FROM game_results WHERE job_id = '{job_id}') "
        f"     + (SELECT COUNT(*) FROM position_analysis_records WHERE job_id = '{job_id}')"))


# --- contributors -----------------------------------------------------------


def link_tree(source: Path, target: Path, skip: Callable[[Path], bool]) -> None:
    """`target` as a directory of symlinks to `source`'s files, less `skip`."""
    target.mkdir(parents=True, exist_ok=True)
    for entry in sorted(source.iterdir()):
        if entry.is_dir():
            link_tree(entry, target / entry.name, skip)
        elif not skip(entry):
            (target / entry.name).symlink_to(entry.resolve())


def derived_file(path: Path) -> bool:
    # Anything a worker builds for itself: a wordmap and its sidecar, a rack
    # info table, a word info table, and leave generation's outputs.
    return (path.name.endswith((".wmp", ".wmp.src", ".rit", ".wit", "_report.txt"))
            or "_gen_" in path.name)


class Worker:
    """One `magpie contribute` identity in a directory of its own.

    Its data directory is its own too, unless `shared_data`: symlinks to the
    MAGPIE install's input files, and none of its derived ones, so whatever the
    worker builds lands here and is checked here rather than in -- or against
    -- the checkout.
    """

    def __init__(self, ctx: Context, name: str, *, small: bool = False,
                 altered: Optional[Dict[str, bytes]] = None, shared_data: bool = False):
        self.ctx = ctx
        self.dir = ctx.args.workdir.resolve() / name
        shutil.rmtree(self.dir, ignore_errors=True)
        self.dir.mkdir(parents=True)
        data = self.dir / "data"
        install = (ctx.args.magpie_root / "data").resolve()
        if shared_data:
            data.symlink_to(install)
        else:
            link_tree(install, data, derived_file)
        if small:
            for inside, source in SMALL_FILES:
                target = data / inside
                if target.exists() or target.is_symlink():
                    target.unlink()
                shutil.copyfile(ctx.args.magpie_root / source, target)
        for inside, extra in (altered or {}).items():
            target = data / inside
            original = target.read_bytes()
            target.unlink()
            target.write_bytes(original + extra)
        self.data = data

    def _settings(self, server: Optional[str], tasks: int) -> None:
        # The identity the server minted on the first run is kept: MAGPIE
        # appends `uuid` to this file, and a rewrite without it would make
        # every run a new anonymous worker.
        path = self.dir / "contribute.txt"
        kept = []
        if path.exists():
            kept = [line for line in path.read_text().splitlines() if line.startswith("uuid ")]
        lines = [f"server {server or self.ctx.args.api}", f"threads {self.ctx.args.threads}",
                 f"maxtasks {tasks}", "idlewait 2", *kept]
        path.write_text("\n".join(lines) + "\n")

    def uuid(self) -> Optional[str]:
        for line in (self.dir / "contribute.txt").read_text().splitlines():
            if line.startswith("uuid "):
                return line.split()[1]
        return None

    def start(self, tasks: int, server: Optional[str] = None) -> subprocess.Popen:
        self._settings(server, tasks)
        # No stdin: given an open one, magpie waits on it after the command
        # finishes instead of exiting.
        return subprocess.Popen(
            [str(self.ctx.args.magpie.resolve()), "contribute", "contribute.txt"],
            cwd=self.dir, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, text=True,
        )

    def finish(self, process: subprocess.Popen, timeout: Optional[float] = None) -> str:
        try:
            output, _ = process.communicate(timeout=timeout or self.ctx.remaining())
        except subprocess.TimeoutExpired:
            process.kill()
            output, _ = process.communicate()
            raise Failure(f"magpie contribute in {self.dir.name} did not finish:\n"
                          f"{output[-3000:]}")
        expect(process.returncode == 0,
               f"magpie contribute exited {process.returncode}:\n{output[-3000:]}")
        return output

    def run(self, tasks: int, server: Optional[str] = None, succeed: bool = True) -> str:
        started = time.time()
        output = self.finish(self.start(tasks, server))
        log(f"magpie ({self.dir.name}) exited after {time.time() - started:.0f}s")
        if succeed:
            for sign in FAILURE_SIGNS:
                expect(sign not in output, f"a task failed ({sign!r}):\n{output[-3000:]}")
        return output

    def remove(self) -> None:
        shutil.rmtree(self.dir, ignore_errors=True)


def contribute(ctx: Context, tasks: int) -> str:
    """Runs `magpie contribute` against the checkout's own data directory
    until it completes `tasks` tasks. M-1 to M-4 run this way, so M-4 can
    assert that leave generation writes nothing into that directory."""
    worker = Worker(ctx, "shared", shared_data=True)
    try:
        return worker.run(tasks)
    finally:
        worker.remove()


def run_job(ctx: Context, data: dict, body: dict, check, needs_build: bool = False) -> None:
    deactivate_everything(ctx)
    job_id = create_and_activate(ctx, data, body)
    try:
        if needs_build:
            build_derived(ctx)
        output = contribute(ctx, ctx.args.tasks)
        claims = completed_claims(ctx, job_id)
        expect(claims >= ctx.args.tasks,
               f"{body['job_type']}: {claims} accepted claims, "
               f"expected at least {ctx.args.tasks}:\n{output[-3000:]}")
        check(job_id)
        log(f"{body['job_type']}: ok ({claims} accepted claims)")
    finally:
        delete_job(ctx, job_id)


# --- players ----------------------------------------------------------------


def static_players(ctx: Context) -> dict:
    data = ctx.data
    return {
        "player1_config_id": create_player(ctx, data, "e2e-static-equity", {}),
        "player2_config_id": create_player(ctx, data, "e2e-static-score",
                                           {"sort_strategy": "score"}),
    }


def wordmap_players(ctx: Context) -> dict:
    data = ctx.data
    return {
        "player1_config_id": create_player(ctx, data, "e2e-wordmap-equity",
                                           {"use_wordmap": True}),
        "player2_config_id": create_player(ctx, data, "e2e-wordmap-score",
                                           {"sort_strategy": "score", "use_wordmap": True}),
    }


def simming_player(ctx: Context) -> str:
    # `all`, not `best`: -r best is MOVE_RECORD_BEST, which leaves movegen with
    # one play, so a simmer configured that way has nothing to choose between
    # and every rack comes back with a single move. Job creation refuses the
    # combination now; this is the config that actually ranks.
    return create_player(ctx, ctx.data, "e2e-simming", {
        "recorder_type": "all",
        "winpct_id": ctx.winpct, "num_plies": 2, "num_plays": 5, "num_plies_recorded": 2,
        "max_iterations": 60, "stopping_pct": 99, "time_limit_secs": 0,
    })


def static_best_player(ctx: Context) -> str:
    # One move per rack, which `best` is exactly right for -- and is what
    # `num_plays_recorded` 1 says. It also asks for a wordmap, so its job goes
    # through the derived-file path: the server builds and hashes one, and
    # MAGPIE builds its own and compares.
    return create_player(ctx, ctx.data, "e2e-static-best",
                         {"num_plays_recorded": 1, "use_wordmap": True})


def games_body(players: dict, batch: int, **extra) -> dict:
    return {"job_type": "games", **players, "games_per_batch": batch,
            "min_games": 1_000_000, "max_games": 1_000_000, **extra}


# --- the small data ---------------------------------------------------------


def small_data(ctx: Context) -> dict:
    """Imports the two-letter data and returns its ids, or fails naming the
    setting that points the backend at the stand-in."""
    expect(ctx.args.github_fixture_port is not None,
           "this case needs --github-fixture-port, with the backend's GITHUB_API_URL and "
           "GITHUB_RAW_URL pointing at http://<this host>:<port>/api and /raw "
           "(scripts/e2e_magpie_native.sh sets both)")
    import_args = argparse.Namespace(tarball_date=SMALL_DATE, git_ref=SMALL_REF,
                                     import_timeout=min(300, ctx.remaining()))
    try:
        seed.import_input_data(ctx.client, import_args)
    except seed.SeedError as err:
        raise Failure(f"importing the small data failed ({err}). Is the backend's "
                      "GITHUB_API_URL/GITHUB_RAW_URL pointing at this script's "
                      f"--github-fixture-port {ctx.args.github_fixture_port}?")
    rows = ctx.get("/api/admin/input-data", "list input data")

    def find(role: str, name: str) -> str:
        for row in rows:
            if row["role"] == role and row["name"] == name:
                return row["id"]
        raise Failure(f"the small data has no {role} {name}")

    return {"kwg": find("kwg", "CSW21_ab"), "klv": find("klv", "CSW21_ab"),
            "letterdist": find("letterdist", "english_ab"), "layout": ctx.data["layout"]}


def remove_small_data(ctx: Context) -> None:
    """Everything that pins the two-letter data, and then the data: a rerun
    starts from an import, not from a leftover."""
    ids = ctx.psql(f"SELECT id FROM input_data WHERE tarball_date = '{SMALL_DATE}'")
    ids = ids.split("\n") if ids else []
    if not ids:
        return
    quoted = ",".join(f"'{i}'" for i in ids)
    jobs = ctx.psql(f"SELECT id FROM jobs WHERE letterdist_id IN ({quoted})")
    for job_id in (jobs.split("\n") if jobs else []):
        delete_job(ctx, job_id)
    players = ctx.psql(f"SELECT id FROM player_configs WHERE kwg_id IN ({quoted})")
    for player_id in (players.split("\n") if players else []):
        ctx.delete(f"/api/admin/player-configs/{player_id}", "delete player config")
    # Reference copies the builder recorded; no endpoint removes one.
    ctx.psql(f"DELETE FROM derived_data WHERE kwg_id IN ({quoted}) "
             f"OR klv_id IN ({quoted}) OR letterdist_id IN ({quoted})")
    for input_id in ids:
        ctx.delete(f"/api/admin/input-data/{input_id}", "delete input data")


# --- M-1 to M-4 --------------------------------------------------------------


def case_games(ctx: Context) -> None:
    """M-1"""
    def games_counted(job_id: str) -> None:
        stats = ctx.get(f"/api/jobs/{job_id}", "job stats")
        expect(stats["games"]["units_completed"] > 0, f"no games counted: {stats['games']}")

    run_job(ctx, ctx.data, games_body(static_players(ctx), 2), games_counted)


def case_pairs(ctx: Context) -> None:
    """M-2"""
    def pairs_counted(job_id: str) -> None:
        stats = ctx.get(f"/api/jobs/{job_id}", "job stats")
        penta = stats["games"]["pentanomial"]
        expect(sum(penta) > 0, f"no pairs counted: {stats['games']}")
        # Also enforced by the server on every accepted result; asserted here
        # on the aggregate so the invariant is visible in this tier too.
        expect(sum(penta) == stats["games"]["units_completed"],
               f"pentanomial {penta} does not count every pair: {stats['games']}")

    run_job(ctx, ctx.data, {"job_type": "game_pairs", **static_players(ctx),
                            "pairs_per_batch": 2, "min_pairs": 1000, "max_pairs": 1000},
            pairs_counted)


def case_opening_racks(ctx: Context) -> None:
    """M-3, for a static and a simming player."""
    def best_moves(job_id: str) -> None:
        page = ctx.get(f"/api/jobs/{job_id}/results", "results")
        expect(page["items"] and all(r["best_move"] for r in page["items"]),
               f"racks without a best move: {page['items'][:3]}")

    def simulated_statistics(job_id: str) -> None:
        page = ctx.get(f"/api/jobs/{job_id}/results", "results")
        expect(page["items"] and all(r["best_move"] for r in page["items"]),
               f"racks without a best move: {page['items'][:3]}")
        # A simmer ranks far more than it reports, and num_moves is the only
        # record of how many. One would mean the recorder kept the best play
        # alone and the simulation had nothing to do.
        expect(ranked_moves(page["items"]) > 1,
               f"the simmer ranked one move per rack: {page['items'][:3]}")
        row = ctx.psql(
            "SELECT COUNT(*) FILTER (WHERE m.win_percentage IS NOT NULL), "
            "       COUNT(*) FILTER (WHERE m.blended_utility IS NOT NULL), "
            "       (SELECT COUNT(*) FROM position_analysis_plies p "
            "        JOIN position_analysis_moves pm ON pm.id = p.move_id "
            "        JOIN position_analysis_records pr ON pr.id = pm.record_id "
            f"       WHERE pr.job_id = '{job_id}') "
            "FROM position_analysis_moves m "
            "JOIN position_analysis_records r ON r.id = m.record_id "
            f"WHERE r.job_id = '{job_id}'")
        win, utility, plies = (int(v) for v in row.split("|"))
        expect(win > 0 and utility > 0 and plies > 0,
               f"simulated statistics missing: win%={win} utility={utility} plies={plies}")

    run_job(ctx, ctx.data, {"job_type": "opening_rack", "player_config_id": static_best_player(ctx),
                            "racks_per_batch": 20, "rack_size": 7}, best_moves,
            needs_build=True)
    run_job(ctx, ctx.data, {"job_type": "opening_rack", "player_config_id": simming_player(ctx),
                            "racks_per_batch": 3, "rack_size": 7}, simulated_statistics)


def case_leave(ctx: Context) -> None:
    """M-4"""
    lexica = ctx.args.magpie_root / "data" / "lexica"
    before = {p.name for p in lexica.iterdir()}

    def leave_occurrences(job_id: str) -> None:
        # An accepted leave result is staged, and folded into the per-rack
        # totals by a merge -- half-hourly, or when a generation nears its end.
        # Both halves are asserted: the submissions staged something and moved
        # the generation's live counters, and a merge folds it in.
        staged = int(ctx.psql(
            f"SELECT COUNT(*) FROM leave_rack_staging WHERE job_id = '{job_id}'"))
        merged_already = int(ctx.psql(
            "SELECT COUNT(*) FROM leave_rack_progress "
            f"WHERE job_id = '{job_id}' AND generation = 1 AND occurrence_count > 0"))
        expect(staged > 0 or merged_already > 0, "no leave result was staged or merged")
        played = int(ctx.psql(
            "SELECT COALESCE(SUM(games_played), 0) FROM leave_generation_progress "
            f"WHERE job_id = '{job_id}'"))
        expect(played > 0, "the generation's live counters did not move")
        outcome = ctx.post(f"/api/admin/jobs/{job_id}/merge-progress", "merge leave progress")
        expect(outcome["folds_merged"] == staged,
               f"merged {outcome['folds_merged']} staged results, expected {staged}")
        left = int(ctx.psql(
            f"SELECT COUNT(*) FROM leave_rack_staging WHERE job_id = '{job_id}'"))
        expect(left == 0, f"{left} results still staged after a merge")
        occurred = int(ctx.psql(
            "SELECT COUNT(*) FROM leave_rack_progress "
            f"WHERE job_id = '{job_id}' AND generation = 1 AND occurrence_count > 0"))
        expect(occurred > 0, "no rack occurrences folded into generation 1")
        short = int(ctx.psql(
            f"SELECT COUNT(*) FROM leave_rack_progress WHERE job_id = '{job_id}' "
            "AND length(rack) <> 7"))
        expect(short == 0, f"{short} progress rows are not full racks")
        written = sorted({p.name for p in lexica.iterdir()} - before)
        stray = [name for name in written if "_gen_" in name or name.endswith("_report.txt")]
        expect(not stray, f"leave generation wrote into MAGPIE's data directory: {stray}")

    # With the wordmap a leave job asks for by default: the builder finds the
    # wordmap already built if M-3 ran, and the worker its own copy matching.
    run_job(ctx, ctx.data, {"job_type": "leave_generation", "kwg_id": ctx.data["kwg"],
                            "num_iterations": 20, "generation_count": 1,
                            "target_rack_count": 1, "racks_per_task": 50},
            leave_occurrences, needs_build=True)


# --- M-5 to M-11 -------------------------------------------------------------


def case_missing_data(ctx: Context) -> None:
    """M-5: a worker whose english.csv differs from the one the job pins
    declines with `missing_data` rather than contributing."""
    deactivate_everything(ctx)
    job_id = create_and_activate(ctx, ctx.data, games_body(static_players(ctx), 2))
    worker = Worker(ctx, "m5", altered={"letterdistributions/english.csv": b"\n"})
    try:
        output = worker.run(tasks=1, succeed=False)
        actual = sha256_file(worker.data / "letterdistributions" / "english.csv")
        expected = ctx.psql("SELECT sha256 FROM input_data "
                            f"WHERE id = '{ctx.data['letterdist']}'")
        expect(actual != expected, "the altered english.csv still matches")

        expect(claim_states(ctx, job_id) == ["declined"],
               f"expected one declined claim, found {claim_states(ctx, job_id)}:\n{output[-2000:]}")
        expect(decline_reasons(ctx, job_id) == ["missing_data"],
               f"decline reasons: {decline_reasons(ctx, job_id)}")
        gaps = data_gaps(ctx, job_id)
        expect(gaps == [["letterdist", "english", expected, actual]],
               f"worker_data_gaps: {gaps}, expected english with {expected} / {actual}")
        expect(stored_results(ctx, job_id) == 0, "a result was stored for a declined task")
        # The job is now in the worker's unsupported set, and it is the only
        # one active: the server's answer is a data shutdown, and MAGPIE stops.
        expect("Cannot contribute to any available job" in output
               and "download_data.sh" in output,
               f"MAGPIE did not report the data shutdown:\n{output[-2000:]}")
        log("M-5: declined with missing_data; the gap names english.csv with both digests")
    finally:
        delete_job(ctx, job_id)
        worker.remove()


def case_version_floor(ctx: Context) -> None:
    """M-6"""
    deactivate_everything(ctx)
    players = static_players(ctx)

    # The real server: a job above this build's version is never handed out.
    # With nothing else active, the answer is a shutdown naming the floor.
    job_id = create_and_activate(ctx, ctx.data,
                                 games_body(players, 2, min_magpie_version="0.2.0"))
    worker = Worker(ctx, "m6")
    try:
        output = worker.run(tasks=1, succeed=False)
        expect("Update MAGPIE to 0.2.0 or newer" in output,
               f"MAGPIE was not told to update:\n{output[-2000:]}")
        expect(claim_states(ctx, job_id) == [], "a job above the worker's version was claimed")
    finally:
        delete_job(ctx, job_id)

    # A server that does not filter -- an older one, or a floor raised after
    # dispatch: MAGPIE's own cross-check declines the job with
    # `magpie_version`. Stood in for by a proxy raising the floor on every
    # assignment it passes back.
    def raise_floor(path: str, body: dict) -> dict:
        if path.startswith("/api/worker/task") and "task_request" in body:
            return {**body, "min_magpie_version": "99.0.0"}
        return body

    job_id = create_and_activate(ctx, ctx.data, games_body(players, 2))
    proxy = capture_contract.RecordingProxy(
        ("127.0.0.1", 0), ctx.args.api, capture_contract.Recorder(None, ()), raise_floor
    ).start()
    try:
        output = worker.run(tasks=1, server=proxy.url, succeed=False)
        expect("declining a job that requires MAGPIE 99.0.0" in output,
               f"MAGPIE did not decline the job:\n{output[-2000:]}")
        expect(claim_states(ctx, job_id) == ["declined"],
               f"claims: {claim_states(ctx, job_id)}")
        expect(decline_reasons(ctx, job_id) == ["magpie_version"],
               f"decline reasons: {decline_reasons(ctx, job_id)}")
        expect(data_gaps(ctx, job_id) == [], "a version decline recorded data gaps")
        expect(stored_results(ctx, job_id) == 0, "a result was stored for a declined task")
        log("M-6: floor above the build -> magpie_too_old shutdown; "
            "an assignment above it anyway -> declined with magpie_version")
    finally:
        proxy.stop()
        delete_job(ctx, job_id)
        worker.remove()


def case_positions(ctx: Context) -> None:
    """M-7: every position `capture_positions` stores is a CGP MAGPIE loads."""
    deactivate_everything(ctx)
    job_id = create_and_activate(ctx, ctx.data,
                                 games_body(static_players(ctx), 1, capture_positions=True))
    worker = Worker(ctx, "m7")
    try:
        worker.run(tasks=1)
        rows = ctx.psql("SELECT position FROM position_analysis_records "
                        f"WHERE job_id = '{job_id}' AND position IS NOT NULL "
                        "ORDER BY game_index, turn_number")
        positions = rows.split("\n") if rows else []
        expect(len(positions) >= 10, f"only {len(positions)} positions captured in a game")
        # Each position in a MAGPIE of its own: an interactive session reads
        # the next line while a command runs and takes it for an async one.
        # `cgp` takes the four CGP fields; the lexicon and distribution the
        # job pins follow. MAGPIE exits 0 either way, so what decides is
        # whether it printed an error -- and a corrupted position is fed first
        # to show that it does.
        def load(cgp: str) -> str:
            run = subprocess.run(
                [str(ctx.args.magpie.resolve()),
                 f"cgp {cgp} -lex NWL23 -ld english -wmp false"],
                cwd=worker.dir, stdin=subprocess.DEVNULL, capture_output=True, text=True,
                timeout=min(60, ctx.remaining()),
            )
            return run.stdout + run.stderr

        board, rest = positions[-1].split(" ", 1)
        corrupted = load(f"{board.split('/', 1)[1]} {rest}")
        expect("(error" in corrupted,
               f"a position missing a row loaded without complaint: {corrupted!r}")
        for cgp in positions:
            output = load(cgp)
            expect("(error" not in output and "error" not in output.lower(),
                   f"MAGPIE refused a captured position:\n{cgp}\n{output}")
        log(f"M-7: {len(positions)} captured positions all load in MAGPIE")
    finally:
        delete_job(ctx, job_id)
        worker.remove()


def case_concurrent(ctx: Context) -> None:
    """M-9: two contributors at once, no duplicate seeds."""
    deactivate_everything(ctx)
    # Tasks long enough (a second or two each) that the two runs overlap for
    # certain, not by the luck of process start-up.
    batch, tasks = 500, 4
    job_id = create_and_activate(ctx, ctx.data, games_body(static_players(ctx), batch))
    workers = [Worker(ctx, "m9a"), Worker(ctx, "m9b")]
    try:
        processes = [w.start(tasks) for w in workers]
        outputs = [w.finish(p) for w, p in zip(workers, processes)]
        for output in outputs:
            for sign in FAILURE_SIGNS:
                expect(sign not in output, f"a task failed ({sign!r}):\n{output[-3000:]}")

        seeds = ctx.psql(f"SELECT seed FROM tasks WHERE job_id = '{job_id}' ORDER BY seed")
        seeds = [int(s) for s in seeds.split("\n")] if seeds else []
        expect(len(seeds) == len(set(seeds)), f"duplicate seeds: {seeds}")
        expect(len(seeds) == 2 * tasks and
               seeds == [seeds[0] + i * batch for i in range(len(seeds))],
               f"the seeds do not tile the space: {seeds}")
        per_task = ctx.psql(
            "SELECT COUNT(*) FROM task_claims c JOIN tasks t ON t.id = c.task_id "
            f"WHERE t.job_id = '{job_id}' AND c.state = 'completed' "
            "GROUP BY t.id HAVING COUNT(*) > 1")
        expect(per_task == "", "a task was completed twice")
        rows = ctx.psql(
            "SELECT c.claimed_by_anon_uuid, COUNT(*), "
            "       EXTRACT(EPOCH FROM MIN(c.claimed_at)), EXTRACT(EPOCH FROM MAX(c.claimed_at)) "
            "FROM task_claims c JOIN tasks t ON t.id = c.task_id "
            f"WHERE t.job_id = '{job_id}' AND c.state = 'completed' "
            "GROUP BY c.claimed_by_anon_uuid ORDER BY 1")
        identities = [r.split("|") for r in rows.split("\n")] if rows else []
        uuids = sorted(w.uuid() for w in workers)
        expect(sorted(i[0] for i in identities) == uuids and
               all(int(i[1]) == tasks for i in identities),
               f"expected {tasks} completed claims for each of {uuids}: {identities}")
        # Concurrent, not merely two runs: each worker claimed while the other
        # was between its first and last claim.
        (_, _, a_first, a_last), (_, _, b_first, b_last) = identities
        expect(max(float(a_first), float(b_first)) < min(float(a_last), float(b_last)),
               f"the two contributors did not overlap: {identities}")
        log(f"M-9: {len(seeds)} tasks, {len(set(seeds))} distinct seeds, two workers interleaved")
    finally:
        delete_job(ctx, job_id)
        for worker in workers:
            worker.remove()


def case_rack_info_table(ctx: Context) -> None:
    """M-10"""
    deactivate_everything(ctx)
    remove_small_data(ctx)
    worker = None
    try:
        data = small_data(ctx)
        players = {
            "player1_config_id": create_player(ctx, data, "e2e-ab-rit-equity",
                                               {"use_rit": True, "use_wordmap": True}),
            "player2_config_id": create_player(ctx, data, "e2e-ab-rit-score",
                                               {"sort_strategy": "score", "use_rit": True,
                                                "use_wordmap": True}),
        }
        job_id = create_and_activate(ctx, data, games_body(players, 2))
        table = "CSW21_ab.CSW21_ab"
        state = ctx.psql(f"SELECT state FROM derived_data WHERE role = 'rit' AND name = '{table}'")
        expect(state == "pending", f"the rack info table is {state!r} before any build")

        # Not dispatched while the table is unbuilt: an anonymous claim finds
        # no work at all, since this is the only active job.
        claim = requests.post(f"{ctx.args.api}/api/worker/task",
                              json={"magpie_version": "0.1.0", "unsupported_jobs": []},
                              timeout=30)
        expect(claim.status_code == 204,
               f"a job waiting on its table dispatched: {claim.status_code} {claim.text[:300]}")
        expect(claim_states(ctx, job_id) == [], "a claim was issued before the table was built")

        build_derived(ctx)
        server_hash = ctx.psql(
            f"SELECT sha256 FROM derived_data WHERE role = 'rit' AND name = '{table}' "
            "AND state = 'built'")
        expect(len(server_hash) == 64, f"no built table recorded: {server_hash!r}")

        worker = Worker(ctx, "m10", small=True)
        worker.run(tasks=2)
        built = list(worker.data.rglob(f"{table}.rit"))
        expect(len(built) == 1, f"the worker built no {table}.rit: {built}")
        expect(sha256_file(built[0]) == server_hash,
               f"the worker's table {sha256_file(built[0])} is not the server's {server_hash}")
        expect(completed_claims(ctx, job_id) == 2, "the rack-info-table job completed no tasks")
        stats = ctx.get(f"/api/jobs/{job_id}", "job stats")
        expect(stats["games"]["units_completed"] == 4, f"games: {stats['games']}")
        log(f"M-10: dispatched only after the build; the worker's {table}.rit matches "
            f"{server_hash[:12]} and played 4 games with it")
    finally:
        remove_small_data(ctx)
        if worker:
            worker.remove()


def case_derived_mismatch(ctx: Context) -> None:
    """M-11: the server's recorded wordmap hash is not what this build makes."""
    deactivate_everything(ctx)
    job_id = create_and_activate(ctx, ctx.data, {
        "job_type": "opening_rack", "player_config_id": static_best_player(ctx),
        "racks_per_batch": 5, "rack_size": 7})
    worker = Worker(ctx, "m11")
    where = "role = 'wmp' AND name = 'NWL23' AND state = 'built'"
    original = None
    try:
        build_derived(ctx)
        original = ctx.psql(f"SELECT sha256 FROM derived_data WHERE {where}")
        expect(len(original) == 64, f"no built NWL23 wordmap recorded: {original!r}")
        # What a server whose builder differs from this worker's would have
        # recorded: some other hash. Written before the job's first claim,
        # since the claim path remembers a job's hashes once it has read them.
        altered = original[:-1] + ("0" if original[-1] != "0" else "1")
        ctx.psql(f"UPDATE derived_data SET sha256 = '{altered}' WHERE {where}")

        output = worker.run(tasks=1, succeed=False)
        expect("declining this task: a file built here does not match" in output,
               f"MAGPIE did not decline over the wordmap:\n{output[-2000:]}")
        expect(claim_states(ctx, job_id) == ["declined"], f"claims: {claim_states(ctx, job_id)}")
        expect(decline_reasons(ctx, job_id) == ["derived_mismatch"],
               f"decline reasons: {decline_reasons(ctx, job_id)}")
        gaps = data_gaps(ctx, job_id)
        expect(gaps == [["wmp", "NWL23", altered, original]],
               f"worker_data_gaps: {gaps}, expected wmp NWL23 {altered} / {original}")
        built = worker.data / "lexica" / "NWL23.wmp"
        expect(built.is_file() and sha256_file(built) == original,
               "the worker's own wordmap is not the one the server built")
        expect(stored_results(ctx, job_id) == 0, "a result was stored for a declined task")
        log("M-11: declined with derived_mismatch; both digests reached worker_data_gaps")
    finally:
        if original:
            ctx.psql(f"UPDATE derived_data SET sha256 = '{original}' "
                     "WHERE role = 'wmp' AND name = 'NWL23'")
        delete_job(ctx, job_id)
        worker.remove()


# --- capturing the contract fixtures ----------------------------------------


def case_capture(ctx: Context) -> None:
    """Runs one job of each type through the recording proxy."""
    out = ctx.args.capture_out
    expect(out is not None, "capture needs --capture-out, the directory to write into")
    out.mkdir(parents=True, exist_ok=True)
    deactivate_everything(ctx)
    remove_small_data(ctx)
    recorder = capture_contract.Recorder(out)
    proxy = capture_contract.RecordingProxy(("127.0.0.1", 0), ctx.args.api, recorder).start()
    worker = Worker(ctx, "capture", small=True)
    jobs: List[str] = []

    def one(body: dict, data: dict, needs_build: bool = False) -> None:
        deactivate_everything(ctx)
        jobs.append(create_and_activate(ctx, data, body))
        if needs_build:
            build_derived(ctx)
        worker.run(tasks=1, server=proxy.url)

    try:
        # First, from a worker with no identity yet: the assignment that mints
        # one. A games job capturing positions, so its result carries them.
        one(games_body(static_players(ctx), 1, capture_positions=True), ctx.data)
        expect(worker.uuid() is not None, "the first assignment minted no worker UUID")
        # Players with a wordmap, so the assignment pins a derived file.
        one({"job_type": "game_pairs", **wordmap_players(ctx), "pairs_per_batch": 2,
             "min_pairs": 1_000_000, "max_pairs": 1_000_000}, ctx.data, needs_build=True)
        one({"job_type": "opening_rack", "player_config_id": simming_player(ctx),
             "racks_per_batch": 2, "rack_size": 7}, ctx.data)
        small = small_data(ctx)
        one({"job_type": "leave_generation", "kwg_id": small["kwg"], "num_iterations": 20,
             "generation_count": 1, "target_rack_count": 1, "racks_per_task": 50},
            small, needs_build=True)

        # A heartbeat goes out thirty seconds into a task, so the last one is a
        # batch far too big to finish: the contributor is stopped once the
        # heartbeat has been seen.
        deactivate_everything(ctx)
        jobs.append(create_and_activate(ctx, ctx.data, games_body(static_players(ctx),
                                                                  10_000_000)))
        process = worker.start(1, server=proxy.url)
        deadline = time.time() + min(120, ctx.remaining())
        while "heartbeat.json" not in recorder.captured and time.time() < deadline:
            time.sleep(1)
        process.send_signal(signal.SIGINT)
        try:
            process.communicate(timeout=30)
        except subprocess.TimeoutExpired:
            process.kill()
            process.communicate()

        missing = recorder.missing()
        expect(not missing, f"not captured: {missing}")
        log(f"captured {len(recorder.captured)} fixtures into {out}")
    finally:
        proxy.stop()
        for job_id in jobs:
            try:
                delete_job(ctx, job_id)
            except seed.SeedError:
                pass  # the small data's own cleanup below takes its job
        remove_small_data(ctx)
        worker.remove()


CASES = {
    "M-1": case_games,
    "M-2": case_pairs,
    "M-3": case_opening_racks,
    "M-4": case_leave,
    "M-5": case_missing_data,
    "M-6": case_version_floor,
    "M-7": case_positions,
    "M-9": case_concurrent,
    "M-10": case_rack_info_table,
    "M-11": case_derived_mismatch,
    "capture": case_capture,
}
DEFAULT_CASES = [name for name in CASES if name.startswith("M-")]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--api", default="http://localhost:8080")
    parser.add_argument("--compose-service", default="postgres")
    parser.add_argument("--backend-service", default="backend")
    parser.add_argument("--magpie", type=Path, required=True, help="the magpie binary")
    parser.add_argument("--magpie-root", type=Path, required=True,
                        help="the MAGPIE checkout: its data/, testdata/ and download_data.sh")
    parser.add_argument("--workdir", type=Path, default=Path(".e2e-worker"))
    parser.add_argument("--threads", type=int, default=4)
    parser.add_argument("--tasks", type=int, default=2, help="tasks per job in M-1..M-4")
    parser.add_argument("--cases", default=",".join(DEFAULT_CASES),
                        help=f"comma-separated, from {', '.join(CASES)} (default: every M case)")
    parser.add_argument("--github-fixture-port", type=int, default=None,
                        help="serve the GitHub stand-in M-10 and capture import from here")
    parser.add_argument("--github-fixture-host", default="127.0.0.1",
                        help="address to bind the stand-in to (0.0.0.0 for a containerised "
                             "backend)")
    parser.add_argument("--capture-out", type=Path, default=None,
                        help="capture: where to write the contract fixtures")
    args = parser.parse_args()

    selected = [name.strip() for name in args.cases.split(",") if name.strip()]
    unknown = [name for name in selected if name not in CASES]
    if unknown:
        parser.error(f"unknown cases {unknown}; known: {', '.join(CASES)}")
    expect(args.magpie.is_file(), f"no MAGPIE binary at {args.magpie}")
    expect((args.magpie_root / "data" / "lexica").is_dir(),
           f"{args.magpie_root}/data is not a download_data.sh install")

    if args.github_fixture_port is not None:
        GitHubStandIn((args.github_fixture_host, args.github_fixture_port),
                      small_tarball(args.magpie_root))
        log(f"GitHub stand-in on {args.github_fixture_host}:{args.github_fixture_port}")

    seed_args = seed.build_parser().parse_args([
        "--api", args.api, "--compose-service", args.compose_service,
        "--backend-service", args.backend_service, "--magpie-root", str(args.magpie_root),
        "--username", "e2e", "--email", "e2e@example.invalid",
        "--password", "an end-to-end passphrase, long enough 42!",
    ])
    seed_args.tarball_date = seed.default_tarball_date(args.magpie_root)
    expect(seed_args.tarball_date is not None, "no DATA_VERSION in the MAGPIE checkout")

    client = seed.Client(args.api)
    seed.sign_in(client, seed_args)
    seed.import_input_data(client, seed_args)
    ctx = Context(args, client, seed_args)
    ctx.data = seed.input_data_ids(client, seed_args)
    ctx.winpct = next((r["id"] for r in ctx.get("/api/admin/input-data", "input data")
                       if r["role"] == "winpct"), None)
    expect(ctx.winpct is not None, "the imported data has no win% model")

    timings = []
    for name in selected:
        log(f"--- {name} ---")
        started = time.time()
        ctx.deadline = started + CASE_TIMEOUT
        CASES[name](ctx)
        timings.append((name, time.time() - started))
        log(f"{name}: passed in {timings[-1][1]:.0f}s")

    errors = subprocess.run(
        ["docker", "compose", "logs", "--no-color", args.backend_service],
        cwd=seed.REPO_ROOT, capture_output=True, text=True,
    ).stdout
    serious = [line for line in errors.splitlines() if '"level":"ERROR"' in line or " ERROR " in line]
    expect(not serious, "backend logged errors:\n" + "\n".join(serious[-20:]))
    shutil.rmtree(args.workdir, ignore_errors=True)
    for name, seconds in timings:
        log(f"  {name:8} {seconds:6.0f}s")
    log(f"passed: {', '.join(selected)}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (Failure, seed.SeedError) as err:
        print(f"[e2e] FAILED: {err}", file=sys.stderr)
        sys.exit(1)
    except requests.RequestException as err:
        print(f"[e2e] cannot reach the backend: {err}", file=sys.stderr)
        sys.exit(1)
