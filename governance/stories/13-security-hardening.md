---
rationale:
  id: STORY-13
  subject_id: untrusted-evidence-boundaries
  outcome_id: bounded-fail-closed-ingestion
  status: accepted
  documents:
    - ADR-0001
---

# Story 13: Harden untrusted evidence boundaries

## Problem

Repositories, forge responses, worker messages, evidence databases, and MCP
arguments cross trust boundaries. A malformed or expanding input must not
invent a verdict, expose a credential, exhaust an unbounded resource, or replace
the last valid evidence.

## Outcome

Apply explicit limits at every public ingestion boundary, validate stored schema
before publication, and continuously compile fuzz targets and check dependency
policy.

## Acceptance criteria

- repository traversal, Git-internal paths, and escaping symlinks fail closed;
- malformed UTF-8, malformed metadata, oversized documents, and excessive YAML
  or TOML nesting are quarantined with content-free diagnostics;
- GitHub response bytes, JSON depth, page count, request count, record expansion,
  pagination origin, and credential handling are bounded and tested;
- snapshot slices and Rust-to-OCaml frames reject declared sizes before bounded
  allocation or proof evaluation;
- worker hangs, crashes, malformed messages, and cancellation cannot become a
  verdict or desynchronize the next request;
- corrupt or incompatible SQLite databases remain intact and failed schema work
  rolls back atomically;
- MCP input schemas expose size limits and runtime validation rejects invalid or
  oversized targets, record identifiers, and candidate queries;
- dependency advisories, licenses, sources, and wildcard requirements are checked
  by a least-privilege workflow with immutable action references;
- protocol framing, Git target syntax, and document metadata have deterministic,
  result-tolerant fuzz harnesses that compile in the normal verification gate.
