# Rationale

Rationale proves whether a line of code is connected to recorded intent—and
shows the missing link when it is not.

## Status

Rationale now has its first cross-language proof slice. Rust can supervise the
long-lived OCaml kernel and evaluate bounded synthetic evidence into canonical
established, partial, not-established, or conflicted results. Repository
evidence can also be published as atomic immutable SQLite snapshots. Repository
line targets can be resolved into explicit local Git blame and rename-aware
history. Namespaced repository documents and verification manifests can supply
explicit decisions and relationships. The CLI and MCP surfaces are still under
development.

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
