#!/usr/bin/env python3
"""Turn an empty birdtest into one with work flowing.

Drives the **real HTTP API** rather than writing SQL, so seeding is itself a
smoke test of registration, confirmation, input-data import, validation and job
creation. Two things have no endpoint and are done against the database
directly, both deliberately: promoting a user to admin (`is_admin` is settable
through no endpoint, by design) and reading the emailed confirmation code
(MAIL_BACKEND=console writes it to the backend's log).

Re-running is safe: an existing user is logged into rather than re-registered,
an already-imported tarball is skipped, and player configs and jobs are reused
when they already exist under the same names.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path
from typing import Optional

import requests

REPO_ROOT = Path(__file__).resolve().parent.parent

# The lexicon, distribution and layout a seeded job runs on. NWL23 rather than
# a made-up name because lexicon names are validated by prefix and an
# unrecognised one cannot be used to create a job at all (backend/src/compat.rs).
DEFAULT_LEXICON = "NWL23"
DEFAULT_LETTERDIST = "english"
DEFAULT_LAYOUT = "standard15"


class SeedError(RuntimeError):
    pass


def log(message: str) -> None:
    print(f"[seed] {message}", flush=True)


# --- the two things that are not HTTP --------------------------------------


def psql(compose_service: str, sql: str) -> str:
    """One-shot query against the compose Postgres, tuple-only and unaligned."""
    result = subprocess.run(
        ["docker", "compose", "exec", "-T", compose_service,
         "psql", "-U", "birdtest", "-d", "birdtest", "-tAc", sql],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise SeedError(f"psql failed: {result.stderr.strip()}")
    return result.stdout.strip()


def confirmation_code(backend_service: str, email: str) -> str:
    """The code the console mail backend printed, scraped from the backend log.

    It cannot come from the database: `email_confirmations` stores only a hash,
    which is the point — a leaked database dump must not hand out working
    confirmation links. So the log is the only place the plaintext exists, and
    that is true only because MAIL_BACKEND=console.
    """
    result = subprocess.run(
        ["docker", "compose", "logs", "--no-color", backend_service],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise SeedError(f"could not read {backend_service} logs: {result.stderr.strip()}")

    # Last match wins: a re-seed against a stack that has been up a while will
    # have older codes for the same address earlier in the log.
    codes = re.findall(r"confirm-email\?code=([0-9a-f]+)", result.stdout)
    if not codes:
        raise SeedError(
            f"no confirmation code for {email} in the {backend_service} log. "
            "Is MAIL_BACKEND=console?"
        )
    return codes[-1]


def promote_to_admin(compose_service: str, username: str) -> None:
    psql(compose_service, f"UPDATE users SET is_admin = true WHERE username = '{username}'")


# --- an authenticated session ----------------------------------------------


class Client:
    """A logged-in session, carrying the CSRF token every mutating call needs.

    The double-submit check compares a cookie against a header, so the header
    is set from whatever the cookie jar holds after login.
    """

    def __init__(self, api: str):
        self.api = api.rstrip("/")
        self.session = requests.Session()

    def _headers(self) -> dict:
        token = self.session.cookies.get("birdtest_csrf")
        return {"x-csrf-token": token} if token else {}

    def get(self, path: str, **kwargs) -> requests.Response:
        return self.session.get(f"{self.api}{path}", timeout=30, **kwargs)

    def post(self, path: str, body: Optional[dict] = None) -> requests.Response:
        return self.session.post(
            f"{self.api}{path}", json=body or {}, headers=self._headers(), timeout=120
        )

    def json(self, response: requests.Response, what: str):
        if response.status_code >= 400:
            raise SeedError(f"{what}: {response.status_code} {response.text[:400]}")
        return response.json() if response.text else None


def confirm_email(client: Client, args) -> None:
    code = confirmation_code(args.backend_service, args.email)
    client.json(
        client.session.post(
            f"{client.api}/api/auth/confirm-email", json={"code": code}, timeout=30
        ),
        "confirm email",
    )
    log("confirmed the email address")


def sign_in(client: Client, args) -> None:
    """Register, confirm, promote and log in — or pick up wherever a previous
    run stopped.

    Registration deliberately does not disclose whether an address is already
    taken, so its response cannot be used to tell "new user" from "seeded
    already". The login is what decides, and an unconfirmed account is a
    recoverable state rather than a failure: the code is still sitting in the
    backend's log.
    """
    registered = client.session.post(
        f"{client.api}/api/auth/register",
        json={"username": args.username, "email": args.email, "password": args.password},
        timeout=30,
    )
    if registered.status_code < 400:
        log(f"registered {args.username}")
        confirm_email(client, args)
    else:
        log(f"{args.username} already registered")

    promote_to_admin(args.compose_service, args.username)

    def login() -> requests.Response:
        return client.session.post(
            f"{client.api}/api/auth/login",
            json={"username": args.username, "password": args.password},
            timeout=30,
        )

    response = login()
    if response.status_code == 403 and "confirm your email" in response.text:
        log("account was left unconfirmed by an earlier run; confirming")
        confirm_email(client, args)
        response = login()
    client.json(response, "login")
    log(f"signed in as {args.username} (admin)")


# --- input data -------------------------------------------------------------


def import_input_data(client: Client, args) -> None:
    """Import one MAGPIE-DATA tarball and confirm the staged diff.

    The default date comes from the contributor's own `download_data.sh`, so
    the rows the server pins are the bytes the worker actually has on disk. If
    they diverge the worker declines every task, which is a confusing way to
    spend an afternoon.
    """
    existing = client.json(client.get("/api/admin/input-data"), "list input data")
    if any(row["tarball_date"] == args.tarball_date for row in existing):
        log(f"input data {args.tarball_date} already imported")
        return

    started = client.json(
        client.post(
            "/api/admin/input-data/imports",
            {"tarball_date": args.tarball_date, "git_ref": args.git_ref},
        ),
        "start import",
    )
    import_id = started["id"]
    log(f"importing data-{args.tarball_date}.tgz at {args.git_ref} ({import_id})")

    deadline = time.time() + args.import_timeout
    while True:
        if time.time() > deadline:
            raise SeedError(f"import {import_id} did not finish in {args.import_timeout}s")
        state = client.json(client.get(f"/api/admin/input-data/imports/{import_id}"), "poll import")
        if state["state"] == "staged":
            break
        if state["state"] in ("failed", "cancelled"):
            raise SeedError(f"import {state['state']}: {state.get('error')}")
        time.sleep(2)

    client.json(client.post(f"/api/admin/input-data/imports/{import_id}/confirm"), "confirm import")
    log("import confirmed")


def input_data_ids(client: Client, args) -> dict:
    rows = client.json(client.get("/api/admin/input-data"), "list input data")

    def find(role: str, name: str) -> str:
        for row in rows:
            if row["role"] == role and row["name"] == name:
                return row["id"]
        available = sorted({f"{r['role']}/{r['name']}" for r in rows})
        raise SeedError(f"no {role} named {name!r} was imported. Available: {available}")

    return {
        "kwg": find("kwg", args.lexicon),
        "klv": find("klv", args.lexicon),
        "letterdist": find("letterdist", args.letterdist),
        "layout": find("layout", args.layout),
    }


# --- player configs and a job ----------------------------------------------


def player_config(client: Client, name: str, sort_strategy: str, data: dict) -> str:
    """A static player: no simulation parameters, and so no win% model either.

    Two static players that sort differently are the cheapest way to get a job
    with real signal in it — they choose different moves on nearly every turn,
    so a paired job's pairs diverge instead of playing out identically.
    """
    for existing in client.json(client.get("/api/admin/player-configs"), "list player configs"):
        if existing["name"] == name:
            return existing["id"]

    created = client.json(
        client.post(
            "/api/admin/player-configs",
            {
                "name": name,
                "recorder_type": "best",
                "sort_strategy": sort_strategy,
                "kwg_id": data["kwg"],
                "klv_id": data["klv"],
            },
        ),
        f"create player config {name}",
    )
    log(f"created player config {name}")
    return created["id"]


def job_config(job_type: str, players: list, args) -> dict:
    if job_type == "opening_rack":
        return {"player_config_id": players[0], "racks_per_batch": args.racks_per_batch,
                "rack_size": args.rack_size}
    if job_type == "games":
        return {"player1_config_id": players[0], "player2_config_id": players[1],
                "games_per_batch": args.batch, "min_games": args.min_units,
                "max_games": args.max_units}
    if job_type == "game_pairs":
        return {"player1_config_id": players[0], "player2_config_id": players[1],
                "pairs_per_batch": args.batch, "min_pairs": args.min_units,
                "max_pairs": args.max_units}
    if job_type == "leave_generation":
        raise SeedError(
            "leave_generation enumerates its whole leave universe at creation time — "
            "914,624 rows for real English — which is not something to do by default. "
            "Create one from /admin/jobs/new when you actually want it."
        )
    raise SeedError(f"unknown job type {job_type!r}")


def existing_active_job(client: Client, job_type: str) -> Optional[str]:
    """An active job of this type, if one is already running.

    Re-seeding should not pile up duplicate active jobs: they would split the
    allocation between identical experiments and make it unclear which one a
    worker is feeding.
    """
    page = client.json(client.get("/api/jobs"), "list jobs")
    for job in page["items"]:
        if job["job_type"] == job_type and job["status"] == "active":
            return job["id"]
    return None


def create_job(client: Client, args, data: dict, players: list) -> str:
    if not args.new_job:
        already = existing_active_job(client, args.job_type)
        if already:
            log(f"an active {args.job_type} job already exists ({already}); reusing it")
            return already

    body = {
        "job_type": args.job_type,
        "variant": args.variant,
        "letterdist_id": data["letterdist"],
        "layout_id": data["layout"],
        "redundancy": args.redundancy,
        **job_config(args.job_type, players, args),
    }
    # A job records its own floor at creation, defaulting to the server-wide
    # one. Left implicit, a job created while the server still had the shipped
    # 0.0.1 default keeps rejecting an unreleased local build for ever, even
    # after the server floor is lowered -- so dev passes it explicitly.
    if args.min_magpie_version:
        body["min_magpie_version"] = args.min_magpie_version
    created = client.json(client.post("/api/admin/jobs", body), "create job")
    # Creation answers with the job plus whatever state it had to build first
    # (leave generation seeds its rack universe here), not a bare id.
    job_id = created["job"]["id"]
    log(f"created {args.job_type} job {job_id}")

    client.json(
        client.post(f"/api/admin/jobs/{job_id}/activate", {"allocation": args.allocation}),
        "activate job",
    )
    log(f"activated it at {args.allocation}% allocation")
    return job_id


# --- defaults that come from the contributor's own MAGPIE -------------------


def default_tarball_date(magpie_root: Optional[Path]) -> Optional[str]:
    """`DATA_VERSION` out of MAGPIE's download_data.sh.

    Using the same version the contributor installed is what makes the
    server's pinned digests match the bytes on the worker's disk.
    """
    if not magpie_root:
        return None
    script = magpie_root / "download_data.sh"
    if not script.is_file():
        return None
    match = re.search(r'DATA_VERSION="(\d{8})"', script.read_text())
    return match.group(1) if match else None


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--api", default="http://localhost:8080",
                        help="birdtest API base URL (default: %(default)s)")
    parser.add_argument("--compose-service", default="postgres",
                        help="compose service holding the database (default: %(default)s)")
    parser.add_argument("--backend-service", default="backend",
                        help="compose service whose log carries the confirmation code "
                             "(default: %(default)s)")

    parser.add_argument("--username", default="dev")
    parser.add_argument("--email", default="dev@example.invalid")
    parser.add_argument("--password", default="devpassword123!")

    parser.add_argument("--magpie-root", type=Path, default=None,
                        help="MAGPIE checkout, used only to default --tarball-date")
    parser.add_argument("--tarball-date", default=None,
                        help="MAGPIE-DATA tarball, YYYYMMDD (default: DATA_VERSION from "
                             "the MAGPIE checkout's download_data.sh)")
    parser.add_argument("--git-ref", default="main",
                        help="ref to resolve the tarball at (default: %(default)s)")
    parser.add_argument("--import-timeout", type=int, default=600,
                        help="seconds to wait for the import to stage (default: %(default)s)")

    parser.add_argument("--lexicon", default=DEFAULT_LEXICON)
    parser.add_argument("--letterdist", default=DEFAULT_LETTERDIST)
    parser.add_argument("--layout", default=DEFAULT_LAYOUT)
    parser.add_argument("--variant", default="classic", choices=["classic", "wordsmog"])

    parser.add_argument("--job-type", default="game_pairs",
                        choices=["game_pairs", "games", "opening_rack"],
                        help="job to create and activate (default: %(default)s)")
    parser.add_argument("--batch", type=int, default=10,
                        help="games or pairs per task (default: %(default)s)")
    parser.add_argument("--min-units", type=int, default=100,
                        help="games/pairs before SPRT is acted on (default: %(default)s)")
    parser.add_argument("--max-units", type=int, default=100000,
                        help="hard cap on games/pairs (default: %(default)s)")
    parser.add_argument("--racks-per-batch", type=int, default=500,
                        help="opening_rack only (default: %(default)s)")
    parser.add_argument("--rack-size", type=int, default=7,
                        help="opening_rack only (default: %(default)s)")
    parser.add_argument("--min-magpie-version", default=None,
                        help="version floor recorded on the job (default: the server-wide "
                             "floor). Set this to your own build's version so an unreleased "
                             "MAGPIE can claim.")
    parser.add_argument("--new-job", action="store_true",
                        help="always create a job, even if an active one of this type exists")
    parser.add_argument("--redundancy", type=int, default=1)
    parser.add_argument("--allocation", type=int, default=100)
    return parser


def seed(args) -> None:
    if not args.tarball_date:
        args.tarball_date = default_tarball_date(args.magpie_root)
    if not args.tarball_date:
        raise SeedError(
            "no --tarball-date, and DATA_VERSION could not be read from a MAGPIE checkout. "
            "Pass --tarball-date YYYYMMDD, or --magpie-root pointing at one."
        )

    client = Client(args.api)
    sign_in(client, args)
    import_input_data(client, args)
    data = input_data_ids(client, args)

    players = [
        player_config(client, "static-equity", "equity", data),
        player_config(client, "static-score", "score", data),
    ]
    create_job(client, args, data, players)
    log("seeded — the job is active and workers can claim")


def main() -> int:
    args = build_parser().parse_args()
    try:
        seed(args)
    except SeedError as err:
        print(f"[seed] {err}", file=sys.stderr)
        return 1
    except requests.RequestException as err:
        print(f"[seed] cannot reach {args.api}: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
