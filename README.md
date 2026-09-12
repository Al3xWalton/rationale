# rationale

A proof tool for Git and GitHub. `rationale` answers why a line or commit exists
by following explicit links through commit history, pull requests, work items,
decisions, and verification records. When the chain is incomplete or
conflicting, it returns the exact missing link instead of asking a model to
guess. The result can then be handed to an agent through MCP for explanation or
repair.

## Requirements

- [`git`](https://git-scm.com/downloads)
- [`python3`](https://www.python.org/downloads/) for the offline demo
- Apple-silicon macOS or x86-64 Linux for the prebuilt release

No model or API key is required.

## To install

Download the latest archive. Use `aarch64-apple-darwin` on an Apple-silicon Mac
or `x86_64-unknown-linux-gnu` on Linux.

```sh
VERSION=v0.1.0
TARGET=aarch64-apple-darwin
ARCHIVE="rationale-${VERSION}-${TARGET}.tar.gz"

curl -fLO "https://github.com/Al3xWalton/rationale/releases/download/${VERSION}/${ARCHIVE}"
curl -fLO "https://github.com/Al3xWalton/rationale/releases/download/${VERSION}/${ARCHIVE}.sha256"

if command -v sha256sum >/dev/null 2>&1; then
  sha256sum -c "${ARCHIVE}.sha256"
else
  shasum -a 256 -c "${ARCHIVE}.sha256"
fi

tar -xzf "$ARCHIVE"
cd "${ARCHIVE%.tar.gz}"
sudo install -m 0755 rationale rationale-kernel-worker /usr/local/bin/
```

Run the included offline demonstration before installing, if you prefer.

```sh
./scripts/run-demo.sh
```

## To use

Synchronize evidence from the current repository.

```sh
rationale sync
```

Ask why a line exists.

```sh
rationale why src/lib.rs:42
```

Rationale returns one of four deterministic verdicts: `established`, `partial`,
`not-established`, or `conflicted`. An established result includes the complete
evidence path. Any other result includes the missing or conflicting
relationship and may include separately ranked, non-proving candidates.

Other commands:

- `rationale sync --local`: synchronize without GitHub access.
- `rationale why <target>`: prove a line, range, historical revision, or commit.
- `rationale show <record-id>`: inspect one normalized evidence record.
- `rationale gaps --changed`: find gaps affecting changed paths.
- `rationale serve`: expose the read-only MCP server over stdio.
- `--json`: return the same result as stable structured data.

Public GitHub repositories need no credential. For private repositories, set a
read-only `GH_TOKEN` or `GITHUB_TOKEN` before running `rationale sync`.

## Agent integration

The MCP server exposes four read-only tools: `explain_rationale`,
`get_evidence`, `find_rationale_gaps`, and `search_candidate_evidence`.

```json
{
  "mcpServers": {
    "rationale": {
      "command": "/absolute/path/to/rationale",
      "args": [
        "--worker",
        "/absolute/path/to/rationale-kernel-worker",
        "serve"
      ],
      "cwd": "/absolute/path/to/your/repository"
    }
  }
}
```

The server never samples a model and cannot mutate evidence. An external agent
can explain or act on the result, but cannot change the proof verdict.

## How it works

- Rust resolves Git history, ingests explicit records, synchronizes GitHub,
  publishes atomic SQLite snapshots, and serves the CLI and MCP interface.
- OCaml receives a bounded typed graph and owns the pure proof decision.
- A versioned, length-prefixed process protocol keeps the language boundary
  explicit and makes worker failure observable without inventing a fallback.

On an Apple M4 Max, the recorded AVA-like benchmark measured a 5.890 ms warm
engine p95 and a 6.061 ms warm MCP p95 while sending 4 of 480 stored nodes to the
proof kernel. See the [methodology](benchmarks/README.md) and
[machine-readable results](benchmarks/results/latest.json).

## Development

Rust 1.98.0 and OCaml 5.5.1 are pinned. To build from source:

```sh
git clone https://github.com/Al3xWalton/rationale && cd rationale
opam switch create rationale-5.5.1 ocaml-base-compiler.5.5.1
opam install --switch=rationale-5.5.1 ./ocaml --deps-only --with-test --with-dev-setup --yes
make verify
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the Story and commit conventions,
[SECURITY.md](SECURITY.md) for the security policy, and
[docs/RELEASING.md](docs/RELEASING.md) for release verification.

## License

Licensed under the [Apache License 2.0](LICENSE). Redistributions retain the
project attribution recorded in [NOTICE](NOTICE).
