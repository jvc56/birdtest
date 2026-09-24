#!/usr/bin/env python3
"""Capture contract fixtures from a real `magpie contribute` exchange.

A recording HTTP proxy for the worker API. Point a contributor's `server` at
it instead of at the backend: every request is forwarded to the backend
unchanged and every answer handed back unchanged, and the first body of each
message type is written into the fixture directory (TESTING.md, "4.
Contract"). A hand-written fixture only proves that its author and the parser
agree; one captured here is what the two programs actually said.

    python3 scripts/capture_contract.py --upstream http://localhost:8080 \\
        --port 8090 --out contract-fixtures

then run `magpie contribute` with `server http://127.0.0.1:8090` against
whatever jobs are active, and stop the proxy with Ctrl-C once it reports every
fixture captured. `scripts/e2e_magpie.py --cases capture` does all of that
itself -- creates one job of each type, runs a real contributor through this
proxy, and fails if any fixture is missing -- and is how the committed
fixtures were made.

What is captured, and from what:

| Fixture                          | Body                                              |
|----------------------------------|---------------------------------------------------|
| anon-uuid-assignment.json        | the first assignment carrying `worker_uuid`       |
| assignment-game-pairs.json       | the first `game_pairs` assignment without one     |
| expected-data.json               | `expected_data` of the first assignment (without  |
|                                  | `worker_uuid`) that pins a derived file           |
| result-<type>.json               | the first result of each job type the server      |
|                                  | accepted (`games`, `game-pairs`, `opening-rack`,  |
|                                  | `leave-generation`)                               |
| heartbeat.json                   | the first heartbeat                               |

Nothing is normalised: the values are the ones that crossed the wire, so a
recapture changes the tokens, ids and timings in them. The files are
pretty-printed with two-space indentation, which changes no value.

Standard library only, so it runs wherever MAGPIE does.
"""

import argparse
import http.client
import json
import sys
import threading
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Callable, Dict, Optional

RESULT_NAMES = {
    "games": "result-games.json",
    "game_pairs": "result-game-pairs.json",
    "opening_rack": "result-opening-rack.json",
    "leave_generation": "result-leave-generation.json",
}

# Every fixture this proxy knows how to capture, in the order TESTING.md lists
# them.
FIXTURES = (
    "assignment-game-pairs.json",
    "result-games.json",
    "result-game-pairs.json",
    "result-opening-rack.json",
    "result-leave-generation.json",
    "heartbeat.json",
    "expected-data.json",
    "anon-uuid-assignment.json",
)

HOP_BY_HOP = {
    "connection", "keep-alive", "proxy-authenticate", "proxy-authorization",
    "te", "trailers", "transfer-encoding", "upgrade", "host", "content-length",
}

# Rewrites an answer on its way back to the contributor: (path, body) -> body.
# For tests that need a server behaving in a way the real one never does --
# see `e2e_magpie.py`'s M-6. Nothing rewritten is ever recorded.
Rewrite = Callable[[str, dict], dict]


def _json(body: bytes) -> Optional[dict]:
    try:
        value = json.loads(body) if body else None
    except ValueError:
        return None
    return value if isinstance(value, dict) else None


class Recorder:
    """Keeps the first body of each message type and writes it out."""

    def __init__(self, out_dir: Optional[Path], wanted=FIXTURES):
        self.out_dir = out_dir
        self.wanted = set(wanted)
        self.captured: Dict[str, dict] = {}
        # A result names only its claim token, so the job type it answers is
        # the one the assignment for that token carried.
        self.job_types: Dict[str, str] = {}
        self.lock = threading.Lock()
        # Every exchange, in order, for tests that assert on what was said:
        # (method, path, request body, status, response body).
        self.exchanges = []

    def missing(self):
        return [name for name in FIXTURES if name in self.wanted and name not in self.captured]

    def _keep(self, name: str, body: dict) -> None:
        if name not in self.wanted or name in self.captured:
            return
        self.captured[name] = body
        if self.out_dir is not None:
            path = self.out_dir / name
            path.write_text(json.dumps(body, indent=2) + "\n")
            print(f"[capture] wrote {path}", flush=True)

    def observe(self, method: str, path: str, request: Optional[dict], status: int,
                response: Optional[dict]) -> None:
        with self.lock:
            self.exchanges.append((method, path, request, status, response))
            route = urllib.parse.urlsplit(path).path
            if route == "/api/worker/task" and status == 200 and response \
                    and "task_request" in response:
                job_type = response["task_request"].get("job_type")
                self.job_types[response.get("claim_token")] = job_type
                if "worker_uuid" in response:
                    self._keep("anon-uuid-assignment.json", response)
                    return
                if job_type == "game_pairs":
                    self._keep("assignment-game-pairs.json", response)
                expected = response.get("expected_data") or {}
                if expected.get("derived"):
                    self._keep("expected-data.json", expected)
            elif route == "/api/worker/result" and status == 200 and request \
                    and (response or {}).get("accepted") is True:
                name = RESULT_NAMES.get(self.job_types.get(request.get("claim_token")))
                if name:
                    self._keep(name, request)
            elif route == "/api/worker/heartbeat" and status < 300 and request:
                self._keep("heartbeat.json", request)


