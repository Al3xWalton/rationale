# Security

Rationale treats repositories, Git metadata, synchronized forge metadata, and
MCP arguments as untrusted input.

The OCaml worker accepts only bounded, versioned frames from its Rust
supervisor. Its standard output is reserved for protocol messages; diagnostics
go to standard error. Timeouts, malformed frames, and worker termination are
reported as proof-unavailable failures and never converted into proof verdicts.

Local Git targets must be normalized repository-relative paths. Absolute paths,
parent traversal, Git-internal paths, and symlinks that escape the discovered
worktree are rejected before content or history is read.

Document inputs are size-bounded and only metadata under the `rationale`
namespace can create proof relationships. Quarantine diagnostics use stable
parser reasons and source locators; raw source fragments are not copied into
diagnostic messages.

Structured metadata nesting, GitHub response bytes, pagination, request count,
record expansion, graph slices, Git targets, record lookups, candidate queries,
and MCP argument schemas all have explicit bounds. HTTP content decompression is
not enabled. GitHub credentials are held only in sensitive request headers;
credentialed URLs, cross-origin pagination, response bodies in errors, and
credential-bearing cursors are rejected.

SQLite initialization validates the exact version-one table shape before
committing a migration. Corrupt or incompatible databases fail closed, and a
failed migration cannot partially replace their schema. The dependency policy
runs on dependency changes and weekly; parser fuzz harnesses are kept compiling
as part of `make verify`.

Do not report suspected vulnerabilities in a public issue. Until a private
security contact is published, keep reports private and do not include live
credentials, private repository contents, or exploit data in test fixtures.

The repository is not yet released and makes no production-security claim.
