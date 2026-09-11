---
rationale:
  id: STORY-2
  subject_id: proof-protocol
  outcome_id: strict-json-v1
  status: accepted
  documents:
    - ADR-0001
---

# Story 2: Freeze proof protocol version 1

## Problem

Rust and OCaml cannot evolve independently until their evidence and proof types
have one strict, versioned wire contract.

## Outcome

Define protocol version 1 as checked JSON Schemas, equivalent Rust and OCaml
types, and shared canonical fixtures that both implementations round-trip.

## Acceptance criteria

- unknown fields and unknown enum values fail closed;
- proof responses and typed errors are disjoint;
- all evidence, verdict, gap, and conflict types are represented;
- candidates remain outside the proof-kernel protocol;
- Rust and OCaml emit identical canonical bytes for every fixture;
- protocol schemas and both language test suites pass.
