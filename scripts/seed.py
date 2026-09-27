#!/usr/bin/env python3
"""Turn an empty birdtest into one with work flowing.

Drives the **real HTTP API** rather than writing SQL, so seeding is itself a
smoke test of registration, confirmation, input-data import, validation and job
creation. Two things have no endpoint and are done against the database
directly, both deliberately: promoting a user to admin (`is_admin` is settable
through no endpoint, by design) and reading the emailed confirmation code:
from the outbox file addressed to the seeded user when the stack writes mail
to files (MAIL_BACKEND=file, as the end-to-end suite's does; pass
--mail-outbox), and otherwise from the backend's log (MAIL_BACKEND=console, as
the development stack and the MAGPIE smoke tier use).

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
import urllib.parse
from pathlib import Path
from typing import List, Optional

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


CONFIRM_LINK = re.compile(r"confirm-email\?code=([0-9A-Za-z%]+)")


def outbox_suffix(email: str) -> str:
    """How MAIL_BACKEND=file names a message to `email`, after its timestamp:
    `backend/src/email.rs::outbox_file_name`, which lowercases ASCII, spells
    `@` as `-at-` and replaces anything else outside `[a-z0-9-]` with `-`."""
    recipient = "".join(c.lower() if c.isascii() else c for c in email).replace("@", "-at-")
    return "-" + re.sub(r"[^a-z0-9-]", "-", recipient) + ".txt"


def outbox_confirmation_code(outbox: Path, email: str, timeout: float = 15.0) -> str:
    """The code in the newest confirmation message to `email` in the outbox.

    Found by recipient, which is the point of the file backend: nothing else
    writing mail into the same directory at the same time can be mistaken for
    this registration. Names start with a timestamp, so the newest sorts last.
    """
    suffix = outbox_suffix(email)
    deadline = time.time() + timeout
    while True:
        for path in sorted(outbox.glob(f"*{suffix}"), reverse=True):
            codes = CONFIRM_LINK.findall(path.read_text())
            if codes:
                return urllib.parse.unquote(codes[-1])
        if time.time() > deadline:
            raise SeedError(f"no confirmation message for {email} in {outbox}")
        time.sleep(0.5)


def log_confirmation_code(backend_service: str, email: str) -> str:
    """The code the console mail backend printed, scraped from the backend log.

    It cannot come from the database: `email_confirmations` stores only a hash,
    which is the point — a leaked database dump must not hand out working
    confirmation links. So the mail is the only place the plaintext exists: a
    file under MAIL_BACKEND=file (see `outbox_confirmation_code`, which should
    be preferred wherever the stack can use it), this log under console.
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


def log_codes(backend_service: str) -> List[str]:
    """Every confirmation code in the backend's log, oldest first."""
    result = subprocess.run(["docker", "compose", "logs", "--no-color", backend_service],
                            cwd=REPO_ROOT, capture_output=True, text=True)
    return re.findall(r"confirm-email\?code=([0-9a-f]+)", result.stdout)


