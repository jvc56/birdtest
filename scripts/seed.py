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
        if state["state"] == "nothing_new":
            # Every file already known (under another date): nothing to confirm.
            log(f"input data {args.tarball_date} has nothing new")
            return
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
    sim: Optional[dict] = None, wordmap: bool = True, rit: bool = True,
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
                # A table is built from the wordmap, so no wordmap is no table.
                "use_rit": rit and wordmap,
                # Off with the wordmap: --no-wordmap means no job waits on the
                # derived-file builder, which builds this table too.
                "use_wit": wordmap,
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
                "games_per_batch": args.batch, "sprt_enabled": True,
                "min_games": 100 if args.min_units is None else args.min_units,
                "max_games": args.max_units}
    if job_type == "game_pairs":
        return {"player1_config_id": players[0], "player2_config_id": players[1],
                "pairs_per_batch": args.batch, "sprt_enabled": True,
                "min_pairs": 50000 if args.min_units is None else args.min_units,
                "max_pairs": args.max_units}
    if job_type == "leave_generation":
        # Small, as tier 6 runs it: one generation, a few iterations. A task
        # still takes MAGPIE a couple of minutes; the rack universe is built by
        # the first claim, not at creation.
        return {"player_config_id": players[0], "num_iterations": 20, "target_rack_counts": [1],
                "racks_per_task": 50}
    raise SeedError(f"unknown job type {job_type!r}")


def active_jobs(client: Client) -> List[dict]:
    """Every active job, through as many pages as there are."""
    jobs: List[dict] = []
    page = 0
    while True:
        listed = client.json(client.get(f"/api/jobs?status=active&per_page=100&page={page}"),
                             "list active jobs")
        jobs += listed["items"]
        if len(listed["items"]) < 100:
            return jobs
        page += 1


def existing_active_job(client: Client, job_type: str, name: str = "") -> Optional[str]:
    """An active job of this type -- and this name, when it has one -- if one
    is already running.

    Re-seeding should not pile up duplicate active jobs: they would split the
    allocation between identical experiments and make it unclear which one a
    worker is feeding.
    """
    for job in active_jobs(client):
        if job["job_type"] == job_type and (not name or job["name"] == name):
            return job["id"]
    return None


def create_job(client: Client, args, data: dict, players: list,
               job_type: Optional[str] = None, name: Optional[str] = None,
               extra: Optional[dict] = None, allocation: Optional[int] = None) -> str:
    job_type = job_type or args.job_type
    name = args.job_name if name is None else name
    allocation = args.allocation if allocation is None else allocation
    if not args.new_job:
        already = existing_active_job(client, job_type, name)
        if already:
            log(f"an active {job_type} job already exists ({already}); reusing it")
            return already

    body = {
        "name": name,
        "job_type": job_type,
        "variant": args.variant,
        "letterdist_id": data["letterdist"],
        "layout_id": data["layout"],
        **job_config(job_type, players, args),
        **(extra or {}),
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
        client.post(f"/api/admin/jobs/{job_id}/activate", {"allocation": allocation}),
        "activate job",
    )
    log(f"activated it at {allocation}% allocation")
    return job_id


# --- the jobs dev.py can start with ------------------------------------------

# Each a job of its own, so any of them can be asked for together. Each runs on
# the main data (--lexicon, on the english distribution), or, with `_ab`
# appended, on MAGPIE's two-letter test data (`english_ab`, eight possible full
# racks), where it finishes in minutes.
#
# Each: the job type it creates, whether its players simulate, its name, and
# what its name says it keeps.
DEV_JOB_KINDS = {
    "leave_generation": ("leave_generation", False, "dev leave generation", None),
    "opening_rack": ("opening_rack", False, "dev opening racks", None),
    "games": ("games", False, "dev games", "positions saved"),
    "games_sim": ("games", True, "dev sim games", "positions saved"),
    "game_pairs": ("game_pairs", False, "dev game pairs", "first divergences saved"),
    "game_pairs_sim": ("game_pairs", True, "dev sim game pairs", "first divergences saved"),
}
SMALL_SUFFIX = "_ab"
DEV_JOBS = tuple(job + suffix for job in DEV_JOB_KINDS for suffix in ("", SMALL_SUFFIX))


