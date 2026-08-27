#!/usr/bin/env python3
"""birdtest worker client — claims tasks, invokes MAGPIE, submits results.

The client owns no business logic of its own: it asks the server what to do,
shells out to MAGPIE, and posts back what MAGPIE said. All scheduling,
validation and aggregation happen server-side.

MAGPIE is an external dependency. The contributor supplies a path to an
already-built MAGPIE directory (`--magpie-dir`); this client never builds,
fetches or updates it. The command lines and output formats below follow
MAGPIE's documented CLI; if a MAGPIE release changes them, the parsers here are
what need updating, and `min_magpie_version` on a job is how the server keeps
older clients from silently producing garbage.
"""

import argparse
import csv
import io
import json
import logging
import os
import subprocess
import sys
import tempfile
import threading
import time
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

import requests
import tomllib

__version__ = "1.0.0"

logger = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------


@dataclass
class Config:
    server_url: str
    magpie_dir: Path
    api_key: Optional[str]  # None → anonymous worker (X-Worker-UUID header)
    worker_uuid: str  # persistent across runs; generated on first run
    heartbeat_interval: int = 30  # seconds between heartbeats
    retry_delay_seconds: int = 5  # seconds to wait when the server has no work

    @property
    def magpie_bin(self) -> Path:
        return self.magpie_dir / "bin" / "magpie"


def _load_or_generate_uuid(state_dir: Path) -> str:
    """Read persistent UUID from disk; generate and save one if absent."""
    path = state_dir / "worker_uuid"
    if path.exists():
        return path.read_text().strip()
    worker_uuid = str(uuid.uuid4())
    state_dir.mkdir(parents=True, exist_ok=True)
    path.write_text(worker_uuid)
    return worker_uuid


def _load_config(args: argparse.Namespace) -> Config:
    """Merge TOML config file with CLI flags; flags take precedence."""
    file_values: dict = {}
    if args.config and args.config.exists():
        with args.config.open("rb") as handle:
            file_values = tomllib.load(handle)

    def pick(name: str, default=None):
        value = getattr(args, name, None)
        if value is not None:
            return value
        return file_values.get(name, default)

    magpie_dir = pick("magpie_dir")
    if magpie_dir is None:
        raise SystemExit(
            "--magpie-dir (or magpie_dir in the config file) is required: "
            "point it at a directory containing an already-built MAGPIE"
        )

    state_dir = Path(file_values.get("state_dir", "~/.birdtest")).expanduser()
    return Config(
        server_url=str(pick("server_url", "http://localhost:8080")).rstrip("/"),
        magpie_dir=Path(magpie_dir).expanduser(),
        api_key=pick("api_key"),
        worker_uuid=_load_or_generate_uuid(state_dir),
        heartbeat_interval=int(file_values.get("heartbeat_interval", 30)),
        retry_delay_seconds=int(file_values.get("retry_delay_seconds", 5)),
    )


# ---------------------------------------------------------------------------
# HTTP helpers
# ---------------------------------------------------------------------------


def _auth_headers(cfg: Config) -> dict:
    if cfg.api_key:
        return {"Authorization": f"Bearer {cfg.api_key}"}
    return {"X-Worker-UUID": cfg.worker_uuid}


def _check_for_self_update(cfg: Config) -> None:
    """
    GET /api/worker/client-version. If the server reports a version different from
    __version__, download the new script to a temp file and re-exec this process with
    it, passing along all original argv. Does not return if an update is applied.
    """
    info = requests.get(f"{cfg.server_url}/api/worker/client-version", timeout=30).json()
    if info["version"] == __version__:
        return
    logger.info("Updating worker %s → %s", __version__, info["version"])
    with tempfile.NamedTemporaryFile(suffix=".py", delete=False) as tmp:
        tmp.write(requests.get(info["download_url"], timeout=120).content)
        tmp_path = tmp.name
    os.chmod(tmp_path, 0o755)
    os.execv(sys.executable, [sys.executable, tmp_path] + sys.argv[1:])


