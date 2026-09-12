#!/usr/bin/env python3
"""Connect to a packaged Rationale MCP server over stdio."""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys
from typing import Any


def main() -> None:
    if len(sys.argv) != 5:
        raise SystemExit(
            "usage: smoke-mcp.py RATIONALE REPOSITORY DATABASE TARGET"
        )

    executable, repository, database, target = sys.argv[1:]
    process = subprocess.Popen(
        [executable, "--database", database, "serve"],
        cwd=repository,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    )
    assert process.stdin is not None
    assert process.stdout is not None

    def send(message: dict[str, Any]) -> None:
        process.stdin.write(json.dumps(message, separators=(",", ":")) + "\n")
        process.stdin.flush()

    def receive(identifier: int) -> dict[str, Any]:
        while True:
            line = process.stdout.readline()
            if not line:
                raise RuntimeError("MCP server closed before responding")
            response = json.loads(line)
            if response.get("id") == identifier:
                if "error" in response:
                    raise RuntimeError(f"MCP request failed: {response['error']}")
                return response["result"]

    try:
        send(
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "rationale-release-smoke", "version": "0.1"},
                },
            }
        )
        initialized = receive(1)
        if initialized.get("serverInfo", {}).get("name") != "rationale":
            raise RuntimeError("MCP server identity did not match Rationale")

        send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        send({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
        tools = receive(2).get("tools", [])
        names = {tool.get("name") for tool in tools}
        expected = {
            "explain_rationale",
            "find_rationale_gaps",
            "get_evidence",
            "search_candidate_evidence",
        }
        if names != expected:
            raise RuntimeError(f"unexpected MCP tools: {sorted(names)}")

        send(
            {
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "explain_rationale",
                    "arguments": {"target": target},
                },
            }
        )
        result = receive(3)
        structured = result.get("structuredContent", {})
        verdict = (
            structured.get("response", {})
            .get("proof", {})
            .get("verdict")
        )
        if verdict != "established":
            raise RuntimeError(f"unexpected packaged MCP verdict: {verdict!r}")
        print("packaged MCP connection returned an established proof")
    finally:
        process.stdin.close()
        try:
            return_code = process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.terminate()
            return_code = process.wait(timeout=5)
        if return_code != 0 and sys.exc_info()[0] is None:
            assert process.stderr is not None
            detail = process.stderr.read().strip()
            raise RuntimeError(f"MCP server exited with {return_code}: {detail}")


if __name__ == "__main__":
    main()
