#!/usr/bin/env python3
"""Tier 6 (TESTING.md): real `magpie contribute` tasks against a real stack.

Runs one small job of every type, one at a time, and asserts that the results
MAGPIE submits are accepted and stored the way the server reads them back:

- `games` and `game_pairs`: games counted, a pentanomial for pairs.
- `opening_rack`, static: a best move per rack.
- `opening_rack`, simming: the simulated statistics (win%, per-ply stats) are
  stored, not just move, score and equity (AUDIT_FINDINGS.md F6).
- `leave_generation`: full-rack occurrences fold into the generation's
  progress (F1), and nothing is written into MAGPIE's data directory (F9).

Expects the compose stack (`postgres`, `minio`, `backend`) to be up, with
MAIL_BACKEND=console, and a built MAGPIE whose `data/` is a real
`download_data.sh` install. Seeding reuses `scripts/seed.py`, so the admin,
the input-data import and the player configs go through the real API.

    docker compose up -d --build --wait postgres minio minio-init backend
    python3 scripts/e2e_magpie.py --magpie ../MAGPIE/bin/magpie --magpie-root ../MAGPIE

Only for a disposable stack: it creates and activates jobs, and deactivates
every job it did not create.
"""

import argparse
import shutil
import subprocess
import sys
import time
from pathlib import Path

import requests

sys.path.insert(0, str(Path(__file__).resolve().parent))
import seed  # noqa: E402


def log(message: str) -> None:
    print(f"[e2e] {message}", flush=True)


class Failure(RuntimeError):
    pass


def expect(condition: bool, message: str) -> None:
    if not condition:
        raise Failure(message)


def psql(args, sql: str) -> str:
    return seed.psql(args.compose_service, sql)


def create_player(client, args, data: dict, name: str, body: dict) -> str:
    for existing in client.json(client.get("/api/admin/player-configs"), "list player configs"):
        if existing["name"] == name:
            return existing["id"]
    created = client.json(
        client.post("/api/admin/player-configs", {
            "name": name, "recorder_type": "best", "sort_strategy": "equity",
            "kwg_id": data["kwg"], "klv_id": data["klv"], "num_plays_recorded": 5,
            **body,
        }),
        f"create player config {name}",
    )
    return created["id"]


def deactivate_everything(client) -> None:
    page = client.json(client.get("/api/jobs?per_page=100"), "list jobs")
    for job in page["items"]:
        if job["status"] == "active":
            client.json(client.post(f"/api/admin/jobs/{job['id']}/deactivate"), "deactivate job")


def create_and_activate(client, args, data: dict, body: dict) -> str:
    # Leave generation seeds every full rack at creation, which takes longer
    # than seed.Client's default request timeout.
    started = time.time()
    response = client.session.post(
        f"{client.api}/api/admin/jobs",
        json={"variant": "classic", "letterdist_id": data["letterdist"],
              "layout_id": data["layout"], **body},
        headers=client._headers(), timeout=1800,
    )
    job_id = client.json(response, f"create {body['job_type']} job")["job"]["id"]
    client.json(client.post(f"/api/admin/jobs/{job_id}/activate", {"allocation": 100}),
                "activate job")
    log(f"created and activated {body['job_type']} job {job_id} "
        f"in {time.time() - started:.1f}s")
    return job_id


def contribute(args, tasks: int) -> str:
    """Runs `magpie contribute` until it completes `tasks` tasks."""
    work = args.workdir.resolve()
    work.mkdir(parents=True, exist_ok=True)
    data_link = work / "data"
    if not data_link.exists():
        data_link.symlink_to((args.magpie_root / "data").resolve())
    (work / "contribute.txt").write_text(
        f"server {args.api}\nthreads {args.threads}\nmaxtasks {tasks}\nidlewait 2\n"
    )
    started = time.time()
    run = subprocess.run(
        [str(args.magpie.resolve()), "contribute", "contribute.txt"],
        # No stdin: given an open one, magpie waits on it after the command
        # finishes instead of exiting.
        cwd=work, stdin=subprocess.DEVNULL, capture_output=True, text=True,
        timeout=args.task_timeout,
    )
    output = run.stdout + run.stderr
    log(f"magpie exited {run.returncode} after {time.time() - started:.0f}s")
    expect(run.returncode == 0, f"magpie contribute failed:\n{output[-3000:]}")
    # MAGPIE exits 0 even when it gives up after repeated task failures, so
    # the output is checked too.
    for sign in ("task failed", "rejected the result", "gave up"):
        expect(sign not in output, f"a task failed ({sign!r}):\n{output[-3000:]}")
    return output


def completed_claims(args, job_id: str) -> int:
    return int(psql(args,
        "SELECT COUNT(*) FROM task_claims c JOIN tasks t ON t.id = c.task_id "
        f"WHERE t.job_id = '{job_id}' AND c.state = 'completed'"))


