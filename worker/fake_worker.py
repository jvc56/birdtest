#!/usr/bin/env python3
"""Synthetic birdtest worker — speaks the worker API without running MAGPIE.

This exists so server-side behaviour can be tested at speed and on purpose.
Real games take real time and produce results nobody chose; almost every
interesting server property is about something else:

  * scheduling — priority tiers, deficit-based allocation, redundancy
  * the claim lifecycle — heartbeat timeouts, stale tokens, reclamation
  * SPRT and Glicko — which need a *chosen* win rate to reach a known verdict
  * anomaly detection — which needs a client that misbehaves deliberately

None of those want a real engine in the loop. `worker.py` is the reference
client that does; this is its counterpart for everything else.

Every mode is deterministic under `--seed`, so a failing CI run reproduces.
"""

import argparse
import json
import logging
import random
import sys
import threading
import time
import uuid
from dataclasses import dataclass, field
from typing import Optional

import requests

logger = logging.getLogger("fake-worker")


@dataclass
class Stats:
    claimed: int = 0
    submitted: int = 0
    accepted: int = 0
    rejected: int = 0
    no_work: int = 0
    rate_limited: int = 0
    errors: int = 0
    lock: threading.Lock = field(default_factory=threading.Lock)

    def bump(self, name: str) -> None:
        with self.lock:
            setattr(self, name, getattr(self, name) + 1)

    def summary(self) -> str:
        return (
            f"claimed={self.claimed} submitted={self.submitted} "
            f"accepted={self.accepted} rejected={self.rejected} "
            f"no_work={self.no_work} rate_limited={self.rate_limited} "
            f"errors={self.errors}"
        )


# ---------------------------------------------------------------------------
# Synthetic results
# ---------------------------------------------------------------------------


def _aggregate(rng: random.Random, games: int, p1_win_probability: float) -> dict:
    """One synthetic `autoplay` summary — the shape MAGPIE actually reports.

    Draws each game's outcome so the counts have realistic sampling noise
    rather than being the exact expectation, which is what makes SPRT runs
    interesting.
    """
    wins = losses = ties = 0
    for _ in range(games):
        roll = rng.random()
        if roll < p1_win_probability:
            wins += 1
        elif roll < p1_win_probability + 0.02:
            ties += 1  # draws are rare but must be exercised
        else:
            losses += 1

    return {
        "games": games,
        "wins": wins,
        "losses": losses,
        "ties": ties,
        "p1_score_mean": round(rng.uniform(400, 460), 6),
        "p1_score_sd": round(rng.uniform(45, 70), 6),
        "p2_score_mean": round(rng.uniform(400, 460), 6),
        "p2_score_sd": round(rng.uniform(45, 70), 6),
    }


