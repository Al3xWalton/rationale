#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: $0 RELEASE_ARCHIVE" >&2
    exit 64
fi

ARCHIVE=$1
CHECKSUM="$ARCHIVE.sha256"
if [ ! -f "$ARCHIVE" ] || [ ! -f "$CHECKSUM" ]; then
    echo "release archive or checksum is missing" >&2
    exit 66
fi

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

IFS=' ' read -r EXPECTED_SHA256 _ < "$CHECKSUM"
ACTUAL_SHA256=$(sha256_file "$ARCHIVE")
if [ "$EXPECTED_SHA256" != "$ACTUAL_SHA256" ]; then
    echo "release archive checksum does not match" >&2
    exit 65
fi

ARCHIVE_NAME=$(basename "$ARCHIVE")
PACKAGE_NAME=${ARCHIVE_NAME%.tar.gz}
if [ "$PACKAGE_NAME" = "$ARCHIVE_NAME" ]; then
    echo "release archive must end in .tar.gz" >&2
    exit 64
fi

TEMPORARY_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/rationale-smoke.XXXXXX")
trap 'rm -rf "$TEMPORARY_ROOT"' EXIT HUP INT TERM
tar -C "$TEMPORARY_ROOT" -xzf "$ARCHIVE"
PACKAGE_ROOT="$TEMPORARY_ROOT/$PACKAGE_NAME"
if [ ! -d "$PACKAGE_ROOT" ]; then
    echo "release archive has an unexpected root" >&2
    exit 65
fi

python3 "$PACKAGE_ROOT/scripts/verify-release.py" "$PACKAGE_ROOT"
"$PACKAGE_ROOT/rationale" --version

DEMO_REPOSITORY="$TEMPORARY_ROOT/demo-repository"
DEMO_LOG="$TEMPORARY_ROOT/demo.log"
if ! "$PACKAGE_ROOT/scripts/run-demo.sh" "$DEMO_REPOSITORY" > "$DEMO_LOG" 2>&1; then
    sed -n '1,240p' "$DEMO_LOG" >&2
    exit 70
fi
if ! grep -q '^Verdict: established$' "$DEMO_LOG" || \
   ! grep -q '^Verdict: partial$' "$DEMO_LOG" || \
   ! grep -q '^Verdict: conflicted$' "$DEMO_LOG"; then
    echo "packaged demo did not exercise the expected verdicts" >&2
    sed -n '1,240p' "$DEMO_LOG" >&2
    exit 70
fi

python3 "$PACKAGE_ROOT/scripts/smoke-mcp.py" \
    "$PACKAGE_ROOT/rationale" \
    "$DEMO_REPOSITORY" \
    .rationale/demo.db \
    src/complete.rs:2

echo "release smoke passed: $PACKAGE_NAME"