def run_job(client, args, data: dict, body: dict, check) -> None:
    deactivate_everything(client)
    job_id = create_and_activate(client, args, data, body)
    output = contribute(args, args.tasks)
    claims = completed_claims(args, job_id)
    expect(claims >= args.tasks, f"{body['job_type']}: {claims} accepted claims, "
                                 f"expected at least {args.tasks}:\n{output[-3000:]}")
    check(job_id)
    client.json(client.post(f"/api/admin/jobs/{job_id}/deactivate"), "deactivate job")
    log(f"{body['job_type']}: ok ({claims} accepted claims)")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--api", default="http://localhost:8080")
    parser.add_argument("--compose-service", default="postgres")
    parser.add_argument("--backend-service", default="backend")
    parser.add_argument("--magpie", type=Path, required=True, help="the magpie binary")
    parser.add_argument("--magpie-root", type=Path, required=True,
                        help="the MAGPIE checkout: its data/ and download_data.sh")
    parser.add_argument("--workdir", type=Path, default=Path(".e2e-worker"))
    parser.add_argument("--threads", type=int, default=4)
    parser.add_argument("--tasks", type=int, default=2, help="tasks per job (default: 2)")
    parser.add_argument("--task-timeout", type=int, default=1800)
    args = parser.parse_args()

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
    data = seed.input_data_ids(client, seed_args)
    winpct = next((r["id"] for r in client.json(client.get("/api/admin/input-data"), "input data")
                   if r["role"] == "winpct"), None)
    expect(winpct is not None, "the imported data has no win% model")

    static_equity = create_player(client, args, data, "e2e-static-equity", {})
    static_score = create_player(client, args, data, "e2e-static-score",
                                 {"sort_strategy": "score"})
    simming = create_player(client, args, data, "e2e-simming", {
        "winpct_id": winpct, "num_plies": 2, "num_plays": 5, "num_plies_recorded": 2,
        "max_iterations": 60, "stopping_pct": 99,
    })

    def games_counted(job_id: str) -> None:
        stats = client.json(client.get(f"/api/jobs/{job_id}"), "job stats")
        expect(stats["games"]["units_completed"] > 0, f"no games counted: {stats['games']}")

    def pairs_counted(job_id: str) -> None:
        stats = client.json(client.get(f"/api/jobs/{job_id}"), "job stats")
        expect(sum(stats["games"]["pentanomial"]) > 0, f"no pairs counted: {stats['games']}")

    def best_moves(job_id: str) -> None:
        page = client.json(client.get(f"/api/jobs/{job_id}/results"), "results")
        expect(page["items"] and all(r["best_move"] for r in page["items"]),
               f"racks without a best move: {page['items'][:3]}")

    def simulated_statistics(job_id: str) -> None:
        best_moves(job_id)
        row = psql(args,
            "SELECT COUNT(*) FILTER (WHERE m.win_percentage IS NOT NULL), "
            "       COUNT(*) FILTER (WHERE m.blended_utility IS NOT NULL), "
            "       (SELECT COUNT(*) FROM position_analysis_plies p "
            "        JOIN position_analysis_moves pm ON pm.id = p.move_id "
            f"       WHERE pm.task_id IN (SELECT id FROM tasks WHERE job_id = '{job_id}')) "
            "FROM position_analysis_moves m JOIN tasks t ON t.id = m.task_id "
            f"WHERE t.job_id = '{job_id}'")
        win, utility, plies = (int(v) for v in row.split("|"))
        expect(win > 0 and utility > 0 and plies > 0,
               f"simulated statistics missing: win%={win} utility={utility} plies={plies}")

    lexica = args.magpie_root / "data" / "lexica"
    before = {p.name for p in lexica.iterdir()}

    def leave_occurrences(job_id: str) -> None:
        occurred = int(psql(args,
            "SELECT COUNT(*) FROM leave_rack_progress "
            f"WHERE job_id = '{job_id}' AND generation = 1 AND occurrence_count > 0"))
        expect(occurred > 0, "no rack occurrences folded into generation 1")
        short = int(psql(args,
            f"SELECT COUNT(*) FROM leave_rack_progress WHERE job_id = '{job_id}' "
            "AND length(rack) <> 7"))
        expect(short == 0, f"{short} progress rows are not full racks")
        written = sorted({p.name for p in lexica.iterdir()} - before)
        stray = [name for name in written if "_gen_" in name or name.endswith("_report.txt")]
        expect(not stray, f"leave generation wrote into MAGPIE's data directory: {stray}")

    common = {"player1_config_id": static_equity, "player2_config_id": static_score}
    run_job(client, args, data, {"job_type": "games", **common, "games_per_batch": 2,
                                 "min_games": 1000, "max_games": 1000}, games_counted)
    run_job(client, args, data, {"job_type": "game_pairs", **common, "pairs_per_batch": 2,
                                 "min_pairs": 1000, "max_pairs": 1000}, pairs_counted)
    run_job(client, args, data, {"job_type": "opening_rack", "player_config_id": static_equity,
                                 "racks_per_batch": 20, "rack_size": 7}, best_moves)
    run_job(client, args, data, {"job_type": "opening_rack", "player_config_id": simming,
                                 "racks_per_batch": 3, "rack_size": 7}, simulated_statistics)
    run_job(client, args, data, {"job_type": "leave_generation", "kwg_id": data["kwg"],
                                 "num_iterations": 20, "generation_count": 1,
                                 "target_rack_count": 1, "racks_per_task": 50,
                                 "use_wordmap": False}, leave_occurrences)

    errors = subprocess.run(
        ["docker", "compose", "logs", "--no-color", args.backend_service],
        capture_output=True, text=True,
    ).stdout
    serious = [line for line in errors.splitlines() if '"level":"ERROR"' in line or " ERROR " in line]
    expect(not serious, "backend logged errors:\n" + "\n".join(serious[-20:]))
    shutil.rmtree(args.workdir, ignore_errors=True)
    log("every job type ran end to end")
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
