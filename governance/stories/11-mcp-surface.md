---
rationale:
  id: STORY-11
  subject_id: agent-integration-boundary
  outcome_id: read-only-mcp-proof-tools
  status: accepted
  documents:
    - ADR-0001
---

# Story 11: Expose deterministic proof tools over MCP

## Problem

AVA and other user agents cannot yet consume the local proof engine through a
standard structured tool boundary.

## Outcome

Serve four read-only MCP tools over stdio while preserving the CLI's structured
results and the strict separation between proofs and candidate suggestions.

## Acceptance criteria

- `rationale serve` writes only MCP protocol traffic to stdout;
- `explain_rationale(target)` returns the same proof object as CLI JSON;
- `get_evidence(record_id)` preserves the normalized record and citation;
- `find_rationale_gaps(scope)` supports the conservative `changed` scope;
- `search_candidate_evidence(query)` returns auditable scores outside proof;
- generated schemas are strict and every tool is annotated read-only;
- the server performs no model sampling and exposes no mutation tool.