def _get_magpie_version(cfg: Config) -> str:
    """Run `magpie version` once and return the version string. Cached by the caller."""
    result = subprocess.run(
        [str(cfg.magpie_bin), "version"],
        capture_output=True,
        text=True,
        check=True,
    )
    return result.stdout.strip()


def _parse_semver(version: str) -> tuple[int, ...]:
    """Parses a "X.Y.Z" string into a tuple of ints for correct numeric comparison —
    plain string comparison is wrong here ("1.10.0" < "1.9.0" as strings)."""
    return tuple(int(part) for part in version.split("."))


def _claim_task(cfg: Config) -> Optional[dict]:
    """POST /api/worker/task. Returns parsed body, or None on 204 (no work available)."""
    response = requests.post(
        f"{cfg.server_url}/api/worker/task", headers=_auth_headers(cfg), timeout=60
    )
    if response.status_code == 204:
        return None
    if response.status_code == 429:
        # The server's per-identity limit is one request a second; back off
        # rather than hammering it.
        time.sleep(int(response.headers.get("Retry-After", "1")))
        return None
    response.raise_for_status()
    return response.json()


def _send_heartbeat(cfg: Config, claim_token: str) -> None:
    """POST /api/worker/heartbeat. Pure liveness ping — no payload; leave-gen progress
    is derived server-side from accepted task results, not from heartbeats."""
    requests.post(
        f"{cfg.server_url}/api/worker/heartbeat",
        headers=_auth_headers(cfg),
        json={"claim_token": claim_token},
        timeout=30,
    ).raise_for_status()


def _submit_result(cfg: Config, claim_token: str, result: dict) -> None:
    """POST /api/worker/result."""
    response = requests.post(
        f"{cfg.server_url}/api/worker/result",
        headers=_auth_headers(cfg),
        json={"claim_token": claim_token, "result": result},
        timeout=120,
    )
    response.raise_for_status()
    if not response.json().get("accepted", False):
        # The claim timed out while we were working and the task went back into
        # the pool. Nothing to do but pick up the next one.
        logger.warning("Server rejected the result as stale; the claim had expired")


# ---------------------------------------------------------------------------
# MAGPIE invocation
# ---------------------------------------------------------------------------


def _player_args(player: dict, slot: int) -> list[str]:
    """Flatten one player config into MAGPIE's per-player arguments.

    The slot number is the suffix on every flag (`-r1`/`-r2`, `-s1`/`-s2`, …).
    Simulation flags are omitted entirely for a static player, where they are all
    null.
    """
    args = ["-r%d" % slot, player["recorder_type"]]
    optional = {
        "sort_strategy": "-s%d",
        "leaves": "-k%d",
        "max_iterations": "-i%d",
        "plies": "-pl%d",
        "top_plays": "-np%d",
        "stopping_pct": "-sc%d",
        "time_limit_secs": "-tl%d",
    }
    for key, flag in optional.items():
        value = player.get(key)
        if value is not None:
            args += [flag % slot, str(value)]
    if player.get("use_inference") is not None:
        args += ["-si%d" % slot, "true" if player["use_inference"] else "false"]
    return args


def _run_magpie(cfg: Config, args: list[str]) -> str:
    """Run MAGPIE and return stdout, raising on a non-zero exit."""
    command = [str(cfg.magpie_bin), *args]
    logger.debug("running %s", " ".join(command))
    result = subprocess.run(command, capture_output=True, text=True, cwd=cfg.magpie_dir)
    if result.returncode != 0:
        raise RuntimeError(
            f"MAGPIE exited {result.returncode}: {result.stderr.strip() or result.stdout.strip()}"
        )
    return result.stdout


def _parse_json_output(raw: str) -> dict:
    """MAGPIE with `-hr false` emits machine-readable JSON on stdout.

    Anything before the first `{` is log noise, so the parse starts there rather
    than assuming the whole stream is the document.
    """
    start = raw.find("{")
    if start < 0:
        raise ValueError(f"no JSON object in MAGPIE output: {raw[:200]!r}")
    return json.loads(raw[start:])


# ---------------------------------------------------------------------------
# Task handlers — one function per job type
# ---------------------------------------------------------------------------