def new_log_code(backend_service: str, seen: set, timeout: float = 15.0) -> str:
    """A code that was not in the log before: the mail is sent off the
    registration's request, so the latest code can still be the last
    account's when the registration answers."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        fresh = [code for code in log_codes(backend_service) if code not in seen]
        if fresh:
            return fresh[-1]
        time.sleep(0.2)
    raise SeedError(f"no new confirmation code in the {backend_service} log within {timeout:.0f}s")


def promote_to_admin(compose_service: str, username: str) -> None:
    # A username may hold a quote; doubled, it is a literal. Matched as sign-in
    # matches it, whatever its case.
    literal = username.replace("'", "''")
    psql(compose_service, f"UPDATE users SET is_admin = true WHERE lower(username) = lower('{literal}')")


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
    if args.mail_outbox:
        code = outbox_confirmation_code(args.mail_outbox, args.email)
    else:
        code = log_confirmation_code(args.backend_service, args.email)
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
    outbox or the backend's log.
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


def make_contributors(args) -> None:
    """Contributor accounts, not admins, each with a new API key, written to
    --keys-out for dev.py to hand its keyed workers. A key is shown only when
    it is made, so every call makes new ones; an account that exists already
    is signed in, not registered again."""
    if not args.keys_out:
        raise SeedError("--contributors needs --keys-out, where the keys go")
    keys = []
    for index in range(1, args.contributors + 1):
        username = f"dev-contributor-{index}"
        email = f"{username}@example.invalid"
        client = Client(args.api)
        seen = set() if args.mail_outbox else set(log_codes(args.backend_service))
        registered = client.session.post(
            f"{client.api}/api/auth/register",
            json={"username": username, "email": email, "password": args.password},
            timeout=30,
        )
        if registered.status_code < 400:
            log(f"registered {username}")
            code = (outbox_confirmation_code(args.mail_outbox, email) if args.mail_outbox
                    else new_log_code(args.backend_service, seen))
            client.json(client.session.post(f"{client.api}/api/auth/confirm-email",
                                            json={"code": code}, timeout=30),
                        f"confirm {username}'s address")
        login = client.session.post(
            f"{client.api}/api/auth/login",
            json={"username": username, "password": args.password}, timeout=30,
        )
        client.json(login, f"sign in as {username}")
        created = client.json(client.post("/api/me/api-keys", {"label": "dev.py worker"}),
                              f"make an API key for {username}")
        keys.append({"username": username, "key": created["key"]})
        log(f"made an API key for {username}")
    args.keys_out.parent.mkdir(parents=True, exist_ok=True)
    args.keys_out.unlink(missing_ok=True)
    fd = os.open(args.keys_out, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as f:
        json.dump(keys, f)


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

    # A simulating player's win% model, where the data has one.
    winpct = next((row["id"] for row in rows if row["role"] == "winpct" and row["name"] == "winpct"), None)
    return {
        "kwg": find("kwg", args.lexicon),
        "klv": find("klv", args.lexicon),
        "letterdist": find("letterdist", args.letterdist),
        "layout": find("layout", args.layout),
        "winpct": winpct,
    }


# --- player configs and a job ----------------------------------------------


def player_config(
    client: Client, name: str, sort_strategy: str, data: dict, recorder: str = "best",
    sim: Optional[dict] = None, wordmap: bool = True,
) -> str:
    """A static player: no simulation parameters, and so no win% model either.

    `best` for games, where only the move played matters. An opening-rack job
    ranks moves, and job creation refuses a static `best` player that records
    more than one, so its player records `all`.

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
                "recorder_type": recorder,
                "sort_strategy": sort_strategy,
                "kwg_id": data["kwg"],
                "klv_id": data["klv"],
                "num_plays_recorded": 10,
                "use_wordmap": wordmap,
                **(sim or {}),
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
        # Small, as tier 6 runs it: one generation, a few iterations. A task
        # still takes MAGPIE a couple of minutes; the rack universe is built by
        # the first claim, not at creation.
        return {"kwg_id": args.leave_kwg, "num_iterations": 20, "generation_count": 1,
                "target_rack_count": 1, "racks_per_task": 50, "use_wordmap": args.wordmap}
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


