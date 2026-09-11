---
rationale:
  id: ADR-0001
  subject_id: runtime-language-boundary
  outcome_id: rust-host-ocaml-worker-v1
  status: accepted
  documents:
    - STORY-1
---

# ADR-0001: Separate the systems host from the proof kernel

## Context

Rationale needs fast repository ingestion and a small decision core whose proof
rules are easy to inspect and test. Using two languages does not itself make the
tool faster, and embedding the OCaml runtime into Rust would add avoidable memory
management and failure-boundary complexity.

## Decision

Use Rust for repository I/O, indexing, synchronization, CLI, and MCP. Use OCaml
for a pure proof kernel. Run the kernel as a long-lived native worker behind a
small versioned process protocol.

## Consequences

- Rust can parallelize and cache systems work without entering the proof rules.
- OCaml receives bounded typed evidence slices and performs no I/O or ranking.
- Worker failure is observable and cannot silently become a heuristic verdict.
- Releases must package two matching binaries and test their protocol agreement.
