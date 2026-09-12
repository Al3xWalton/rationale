---
rationale:
  id: STORY-8
  subject_id: local-user-slice
  outcome_id: offline-cli-proof
  status: accepted
  documents:
    - ADR-0001
---

# Story 8: Explain local rationale evidence

## Problem

The components can normalize and prove evidence, but a user cannot yet build a
snapshot or ask why a real line exists from one coherent interface.

## Outcome

Connect local Git, document ingestion, SQLite publication, target resolution,
and the OCaml worker behind a Rust CLI with human and JSON output.

## Acceptance criteria

- `rationale sync --local` publishes one deterministic offline snapshot;
- `rationale why <target>` returns the kernel verdict and proof before any gaps;
- `rationale show <record-id>` exposes a current normalized record and origin;
- `rationale gaps --changed` reports changed working-copy paths without inference;
- all commands support `--json` and documented stable exit categories;
- human output orders verdict, proof paths, gaps, candidates, then freshness;
- the first end-to-end test uses a generated Git repository and the real OCaml worker;
- proof-unavailable failures remain distinct from missing rationale.
