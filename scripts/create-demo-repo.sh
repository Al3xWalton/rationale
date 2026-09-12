#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: $0 TARGET_DIRECTORY" >&2
    exit 64
fi

SCRIPT_DIRECTORY=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
PROJECT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIRECTORY/.." && pwd -P)
TEMPLATE_ROOT="$PROJECT_ROOT/fixtures/demo/repository"
GITHUB_TEMPLATE_ROOT="$PROJECT_ROOT/fixtures/demo/github"
TARGET=$1

case "$TARGET" in
    ""|/|"${HOME:-__unset__}")
        echo "refusing unsafe demo target: $TARGET" >&2
        exit 64
        ;;
esac
if [ -e "$TARGET" ]; then
    echo "demo target already exists: $TARGET" >&2
    exit 73
fi

umask 022
mkdir -p "$TARGET/src" "$TARGET/governance" "$TARGET/decisions" "$TARGET/verification"
git -C "$TARGET" init --quiet --initial-branch=main
git -C "$TARGET" config user.name "Rationale Demo"
git -C "$TARGET" config user.email "demo@rationale.invalid"
git -C "$TARGET" config commit.gpgsign false
git -C "$TARGET" remote add origin https://github.com/rationale-labs/demo.git

commit_at() {
    commit_date=$1
    subject=$2
    body=${3:-}
    git -C "$TARGET" add .
    if [ -n "$body" ]; then
        GIT_AUTHOR_DATE="$commit_date" GIT_COMMITTER_DATE="$commit_date" \
            git -C "$TARGET" commit --quiet -m "$subject" -m "$body"
    else
        GIT_AUTHOR_DATE="$commit_date" GIT_COMMITTER_DATE="$commit_date" \
            git -C "$TARGET" commit --quiet -m "$subject"
    fi
}

install -m 0644 "$TEMPLATE_ROOT/complete.rs" "$TARGET/src/complete.rs"
install -m 0644 "$TEMPLATE_ROOT/work-item-101.md" "$TARGET/governance/ISSUE-101.md"
install -m 0644 "$TEMPLATE_ROOT/decision-0101.md" "$TARGET/decisions/ADR-0101.md"
install -m 0644 "$TEMPLATE_ROOT/verification-0101.rationale.toml" \
    "$TARGET/verification/checkout.rationale.toml"
commit_at "2026-01-01T12:00:00Z" "feat(checkout): enforce the documented timeout"
COMPLETE_COMMIT=$(git -C "$TARGET" rev-parse HEAD)
git -C "$TARGET" tag demo-complete

install -m 0644 "$TEMPLATE_ROOT/incomplete.rs" "$TARGET/src/incomplete.rs"
install -m 0644 "$TEMPLATE_ROOT/candidate-202.md" "$TARGET/governance/STORY-202.md"
commit_at "2026-01-01T12:01:00Z" "feat(checkout): add checkout retry guard"
INCOMPLETE_COMMIT=$(git -C "$TARGET" rev-parse HEAD)
git -C "$TARGET" tag demo-incomplete

install -m 0644 "$TEMPLATE_ROOT/absent.rs" "$TARGET/src/absent.rs"
commit_at "2026-01-01T12:02:00Z" "feat(cache): add an unexplained cache switch"
ABSENT_COMMIT=$(git -C "$TARGET" rev-parse HEAD)
git -C "$TARGET" tag demo-absent

install -m 0644 "$TEMPLATE_ROOT/conflict-v1.rs" "$TARGET/src/conflict.rs"
install -m 0644 "$TEMPLATE_ROOT/work-item-303.md" "$TARGET/governance/STORY-303.md"
install -m 0644 "$TEMPLATE_ROOT/decision-0303.md" "$TARGET/decisions/ADR-0303.md"
install -m 0644 "$TEMPLATE_ROOT/decision-0304.md" "$TARGET/decisions/ADR-0304.md"
commit_at "2026-01-01T12:03:00Z" \
    "feat(session): apply competing timeout policy" \
    "Story: #303 | Lineage: 303.1.0"
CONFLICT_COMMIT=$(git -C "$TARGET" rev-parse HEAD)
git -C "$TARGET" tag demo-conflict

install -m 0644 "$TEMPLATE_ROOT/conflict-v2.rs" "$TARGET/src/conflict.rs"
install -m 0644 "$TEMPLATE_ROOT/decision-0305.md" "$TARGET/decisions/ADR-0305.md"
commit_at "2026-01-01T12:04:00Z" \
    "feat(session): supersede the legacy timeout decision" \
    "Decision: ADR-0305

Story: #303 | Lineage: 303.2.0"
RESOLVED_COMMIT=$(git -C "$TARGET" rev-parse HEAD)
git -C "$TARGET" tag demo-resolved

FIXTURE_ROOT="$TARGET/.git/rationale-demo-fixtures"
mkdir -p "$FIXTURE_ROOT"
install -m 0644 "$GITHUB_TEMPLATE_ROOT/issues.json" "$FIXTURE_ROOT/issues.json"
sed "s/{{COMPLETE_COMMIT}}/$COMPLETE_COMMIT/g" \
    "$GITHUB_TEMPLATE_ROOT/pull-11-commits.json.in" > "$FIXTURE_ROOT/pull-11-commits.json"
sed "s/{{INCOMPLETE_COMMIT}}/$INCOMPLETE_COMMIT/g" \
    "$GITHUB_TEMPLATE_ROOT/pull-22-commits.json.in" > "$FIXTURE_ROOT/pull-22-commits.json"
{
    printf 'COMPLETE_COMMIT=%s\n' "$COMPLETE_COMMIT"
    printf 'INCOMPLETE_COMMIT=%s\n' "$INCOMPLETE_COMMIT"
    printf 'ABSENT_COMMIT=%s\n' "$ABSENT_COMMIT"
    printf 'CONFLICT_COMMIT=%s\n' "$CONFLICT_COMMIT"
    printf 'RESOLVED_COMMIT=%s\n' "$RESOLVED_COMMIT"
} > "$FIXTURE_ROOT/refs.env"

echo "Created deterministic Rationale demo at $TARGET"
echo "Conflict state: demo-conflict"
echo "Resolved state: demo-resolved"
