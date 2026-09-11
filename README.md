# Rationale

Rationale proves whether a line of code is connected to recorded intent—and
shows the missing link when it is not.

## Status

Rationale is in bootstrap development. The current repository establishes the
Rust systems host, the OCaml proof-kernel boundary, and the governance needed to
make its own development history inspectable. It does not yet evaluate
repository evidence.

## Architecture

- **Rust** owns repository ingestion, indexing, the CLI, and MCP.
- **OCaml** owns the pure, deterministic proof kernel.
- A versioned process protocol keeps the language boundary explicit.
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

See [CONTRIBUTING.md](CONTRIBUTING.md) for the Story and commit conventions.

## License

No public license has been selected yet. All rights are reserved until a license
file is added.
