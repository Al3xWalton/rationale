---
rationale:
  id: STORY-3
  subject_id: proof-evaluation
  outcome_id: pure-ocaml-kernel
  status: accepted
  documents:
    - ADR-0001
---

# Story 3: Evaluate rationale proofs in OCaml

## Problem

The protocol can describe evidence, but no trusted component yet decides whether
that evidence establishes recorded intent.

## Outcome

Implement a pure OCaml evaluator that validates bounded evidence slices, follows
only current admissible edges, emits canonical proof objects, exposes typed gaps,
and resolves only explicitly represented supersession and conflict.

## Acceptance criteria

- established, partial, not-established, and conflicted verdicts are tested;
- historical evidence cannot establish a current rationale;
- graph cycles terminate without entering a proof twice;
- input ordering does not change canonical output bytes;
- removing a decisive edge downgrades an established result;
- candidates, repository I/O, and model calls remain outside the kernel;
- malformed references and resource excess return typed errors.
