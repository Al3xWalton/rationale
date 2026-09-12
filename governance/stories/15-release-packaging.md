---
rationale:
  id: STORY-15
  subject_id: portfolio-release
  outcome_id: reproducible-v0.1-package
  status: accepted
  documents:
    - ADR-0001
---

# Story 15: Prepare the portfolio and version 0.1 release

## Problem

A technically complete proof engine is not yet a legible portfolio artifact.
An evaluator needs one clear promise, an immediate demonstration, honest
boundaries, measured numbers, and a package that proves the Rust/OCaml split can
be installed as one tool.

## Outcome

Present Rationale around its deterministic proof guarantee, ship the Rust CLI
beside the OCaml worker, and verify the extracted archive through both the CLI
and MCP without depending on AVA or live GitHub.

## Acceptance criteria

- the README leads with one sentence and a release-to-demo path intended to fit
  inside one minute after download;
- a truthful visual shows both an established proof and a missing edge;
- architecture, proof-versus-candidate separation, no-model behavior,
  limitations, non-goals, MCP configuration, and current benchmark numbers are
  explicit;
- LocalityBench and SWE-Story are framed as independent work without borrowing
  results or implying completed claims;
- macOS Apple-silicon and Linux x86-64 archives contain adjacent Rust and OCaml
  executables, demo fixtures, a manifest, and SHA-256 verification;
- clean-runner jobs exercise the extracted CLI, offline demo, and MCP server;
- tag publication verifies matching Apache-2.0 metadata and attribution; and
- the repository contains no private AVA code, data, timings, or credentials.