def dev_job_kind(job: str) -> str:
    """The DEV_JOB_KINDS entry one of DEV_JOBS is."""
    return job[:-len(SMALL_SUFFIX)] if job.endswith(SMALL_SUFFIX) else job


def dev_job_type(job: str) -> str:
    """The job type one of DEV_JOBS creates."""
    return DEV_JOB_KINDS[dev_job_kind(job)][0]


def dev_job_name(job: str) -> str:
    _, _, name, keeps = DEV_JOB_KINDS[dev_job_kind(job)]
    notes = (["english_ab"] if job.endswith(SMALL_SUFFIX) else []) + ([keeps] if keeps else [])
    return f"{name} ({', '.join(notes)})" if notes else name


def small_input_data(client: Client, args, layout: str) -> dict:
    """Imports MAGPIE's two-letter test data and returns its ids. It is served
    by dev.py's GitHub stand-in, under a date and ref no MAGPIE-DATA release
    has; it brings no board, so the main data's is used."""
    if not args.small_tarball_date or not args.small_git_ref:
        raise SeedError("the english_ab jobs need --small-tarball-date and --small-git-ref, "
                        "the version dev.py serves the two-letter test data as")
    import_input_data(client, argparse.Namespace(
        tarball_date=args.small_tarball_date, git_ref=args.small_git_ref,
        import_timeout=args.import_timeout))
    rows = client.json(client.get("/api/admin/input-data"), "list input data")

    def find(role: str, name: str) -> str:
        for row in rows:
            if row["role"] == role and row["name"] == name:
                return row["id"]
        raise SeedError(f"the two-letter test data has no {role} {name}")

    return {"kwg": find("kwg", "CSW21_ab"), "klv": find("klv", "CSW21_ab"),
            "letterdist": find("letterdist", "english_ab"), "layout": layout, "winpct": None}


