---
rationale:
  id: STORY-9
  subject_id: candidate-evidence
  outcome_id: deterministic-non-proving-ranking
  status: accepted
  documents:
    - ADR-0001
---

# Story 9: Rank candidate evidence without proving it

## Problem

An incomplete proof identifies the missing relationship but does not help a user
find nearby explicit records that may be worth reviewing.

## Outcome

Rank candidate records with a versioned, bounded, fully reconstructible scoring
function after the OCaml kernel has returned its authoritative verdict.

## Acceptance criteria

- exact identifier, normalized token, path-segment, and source-recency inputs are
  returned as separate score components;
- scores use fixed weights and stable record identifiers break ties;
- unrelated, historical, and already established records are excluded;
- candidates appear only for partial or not-established proofs;
- adding, removing, or reordering candidates cannot change the kernel verdict;
- no embedding, model, fuzzy search, or candidate relationship enters a proof.