class RecordingProxy(ThreadingHTTPServer):
    """Forwards to `upstream`; records into `recorder`; optionally rewrites."""

    daemon_threads = True

    def __init__(self, listen, upstream: str, recorder: Recorder,
                 rewrite: Optional[Rewrite] = None):
        parsed = urllib.parse.urlsplit(upstream)
        if parsed.scheme != "http":
            raise ValueError("the upstream must be an http:// URL")
        self.upstream_host = parsed.hostname
        self.upstream_port = parsed.port or 80
        self.upstream_base = parsed.path.rstrip("/")
        self.recorder = recorder
        self.rewrite = rewrite
        super().__init__(listen, _Handler)

    @property
    def url(self) -> str:
        host, port = self.server_address[:2]
        return f"http://{host}:{port}"

    def start(self) -> "RecordingProxy":
        threading.Thread(target=self.serve_forever, daemon=True).start()
        return self

    def stop(self) -> None:
        self.shutdown()
        self.server_close()


class _Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server: RecordingProxy

    def log_message(self, *_args) -> None:
        pass

    def _forward(self) -> None:
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else b""
        headers = {k: v for k, v in self.headers.items() if k.lower() not in HOP_BY_HOP}
        upstream = http.client.HTTPConnection(
            self.server.upstream_host, self.server.upstream_port, timeout=600)
        try:
            upstream.request(self.command, self.server.upstream_base + self.path,
                             body=body or None, headers=headers)
            answer = upstream.getresponse()
            payload = answer.read()
            status = answer.status
            answer_headers = [(k, v) for k, v in answer.getheaders()
                              if k.lower() not in HOP_BY_HOP]
        finally:
            upstream.close()

        is_json = any(k.lower() == "content-type" and "json" in v for k, v in answer_headers)
        response = _json(payload) if is_json else None
        self.server.recorder.observe(self.command, self.path, _json(body), status, response)
        if response is not None and self.server.rewrite is not None:
            rewritten = self.server.rewrite(self.path, response)
            if rewritten is not response:
                payload = json.dumps(rewritten).encode()

        self.send_response(status)
        for key, value in answer_headers:
            self.send_header(key, value)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(payload)

    do_GET = do_POST = do_PUT = do_DELETE = do_HEAD = _forward


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--upstream", default="http://localhost:8080",
                        help="the birdtest backend (default: %(default)s)")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8090)
    parser.add_argument("--out", type=Path, default=Path("contract-fixtures"),
                        help="directory to write fixtures into (default: %(default)s)")
    parser.add_argument("--only", default=",".join(FIXTURES),
                        help="comma-separated fixture names to capture (default: all)")
    args = parser.parse_args()

    wanted = [name.strip() for name in args.only.split(",") if name.strip()]
    unknown = sorted(set(wanted) - set(FIXTURES))
    if unknown:
        parser.error(f"unknown fixtures {unknown}; known: {', '.join(FIXTURES)}")
    args.out.mkdir(parents=True, exist_ok=True)
    recorder = Recorder(args.out, wanted)
    proxy = RecordingProxy((args.host, args.port), args.upstream, recorder)
    print(f"[capture] {proxy.url} -> {args.upstream}; waiting for {', '.join(wanted)}",
          flush=True)
    try:
        proxy.serve_forever()
    except KeyboardInterrupt:
        pass
    missing = recorder.missing()
    if missing:
        print(f"[capture] not captured: {', '.join(missing)}", file=sys.stderr)
        return 1
    print("[capture] every fixture captured")
    return 0


if __name__ == "__main__":
    sys.exit(main())