def _result_for(request: dict, rng: random.Random, p1_win_probability: float) -> dict:
    job_type = request["job_type"]

    if job_type == "games":
        return {"all_games": _aggregate(rng, request["num_games"], p1_win_probability)}

    if job_type == "game_pairs":
        # Two games per pair. Pairs whose games played identically are
        # guaranteed ties and are excluded from the divergent subset, which is
        # what the server computes the LLR from — so the divergent count is a
        # fraction of the total, and the outcomes live there.
        games = request["num_games"] * 2
        divergent = max(2, (int(games * rng.uniform(0.5, 1.0)) // 2) * 2)
        identical_pairs = (games - divergent) // 2
        divergent_agg = _aggregate(rng, divergent, p1_win_probability)
        all_games = {
            **divergent_agg,
            "games": games,
            # Each identical pair contributes one win to each side: the same
            # game played from both seats.
            "wins": divergent_agg["wins"] + identical_pairs,
            "losses": divergent_agg["losses"] + identical_pairs,
        }
        return {"all_games": all_games, "divergent_games": divergent_agg}

    if job_type == "opening_rack_analysis":
        rack = request["position"].split()[1].rstrip("/")
        count = rng.randint(2, 6)
        moves = []
        equity = rng.uniform(20.0, 45.0)
        for i in range(count):
            equity -= rng.uniform(0.5, 4.0)
            moves.append(
                {
                    "move": f"8{chr(ord('D') + i)} {rack[: rng.randint(2, len(rack))]}",
                    "score": rng.randint(12, 90),
                    "equity": round(equity, 3),
                    "plies": [
                        {
                            "ply": p,
                            "bingo_percentage": round(rng.uniform(0, 25), 3),
                            "average_score": round(rng.uniform(25, 45), 3),
                        }
                        for p in range(2)
                    ],
                }
            )
        return {"moves": moves}

    if job_type == "leave_generation":
        return {
            "racks": [
                {
                    "rack": rack,
                    "count": rng.randint(1, 12),
                    "mean": round(rng.uniform(-8.0, 32.0), 3),
                }
                for rack in request["forced_racks"]
            ]
        }

    raise ValueError(f"unknown job type {job_type!r}")


def _corrupt(result: dict, rng: random.Random) -> dict:
    """Produce a submission the server should reject with 400.

    Each variant violates a different rule, so a run with enough tasks
    exercises all of them.
    """
    variants = [
        "wrong_type", "missing_field", "inconsistent_counts", "odd_pair_count", "empty",
    ]
    choice = rng.choice(variants)

    if choice == "empty":
        return {}
    if choice == "wrong_type":
        return {"all_games": "not-an-object", "moves": "not-a-list", "racks": "not-a-list"}
    if choice == "missing_field" and result.get("all_games"):
        stripped = dict(result["all_games"])
        stripped.pop("wins", None)
        return {**result, "all_games": stripped}
    if choice == "inconsistent_counts" and result.get("all_games"):
        # wins + losses + ties must equal games.
        broken = {**result["all_games"], "wins": result["all_games"]["wins"] + 7}
        return {**result, "all_games": broken}
    if choice == "odd_pair_count" and result.get("all_games"):
        # A game_pairs task must report an even number of games, two per pair.
        broken = {**result["all_games"]}
        broken["games"] += 1
        broken["wins"] += 1
        return {**result, "all_games": broken}
    return {"unexpected": True}


# ---------------------------------------------------------------------------
# One simulated worker
# ---------------------------------------------------------------------------


class FakeWorker:
    # A task costs two requests and the worker endpoints are rate limited per
    # identity, so throttling is expected under load.
    RATE_LIMIT_RETRIES = 5

    def __init__(self, args: argparse.Namespace, index: int, stats: Stats):
        self.args = args
        self.stats = stats
        self.worker_uuid = str(uuid.uuid4())
        self.session = requests.Session()
        # Each simulated worker gets its own stream so concurrent runs stay
        # reproducible regardless of thread interleaving.
        self.rng = random.Random(f"{args.seed}:{index}")

    @property
    def headers(self) -> dict:
        if self.args.api_key:
            return {"Authorization": f"Bearer {self.args.api_key}"}
        return {"X-Worker-UUID": self.worker_uuid}

    def _url(self, path: str) -> str:
        return f"{self.args.server_url.rstrip('/')}{path}"

    def claim(self) -> Optional[dict]:
        response = self.session.post(
            self._url("/api/worker/task"), headers=self.headers, timeout=30
        )
        if response.status_code == 204:
            self.stats.bump("no_work")
            return None
        if response.status_code == 429:
            self.stats.bump("rate_limited")
            time.sleep(float(response.headers.get("Retry-After", "1")))
            return None
        response.raise_for_status()
        self.stats.bump("claimed")
        return response.json()

    def submit(self, claim_token: str, result: dict) -> None:
        # Back off and retry rather than counting a throttle as a rejection —
        # "the server rate limited me" and "the server refused my data" are the
        # distinction this whole harness exists to make.
        for attempt in range(self.RATE_LIMIT_RETRIES):
            response = self.session.post(
                self._url("/api/worker/result"),
                headers=self.headers,
                json={"claim_token": claim_token, "result": result},
                timeout=60,
            )
            if response.status_code != 429:
                break
            self.stats.bump("rate_limited")
            if attempt == self.RATE_LIMIT_RETRIES - 1:
                return
            time.sleep(float(response.headers.get("Retry-After", "1")))

        self.stats.bump("submitted")

        if response.status_code >= 400:
            # Expected in `malformed` mode; a finding anywhere else.
            self.stats.bump("rejected")
            logger.debug("submission rejected: %s %s", response.status_code, response.text[:200])
            return

        # A stale claim token is silently ignored by design, and reported as
        # accepted=false rather than as an error.
        if response.json().get("accepted"):
            self.stats.bump("accepted")
        else:
            self.stats.bump("rejected")

    def run(self, deadline: Optional[float]) -> None:
        completed = 0
        while self.args.tasks == 0 or completed < self.args.tasks:
            if deadline and time.monotonic() > deadline:
                return
            try:
                assignment = self.claim()
            except Exception:
                self.stats.bump("errors")
                logger.warning("claim failed", exc_info=True)
                time.sleep(self.args.idle_wait)
                continue

            if assignment is None:
                if self.args.stop_when_idle:
                    return
                time.sleep(self.args.idle_wait)
                continue

            token = assignment["claim_token"]
            request = assignment["task_request"]

            if self.args.mode == "abandon":
                # Claim and never submit, so the heartbeat timeout has to
                # reclaim the task. Pair with a short HEARTBEAT_TIMEOUT_SECONDS.
                completed += 1
                continue

            try:
                result = _result_for(request, self.rng, self.args.p1_win_rate)
            except Exception:
                self.stats.bump("errors")
                logger.exception("could not build a synthetic result")
                continue

            if self.args.mode == "malformed":
                result = _corrupt(result, self.rng)
            if self.args.mode == "stale":
                # A token that was never issued must be ignored, not accepted.
                token = str(uuid.uuid4())

            if self.args.work_seconds:
                time.sleep(self.args.work_seconds)

            try:
                self.submit(token, result)
            except Exception:
                self.stats.bump("errors")
                logger.warning("submit failed", exc_info=True)

            completed += 1
            if self.args.delay:
                time.sleep(self.args.delay)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Synthetic birdtest worker for testing the server without MAGPIE."
    )
    parser.add_argument("--server-url", default="http://localhost:8080")
    parser.add_argument("--api-key", help="authenticate instead of running anonymously")
    parser.add_argument(
        "--workers", type=int, default=1,
        help="simulated workers running concurrently, for exercising claim races",
    )
    parser.add_argument(
        "--tasks", type=int, default=1,
        help="tasks each worker completes; 0 runs until interrupted",
    )
    parser.add_argument(
        "--mode",
        choices=["normal", "malformed", "stale", "abandon"],
        default="normal",
        help=(
            "normal: plausible results. malformed: submissions the server should "
            "reject. stale: submit under a claim token that was never issued. "
            "abandon: claim and never submit, so the heartbeat timeout reclaims."
        ),
    )
    parser.add_argument(
        "--p1-win-rate", type=float, default=0.5,
        help="bias player 1's results, to drive SPRT to a chosen verdict",
    )
    parser.add_argument("--seed", default="birdtest", help="makes a run reproducible")
    parser.add_argument("--delay", type=float, default=0.0, help="seconds between tasks")
    parser.add_argument(
        "--work-seconds", type=float, default=0.0,
        help="pretend a task takes this long, to hold a claim open",
    )
    parser.add_argument("--idle-wait", type=float, default=1.0)
    parser.add_argument(
        "--stop-when-idle", action="store_true",
        help="exit on the first 204 instead of waiting for more work",
    )
    parser.add_argument("--timeout", type=float, default=0.0, help="give up after N seconds")
    parser.add_argument("--verbose", action="store_true")
    args = parser.parse_args()

    logging.basicConfig(
        level=logging.DEBUG if args.verbose else logging.INFO,
        format="%(asctime)s %(levelname)s %(message)s",
    )

    stats = Stats()
    deadline = time.monotonic() + args.timeout if args.timeout else None
    workers = [FakeWorker(args, i, stats) for i in range(args.workers)]
    threads = [
        threading.Thread(target=w.run, args=(deadline,), daemon=True) for w in workers
    ]

    started = time.monotonic()
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()

    elapsed = time.monotonic() - started
    logger.info("%s in %.1fs", stats.summary(), elapsed)
    print(json.dumps({
        "claimed": stats.claimed,
        "submitted": stats.submitted,
        "accepted": stats.accepted,
        "rejected": stats.rejected,
        "no_work": stats.no_work,
        "rate_limited": stats.rate_limited,
        "errors": stats.errors,
        "elapsed_seconds": round(elapsed, 3),
    }))

    # Non-zero on transport failures only. A rejection is a *result* here, not
    # an error: `malformed` and `stale` runs expect them.
    sys.exit(1 if stats.errors else 0)


if __name__ == "__main__":
    main()