def _handle_opening_rack(request: dict, cfg: Config) -> dict:
    """Invoke MAGPIE to analyze a single opening rack position."""
    args = [
        "sim" if request["player"].get("max_iterations") else "gen",
        "-lex", request["lexicon"],
        "-var", request["variant"],
        "-cgp", request["position"],
        "-hr", "false",
        *_player_args(request["player"], 1),
    ]
    if request.get("previous_play"):
        # Required whenever inference is enabled — the inference needs to know
        # what the opponent just did.
        args += ["-previousplay", request["previous_play"]]

    output = _parse_json_output(_run_magpie(cfg, args))
    moves = []
    for entry in output["moves"]:
        move = {
            "move": entry["move"],
            "score": int(entry["score"]),
            "equity": float(entry["equity"]),
        }
        if entry.get("plies"):
            move["plies"] = [
                {
                    "ply": int(p["ply"]),
                    "bingo_percentage": float(p["bingo_percentage"]),
                    "average_score": float(p["average_score"]),
                }
                for p in entry["plies"]
            ]
        moves.append(move)
    return {"moves": moves}


def _autoplay(request: dict, cfg: Config, extra: list[str]) -> dict:
    """Shared driver for the games and game-pairs handlers."""
    args = [
        "autoplay",
        "-lex", request["lexicon"],
        "-var", request["variant"],
        "-seed", str(request["seed"]),
        "-gms", str(request["num_games"]),
        "-hr", "false",
        *_player_args(request["player1"], 1),
        *_player_args(request["player2"], 2),
        *extra,
    ]
    output = _parse_json_output(_run_magpie(cfg, args))
    games = [
        {
            "score1": int(g["score1"]),
            "score2": int(g["score2"]),
            "winner": int(g["winner"]),
            "num_turns": int(g["num_turns"]),
        }
        for g in output["games"]
    ]
    return {"games": games}


def _handle_game(request: dict, cfg: Config) -> dict:
    """Invoke MAGPIE autoplay to run a batch of games."""
    return _autoplay(request, cfg, [])


def _handle_game_pair(request: dict, cfg: Config) -> dict:
    """Invoke MAGPIE autoplay with -gp true for a batch of game pairs.

    MAGPIE plays both orderings from the same seed, emitting two games per pair
    in order, which is exactly the shape the server stores.
    """
    return _autoplay(request, cfg, ["-gp", "true"])


def _handle_leave_gen(request: dict, cfg: Config) -> dict:
    """Download previous-gen leaves from S3, write the forced-rack subset to a local
    file, run autoplay with -forceracksfile, parse the resulting rack-equity CSV (every
    rack that occurred, forced or not), and return it as an inline {rack, count, mean}
    list — the CSV itself is scratch and is not uploaded anywhere."""
    with tempfile.TemporaryDirectory(prefix="birdtest-leavegen-") as scratch:
        scratch = Path(scratch)
        racks_file = scratch / "forced_racks.txt"
        racks_file.write_text("\n".join(request["forced_racks"]) + "\n")
        csv_path = scratch / "rack_equity.csv"

        args = [
            "autoplay",
            "-lex", request["lexicon"],
            "-var", request["variant"],
            "-gms", str(request["num_games"]),
            "-forceracksfile", str(racks_file),
            "-writerackequitycsv", "true",
            "-rackequitycsvpath", str(csv_path),
            "-hr", "false",
        ]

        previous = request.get("previous_artifact_key")
        if previous:
            # Generation N > 1 plays with the KLV the server built from
            # generation N-1. Generation 1 has no artifact and MAGPIE falls back
            # to the lexicon's default leaves.
            leaves_path = scratch / "previous.klv2"
            _download_artifact(cfg, previous, leaves_path)
            args += ["-k1", str(leaves_path), "-k2", str(leaves_path)]

        _run_magpie(cfg, args)

        if not csv_path.exists():
            raise RuntimeError(f"MAGPIE did not write the rack-equity CSV at {csv_path}")
        return {"racks": _parse_rack_equity_csv(csv_path.read_text())}


