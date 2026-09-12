---
rationale:
  id: STORY-6
  subject_id: local-git-resolution
  outcome_id: explicit-history-evidence
  status: accepted
  documents:
    - ADR-0001
---

# Story 6: Resolve code targets into explicit Git history

## Problem

A proof begins at a code target, but paths, revisions, working-copy changes, and
renames make attribution unsafe to infer from filenames alone.

## Outcome

Resolve repository-relative line and commit targets through a conservative Git
adapter. Return explicit blame and path-history citations, working-copy state,
rename provenance, and references that are actually present in commit messages.

## Acceptance criteria

- repository discovery and normalized repository-relative paths are tested;
- file lines, ranges, historical revisions, and commits parse unambiguously;
- blame identifies commits for attributable lines and rejects changed new lines;
- history covers initial commits, edits, renames, merges, and deletions;
- shallow history is marked incomplete rather than silently treated as complete;
- invalid revisions, outside paths, and symlink escapes fail closed;
- explicit issue, Story, ADR, and forge references are extracted without fuzzy inference;
- the concrete implementation is hidden behind a resolver trait for fixture adapters.
