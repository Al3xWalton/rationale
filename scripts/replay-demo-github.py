#!/usr/bin/env python3
"""Serve the generated public-demo GitHub fixtures on loopback."""

from __future__ import annotations

import argparse
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("repository", type=Path)
    parser.add_argument("--ready-file", required=True, type=Path)
    return parser.parse_args()


def load_json(path: Path) -> bytes:
    return json.dumps(json.loads(path.read_text(encoding="utf-8")), separators=(",", ":")).encode()


def handler(responses: dict[str, bytes]) -> type[BaseHTTPRequestHandler]:
    class FixtureHandler(BaseHTTPRequestHandler):
        server_version = "RationaleFixture/1"

        def do_GET(self) -> None:  # noqa: N802 - stdlib callback name
            path = urlsplit(self.path).path
            if path == "/repos/rationale-labs/demo/issues":
                if self.headers.get("If-None-Match") == '"demo-v1"':
                    self.send_response(304)
                    self._common_headers()
                    self.end_headers()
                    return
                self._json_response(responses["issues"], etag='"demo-v1"')
                return
            if path == "/repos/rationale-labs/demo/pulls/11/commits":
                self._json_response(responses["pull-11"])
                return
            if path == "/repos/rationale-labs/demo/pulls/22/commits":
                self._json_response(responses["pull-22"])
                return
            self.send_error(404)

        def _json_response(self, body: bytes, etag: str | None = None) -> None:
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            if etag is not None:
                self.send_header("ETag", etag)
            self._common_headers()
            self.end_headers()
            self.wfile.write(body)

        def _common_headers(self) -> None:
            self.send_header("X-RateLimit-Limit", "5000")
            self.send_header("X-RateLimit-Remaining", "4997")
            self.send_header("X-RateLimit-Reset", "1789150000")

        def log_message(self, _format: str, *args: object) -> None:
            del args

    return FixtureHandler


def main() -> None:
    args = parse_args()
    fixture_root = args.repository.resolve() / ".git" / "rationale-demo-fixtures"
    responses = {
        "issues": load_json(fixture_root / "issues.json"),
        "pull-11": load_json(fixture_root / "pull-11-commits.json"),
        "pull-22": load_json(fixture_root / "pull-22-commits.json"),
    }
    server = ThreadingHTTPServer(("127.0.0.1", 0), handler(responses))
    host, port = server.server_address
    args.ready_file.write_text(f"http://{host}:{port}/\n", encoding="utf-8")
    server.serve_forever()


if __name__ == "__main__":
    main()
