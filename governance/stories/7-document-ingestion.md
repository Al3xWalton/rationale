---
rationale:
  id: STORY-7
  subject_id: document-ingestion
  outcome_id: explicit-rationale-metadata
  status: accepted
  documents:
    - ADR-0001
---

# Story 7: Ingest explicit rationale documents

## Problem

Git can establish what changed, but decisions, work items, supersession, and
verification live in human-readable repository artifacts. Proximity or loose
language must not be promoted into proof relationships.

## Outcome

Parse bounded Markdown metadata and verification manifests into normalized
records, explicit relationships, conflicts, and sanitized quarantine
diagnostics. Recognize only configured literal identifiers and links.

## Acceptance criteria

- YAML and TOML front matter is scoped under a `rationale` namespace;
- Markdown links and configured identifiers create only `documents` edges;
- `supersedes`, `verified_by`, and conflicts require explicit metadata;
- verification manifests name both an artifact and at least one target;
- a neighbouring test file never creates a verification relationship;
- malformed or oversized files are quarantined without aborting valid inputs;
- diagnostics include a relative locator and reason but never source contents;
- input order cannot change normalized record, edge, conflict, or diagnostic order.
