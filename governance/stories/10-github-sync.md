---
rationale:
  id: STORY-10
  subject_id: github-evidence-source
  outcome_id: explicit-stale-safe-review-sync
  status: accepted
  documents:
    - ADR-0001
---

# Story 10: Synchronize explicit GitHub review relationships

## Problem

Local history can identify the change behind a line, but cannot prove the
review and work item that authorized it when those relationships live on the
repository forge.

## Outcome

Synchronize bounded, read-only GitHub evidence into the same atomic snapshot,
while preserving the last valid forge contribution as visibly stale whenever a
refresh fails.

## Acceptance criteria

- supported `github.com` remotes normalize to one unambiguous repository;
- issue, pull-request, commit-membership, and explicit closing relationships
  are retrieved through versioned REST endpoints and recorded fixtures;
- pagination, response size, record count, and request count are bounded;
- conditional requests persist only an opaque credential-free cursor;
- `GH_TOKEN` or `GITHUB_TOKEN` is used in memory and redacted from errors;
- `rationale sync` combines GitHub and local evidence, while `--local` remains
  offline-only;
- a failed refresh retains prior GitHub evidence, marks its source stale, and
  leaves current local evidence queryable;
- neither issue-number coincidence nor unrelated prose creates a proof edge.
