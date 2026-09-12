---
rationale:
  id: STORY-14
  subject_id: local-proof-performance
  outcome_id: measured-ava-scale-budgets
  status: accepted
  documents:
    - ADR-0001
---

# Story 14: Establish local performance budgets

## Problem

The Rust and OCaml split is only useful if repository ingestion, proof slicing,
process communication, and agent-facing queries remain fast and stable at a
realistic project scale. Unmeasured speed claims are not suitable portfolio
evidence.

## Outcome

Generate public small, AVA-like, and stress-scale repositories; measure each
layer independently in release mode; and enforce version-one latency, slicing,
determinism, and memory budgets from a machine-readable report.

## Acceptance criteria

- generated fixtures disclose their scale without copying private AVA data;
- cold ingestion, incremental sync, graph slicing, protocol encoding, OCaml
  evaluation, worker round trip, warm engine/CLI/MCP queries, and SQLite
  publication are timed separately;
- the AVA-like fixture rounds up to 400 commits, 1,000 files, and 50 documents;
- warm engine, CLI, and MCP p95 are each below 100 ms on the documented machine;
- incremental offline sync p95 is below one second;
- proof requests contain only forward-reachable evidence rather than the full
  stored graph;
- repeated queries remain within an 8 MiB post-warmup resident-memory growth
  budget;
- canonical engine, CLI, MCP, and worker payloads are byte-identical across
  repeated samples;
- the report records date, hardware, toolchains, source tree, sample count,
  methodology, and every pass/fail outcome.