def create_job(client: Client, args, data: dict, players: list,
               job_type: Optional[str] = None, name: Optional[str] = None) -> str:
    job_type = job_type or args.job_type
    name = args.job_name if name is None else name
    if not args.new_job:
        already = existing_active_job(client, job_type)
        if already:
            log(f"an active {job_type} job already exists ({already}); reusing it")
            return already

    body = {
        "name": name,
        "job_type": job_type,
        "variant": args.variant,
        "letterdist_id": data["letterdist"],
        "layout_id": data["layout"],
        "redundancy": args.redundancy,
        **job_config(job_type, players, args),
    }
    # A job records its own floor at creation, defaulting to the server-wide
    # one. Left implicit, a job created while the server had a higher floor
    # keeps rejecting an older local build for ever, even after the server
    # floor is lowered -- so dev passes it explicitly.
    if args.min_magpie_version:
        body["min_magpie_version"] = args.min_magpie_version
    created = client.json(client.post("/api/admin/jobs", body), "create job")
    # Creation answers with the job plus whatever state it had to build first
    # (leave generation seeds its rack universe here), not a bare id.
    job_id = created["job"]["id"]
    log(f"created {job_type} job {job_id}")

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
                             "when mail goes to the console (default: %(default)s)")
    parser.add_argument("--mail-outbox", type=Path,
                        default=os.environ.get("BIRDTEST_MAIL_OUTBOX") or None,
                        help="host directory the backend's MAIL_BACKEND=file writes into; "
                             "the confirmation code is read from there rather than the "
                             "backend log (default: $BIRDTEST_MAIL_OUTBOX)")

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
    parser.add_argument("--job-name", default="",
                        help="what to call the seeded job (shown first in the jobs list); "
                             "unnamed by default, so it is titled by its type")
    parser.add_argument("--allocation", type=int, default=100)
    parser.add_argument("--no-wordmap", dest="wordmap", action="store_false",
                        help="players (and the leave-generation bot) play without a wordmap, "
                             "so no job waits on the derived-file builder; a player config "
                             "that already exists by name is reused as it is")
    parser.add_argument("--all-job-types", action="store_true",
                        help="create a job of every type -- games, opening racks, leave "
                             "generation, and three game-pairs jobs among two static players "
                             "and a 1-ply simming one, so a rating pool can rate all three -- "
                             "each at --allocation, instead of --job-type")
    parser.add_argument("--contributors", type=int, default=0,
                        help="contributor accounts to make (dev-contributor-1, ...), each with a "
                             "new API key written to --keys-out")
    parser.add_argument("--keys-out", type=Path, default=None,
                        help="where to write the contributors' API keys, as JSON (readable by "
                             "its owner only)")
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
    args.leave_kwg = data["kwg"]

    def players_for(job_type: str) -> list:
        if job_type == "opening_rack":
            return [player_config(client, "static-equity-all", "equity", data, recorder="all",
                                  wordmap=args.wordmap)]
        if job_type == "leave_generation":
            return []
        return [
            player_config(client, "static-equity", "equity", data, wordmap=args.wordmap),
            player_config(client, "static-score", "score", data, wordmap=args.wordmap),
        ]

    if args.all_job_types:
        # Every job is made, whatever is active already: three are game pairs.
        args.new_job = True
        equity, score = players_for("game_pairs")
        if not data["winpct"]:
            raise SeedError("--all-job-types needs a win% model in the data, for its simming player")
        # A third player, stronger than either static one, on the same files:
        # three players every pair of whom has a pairs job is a set a rating
        # pool can rate, once the jobs are done.
        sim = player_config(client, "sim-1ply", "equity", data, sim={
            "winpct_id": data["winpct"], "num_plies": 1, "max_iterations": 100,
            "time_limit_secs": 0, "num_plays": 10, "use_inference": False,
        }, wordmap=args.wordmap)
        create_job(client, args, data, [equity, score], "games", "dev games")
        for (p1, n1), (p2, n2) in (((equity, "static equity"), (score, "static score")),
                                   ((equity, "static equity"), (sim, "1-ply sim")),
                                   ((score, "static score"), (sim, "1-ply sim"))):
            create_job(client, args, data, [p1, p2], "game_pairs", f"dev game pairs: {n1} vs {n2}")
        create_job(client, args, data, players_for("opening_rack"), "opening_rack", "dev opening racks")
        create_job(client, args, data, [], "leave_generation", "dev leave generation")
        log(f"seeded — six jobs, each at {args.allocation}%")
    else:
        create_job(client, args, data, players_for(args.job_type))
        log("seeded — the job is active and workers can claim")

    if args.contributors:
        make_contributors(args)


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