def create_dev_jobs(client: Client, args, data: dict) -> None:
    """The jobs asked for with --dev-job, each reused if an active one of its
    name already runs, the new ones sharing what allocation is free."""
    wanted = list(dict.fromkeys(args.dev_job))
    small = (small_input_data(client, args, data["layout"])
             if any(job.endswith(SMALL_SUFFIX) for job in wanted) else None)

    def spec(job: str):
        """(data, players, settings) for one of DEV_JOBS."""
        on_small = job.endswith(SMALL_SUFFIX)
        job_data = small if on_small else data
        # Each data set's players named apart, since a config that already
        # exists by name is reused as it is.
        prefix = "ab-" if on_small else ""

        def player(name: str, sort: str, recorder: str = "best", rit: bool = args.rit,
                   sim: Optional[dict] = None) -> str:
            return player_config(client, prefix + name, sort, job_data, recorder=recorder,
                                 sim=sim, wordmap=args.wordmap, rit=rit)

        job_type = dev_job_type(job)
        if job_type == "leave_generation":
            # Static on equity with no rack info table: it measures the leaves
            # a table would cache. A generation closes once every rack has
            # been seen its target number of times -- minutes for the eight
            # english_ab racks, far longer for english's millions.
            name = "static-equity" if on_small else "static-equity-no-rit"
            return job_data, [player(name, "equity", rit=False)], {
                "num_iterations": 1000, "racks_per_task": 50,
                "target_rack_counts": [100, 200, 500, 1000, 1000, 1000]}
        if job_type == "opening_rack":
            # Every play ranked: english_ab's eight racks two to a task,
            # english's millions at the server's default batch.
            return job_data, [player("static-equity-all", "equity", recorder="all")], {
                "racks_per_batch": 2 if on_small else 500, "rack_size": 7}
        # english_ab's players share the leave job's config, which has no
        # rack info table: one on eight racks saves nothing.
        rit = args.rit and not on_small
        if DEV_JOB_KINDS[dev_job_kind(job)][1]:
            # A simmer must sort on equity, so the two differ in depth
            # instead. They consider at least the ten plays a captured
            # position keeps (player_config's num_plays_recorded), as job
            # creation requires. A task is two games or one pair, so a claim
            # finishes in minutes. english_ab brings no win% model, so its
            # simmers use the main data's.
            if not data["winpct"]:
                raise SeedError(f"{dev_job_name(job)} needs a win% model, and the imported "
                                "data has none")
            players = [player(f"sim-{plies}ply", "equity", recorder="all", rit=rit, sim={
                "winpct_id": data["winpct"], "num_plies": plies, "num_plays": 10,
                "max_iterations": 200, "time_limit_secs": 0}) for plies in (2, 1)]
            batch = ({"games_per_batch": 2} if job_type == "games" else {"pairs_per_batch": 1})
        else:
            players = [player("static-equity", "equity", rit=rit),
                       player("static-score", "score", rit=rit)]
            batch = {}
        if job_type == "games":
            # Not acted on before 50,000 games, as the pairs job's test waits
            # 50,000 pairs: equity against score decides within a hundred
            # games, and a job that finishes in a minute is no use to work on.
            return job_data, players, {"min_games": 50000, "capture_positions": True, **batch}
        # The players diverge within a few turns, so most pairs keep a
        # position.
        return job_data, players, {"capture_positions": True, "capture_first_divergence": True,
                                   **batch}

    running = {job: existing_active_job(client, dev_job_type(job), dev_job_name(job))
               for job in wanted}
    new = [job for job in wanted if not running[job] or args.new_job]
    taken = sum(job.get("allocation") or 0 for job in active_jobs(client))
    share = (100 - taken) // len(new) if new else 0
    if new and share < 1:
        raise SeedError(f"the active jobs already take {taken}% of the fleet, leaving nothing "
                        "for a new one; deactivate some, or start with --reset-db")
    for job in wanted:
        if job not in new:
            log(f"{dev_job_name(job)} is already active ({running[job]}); reusing it")
            continue
        job_data, players, settings = spec(job)
        create_job(client, args, job_data, players, dev_job_type(job), dev_job_name(job),
                   extra=settings, allocation=share)
    log(f"seeded — {len(wanted)} job(s) active" + (f", the new ones at {share}% each" if new else ""))


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
    parser.add_argument("--min-units", type=int, default=None,
                        help="games/pairs before SPRT is acted on (default: 100 games, "
                             "50000 pairs)")
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
    parser.add_argument("--job-name", default="",
                        help="what to call the seeded job (shown first in the jobs list); "
                             "unnamed by default, so it is titled by its type")
    parser.add_argument("--allocation", type=int, default=100)
    parser.add_argument("--no-wordmap", dest="wordmap", action="store_false",
                        help="players (and the leave-generation bot) play without a wordmap, "
                             "the rack info table built from one, or a word info table, so no "
                             "job waits on the derived-file builder; a player config that "
                             "already exists by name is reused as it is")
    parser.add_argument("--no-rit", dest="rit", action="store_false",
                        help="players play without a rack info table (about 1.9 GB of disk "
                             "and of memory per worker process)")
    parser.add_argument("--dev-job", action="append", default=[], choices=DEV_JOBS,
                        help="instead of --job-type, start with this one of dev.py's jobs; "
                             "repeat for several: leave_generation (six generations), "
                             "opening_rack, games (saving positions), game_pairs (saving each "
                             "pair's first divergence), or games_sim or game_pairs_sim (the "
                             "same, between two simmers), each on --lexicon and --letterdist, "
                             "or with _ab "
                             "appended on the two-letter english_ab data. New ones share the "
                             "allocation the active jobs leave free")
    parser.add_argument("--no-job", action="store_true",
                        help="create no job: just the admin and the input data")
    parser.add_argument("--small-tarball-date", default=None,
                        help="the version the two-letter test data is imported as, for the "
                             "english_ab jobs (dev.py serves it)")
    parser.add_argument("--small-git-ref", default=None,
                        help="the ref the two-letter test data is resolved at")
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

    def players_for(job_type: str) -> list:
        if job_type == "opening_rack":
            return [player_config(client, "static-equity-all", "equity", data, recorder="all",
                                  wordmap=args.wordmap, rit=args.rit)]
        return [
            player_config(client, "static-equity", "equity", data, wordmap=args.wordmap, rit=args.rit),
            player_config(client, "static-score", "score", data, wordmap=args.wordmap, rit=args.rit),
        ]

    if args.no_job:
        log("seeded — no job")
    elif args.dev_job:
        create_dev_jobs(client, args, data)
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
