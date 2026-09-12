---
rationale:
  id: STORY-12
  subject_id: public-proof-demonstration
  outcome_id: reproducible-four-verdict-scenarios
  status: accepted
  documents:
    - ADR-0001
---

# Story 12: Demonstrate every proof verdict publicly

## Problem

Individual fixtures validate components, but they do not give a user or
investor one reproducible repository in which the full system is visible.

## Outcome

Generate a deterministic repository, replay synthetic GitHub responses
offline, and demonstrate established, partial, not-established, conflicted,
and explicitly resolved rationale through both CLI and MCP.

## Acceptance criteria

- the generator uses fixed authorship, timestamps, content, and commit order;
- independent generated repositories have identical commit hashes;
- no AVA source, history, identity, endpoint, or credential enters the demo;
- the complete line traverses commit, pull request, issue, and decision evidence;
- the incomplete line returns a plausible candidate outside its proof object;
- an unlinked commit returns `not_established`;
- two reachable current decisions with one subject and different outcomes
  return `conflicted`;
- a follow-up decision explicitly supersedes the old outcome and restores an
  established proof;
- CLI and MCP structured results match for all four verdicts;
- removing each decisive edge from the complete proof produces the expected
  deterministic downgrade.
