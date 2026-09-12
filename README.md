# Rationale

Rationale proves whether a line of code is connected to recorded intent—and
shows the missing link when it is not.

## Status

Rationale now has a complete offline proof slice. The Rust CLI synchronizes
local Git history and namespaced repository documents into an atomic SQLite
snapshot, resolves real lines through blame and rename-aware history, and asks
the long-lived OCaml kernel for a canonical established, partial,
not-established, or conflicted result. Candidate ranking, GitHub synchronization,
and the MCP surface are still under development.

## Architecture

- **Rust** owns repository ingestion, indexing, the CLI, and MCP.
- **OCaml** owns the pure, deterministic proof kernel.
- A versioned process protocol keeps the language boundary explicit.
- Four-byte length-prefixed frames, deadlines, and bounded restart keep worker
  failure observable without inventing a fallback verdict.
- SQLite holds immutable evidence and exposes only complete current snapshots to
  proof readers, even while the next synchronization is being assembled.
- Local Git resolution distinguishes committed evidence from unattributed
  working-copy lines and marks shallow history as incomplete.
- Document ingestion recognizes configured literal identifiers and quarantines
  malformed metadata without turning nearby files into inferred relationships.
- Rationale never calls a language model. An external agent may explain its
  structured results without changing their verdicts.

The approved implementation proceeds as a thin end-to-end proof path before
adding GitHub synchronization, candidate ranking, or performance tuning.

## Local CLI

Build the worker and CLI, then synchronize a repository from anywhere inside
its working tree:

```sh
opam exec --switch=rationale-5.5.1 -- dune build --root ocaml @all
cargo build --release --bin rationale

./target/release/rationale sync --local
RATIONALE_KERNEL_WORKER="$PWD/ocaml/_build/default/worker/main.exe" \
  ./target/release/rationale why src/lib.rs:42
```

The local commands are:

```text
rationale sync --local
rationale why <path:line|path:start-end@revision|commit:revision>
rationale show <record-id>
rationale gaps --changed
```

Add `--json` to any command for structured output. Human `why` output always
orders the verdict, proof paths, gaps, non-proving candidates, and source
freshness. The CLI uses stable exit categories so scripts never need to parse
prose:

| Exit | Category | Meaning |
| ---: | --- | --- |
| 0 | success | The command succeeded or rationale was established. |
| 1 | operational | Local storage, filesystem, or resource-bound failure. |
| 2 | missing rationale | A record or admissible proof path is missing. |
| 3 | conflict | Current explicit evidence conflicts. |
| 4 | invalid input | The target, path, revision, or kernel request is invalid. |
| 5 | stale source | The result used incomplete or stale source history. |
| 6 | proof unavailable | The authoritative OCaml worker could not return a proof. |

Rationale stores local state under `.rationale/` by default. It ignores that
directory when reporting changed working-copy gaps.

## Development

Rust 1.98.0 is pinned through `rust-toolchain.toml`. OCaml uses a named opam
switch pinned to OCaml 5.5.1. The named switch keeps opam build prefixes free of
spaces even when the repository lives under a workspace path containing spaces.

```sh
opam switch create rationale-5.5.1 ocaml-base-compiler.5.5.1
opam install --switch=rationale-5.5.1 ./ocaml \
  --deps-only --with-test --with-dev-setup --yes
make verify
```

During development, `make verify` builds the OCaml companion worker before the
Rust cross-language tests. Installed builds discover `rationale-kernel-worker`
beside the Rust executable. Set `RATIONALE_KERNEL_WORKER` to an explicit path
when testing or packaging a different worker build.

See [CONTRIBUTING.md](CONTRIBUTING.md) for the Story and commit conventions.

## License

No public license has been selected yet. All rights are reserved until a license
file is added.
