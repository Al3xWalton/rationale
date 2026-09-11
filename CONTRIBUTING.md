# Contributing

Rationale preserves the reason for a change in the same history that carries the
change.

## Stories and branches

- One Story is delivered per branch and per pull request.
- Branches use `story/<number>-<short-name>`.
- Before a public GitHub remote exists, `Story: #<number>` refers to the matching
  file under `governance/stories/`.
- Keep the history linear and rebase the Story branch before review.

## Commit messages

Titles use this form and remain at most 72 characters:

```text
<type>(<scope>): <lowercase imperative summary>
```

Allowed types are `feat`, `fix`, `docs`, `test`, `ci`, `refactor`, `chore`, and
`style`.

Every non-bootstrap Story commit records at least two changes, one substantive
clarification, and its Story lineage:

```text
feat(protocol): define the worker handshake

Changes:
- Add the versioned request envelope
- Reject unsupported protocol revisions

Clarification:
This freezes compatibility before either implementation can diverge.

Story: #2 | Lineage: 2.1.0
```

The title says what the commit does. `Changes` describes observable work.
`Clarification` records why the boundary matters. Lineage advances within a Story
without implying that commit count measures quality.

## Verification

Run `make verify` before committing. A commit should leave both language
workspaces buildable and their relevant tests passing.
