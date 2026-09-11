---
rationale:
  id: STORY-4
  subject_id: language-boundary
  outcome_id: supervised-worker-protocol
  status: accepted
  documents:
    - ADR-0001
---

# Story 4: Connect Rust to the OCaml proof kernel

## Problem

The proof kernel is deterministic but still runs only inside OCaml tests. The
Rust host needs a durable boundary that preserves proof authority when the
worker is slow, cancelled, malformed, or unavailable.

## Outcome

Run the OCaml evaluator as a long-lived framed worker behind an asynchronous
Rust supervisor with version negotiation, bounded frames, deadlines, safe
cancellation, crash detection, and bounded restart.

## Acceptance criteria

- the worker reserves stdout exclusively for framed protocol traffic;
- Rust validates the worker protocol version before accepting queries;
- all golden requests cross the real Rust-to-OCaml process boundary;
- request and response frames enforce an explicit maximum size;
- cancelling a caller cannot desynchronise the shared worker stream;
- worker hangs, crashes, and malformed responses are proof-unavailable errors;
- no worker failure is converted into a proof verdict;
- restart attempts and backoff are bounded.