def _parse_rack_equity_csv(text: str) -> list[dict]:
    """Parse MAGPIE's `<rack>,<count>,<mean>` output.

    Every rack that occurred during the batch is here, forced or not: racks the
    games happen to draw naturally count toward their occurrence target too.
    """
    racks = []
    for row in csv.reader(io.StringIO(text)):
        if not row or row[0].startswith("#"):
            continue
        if len(row) < 3:
            raise ValueError(f"malformed rack-equity row: {row!r}")
        racks.append({"rack": row[0].strip(), "count": int(row[1]), "mean": float(row[2])})
    if not racks:
        raise ValueError("rack-equity CSV contained no rows")
    return racks


def _download_artifact(cfg: Config, key: str, destination: Path) -> None:
    """Fetch a stored artifact (a previous generation's combined KLV)."""
    response = requests.get(
        f"{cfg.server_url}/api/worker/artifact",
        headers=_auth_headers(cfg),
        params={"key": key},
        timeout=300,
    )
    response.raise_for_status()
    destination.write_bytes(response.content)


_HANDLERS = {
    "opening_rack_analysis": _handle_opening_rack,
    "games":                 _handle_game,
    "game_pairs":            _handle_game_pair,
    "leave_generation":      _handle_leave_gen,
}

# ---------------------------------------------------------------------------
# Heartbeat thread
# ---------------------------------------------------------------------------


def _heartbeat_loop(
    cfg: Config,
    claim_token: str,
    stop: threading.Event,
) -> None:
    while not stop.wait(timeout=cfg.heartbeat_interval):
        try:
            _send_heartbeat(cfg, claim_token)
        except Exception:
            logger.warning("Heartbeat failed", exc_info=True)


# ---------------------------------------------------------------------------
# Main loop
# ---------------------------------------------------------------------------


def _worker_loop(cfg: Config, magpie_version: str) -> None:
    while True:
        # 1. Claim
        try:
            response = _claim_task(cfg)
        except Exception:
            logger.warning("Could not claim a task; retrying", exc_info=True)
            time.sleep(cfg.retry_delay_seconds)
            continue

        if response is None:
            time.sleep(cfg.retry_delay_seconds)
            continue

        claim_token = response["claim_token"]
        task_request = response["task_request"]
        min_ver = response.get("min_magpie_version")

        # 2. Version gate — skip task if MAGPIE is too old; claim expires server-side
        # Compared as a tuple of ints (_parse_semver), not string order: "1.10.0" < "1.9.0"
        # as strings, which is backwards.
        if min_ver and _parse_semver(magpie_version) < _parse_semver(min_ver):
            logger.error(
                "MAGPIE %s < required %s for this job; skipping task", magpie_version, min_ver
            )
            time.sleep(cfg.retry_delay_seconds)
            continue

        handler = _HANDLERS[task_request["job_type"]]

        # 3. Heartbeat
        stop = threading.Event()
        hb = threading.Thread(
            target=_heartbeat_loop,
            args=(cfg, claim_token, stop),
            daemon=True,
        )
        hb.start()

        result = None
        try:
            # 4. Execute
            result = handler(task_request, cfg)
        except Exception:
            logger.exception("Task execution failed; claim will expire server-side")
        finally:
            stop.set()
            hb.join()

        # 5. Submit
        if result is not None:
            try:
                _submit_result(cfg, claim_token, result)
            except Exception:
                logger.exception("Could not submit the result; claim will expire server-side")


def _parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description="birdtest worker client")
    p.add_argument("--config", type=Path, default=Path("~/.birdtest/config.toml").expanduser())
    p.add_argument("--magpie-dir", type=Path)
    p.add_argument("--api-key")
    p.add_argument("--server-url")
    return p.parse_args()


def main() -> None:
    args = _parse_args()
    cfg = _load_config(args)
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")

    # 1. Self-update check — re-execs this process if a newer script is available
    _check_for_self_update(cfg)

    # 2. Cache MAGPIE version once at startup
    magpie_version = _get_magpie_version(cfg)
    logger.info(
        "Worker started (uuid=%s, authenticated=%s, magpie=%s, client=%s)",
        cfg.worker_uuid, cfg.api_key is not None, magpie_version, __version__,
    )

    _worker_loop(cfg, magpie_version)


if __name__ == "__main__":
    main()
