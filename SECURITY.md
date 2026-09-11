# Security

Rationale treats repositories, Git metadata, synchronized forge metadata, and
MCP arguments as untrusted input.

The OCaml worker accepts only bounded, versioned frames from its Rust
supervisor. Its standard output is reserved for protocol messages; diagnostics
go to standard error. Timeouts, malformed frames, and worker termination are
reported as proof-unavailable failures and never converted into proof verdicts.

Do not report suspected vulnerabilities in a public issue. Until a private
security contact is published, keep reports private and do not include live
credentials, private repository contents, or exploit data in test fixtures.

The repository is not yet released and makes no production-security claim.
