---
rationale:
  id: STORY-5
  subject_id: evidence-persistence
  outcome_id: immutable-sqlite-snapshots
  status: accepted
  documents:
    - ADR-0001
---

# Story 5: Publish immutable evidence snapshots

## Problem

Proof queries need a stable evidence view while local and remote sources are
being synchronized. A partial or failed synchronization must never replace the
last complete view.

## Outcome

Store normalized records, edges, conflicts, source freshness, cursors, and
quarantine diagnostics in inspectable SQLite tables. Build each candidate in a
transaction and publish its current-snapshot pointer atomically.

## Acceptance criteria

- records and edges are immutable and inserted idempotently;
- content mismatch under an existing ID fails closed;
- incomplete candidates remain invisible to readers;
- failure before or during publication preserves the previous snapshot;
- graph slices are read from one stable transaction and enforce bounds;
- source cursors advance only with a published snapshot;
- quarantine diagnostics store locations and reasons, not source contents;
- a generated database opens successfully with the SQLite command-line tool.
