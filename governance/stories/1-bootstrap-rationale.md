---
rationale:
  id: STORY-1
  subject_id: repository-bootstrap
  outcome_id: rust-host-ocaml-worker
  status: accepted
  documents:
    - ADR-0001
---

# Story 1: Bootstrap Rationale

## Problem

Rationale has an approved design but no independent, reproducible repository in
which to implement it.

## Outcome

Establish a buildable Rust workspace and OCaml proof-kernel project with an
explicit process boundary, local verification commands, and inspectable commit
governance.

## Acceptance criteria

- Rust is pinned and the workspace builds, lints, and tests.
- OCaml is pinned in a local opam switch and the kernel builds and tests.
- the language boundary is recorded as an accepted architecture decision;
- contribution guidance records the Story, lineage, and commit format;
- no repository evidence verdict is claimed during bootstrap.
