#!/bin/sh
set -eu

SCRIPT_DIRECTORY=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
PROJECT_ROOT=$(CDPATH='' cd -- "$SCRIPT_DIRECTORY/.." && pwd -P)

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
    echo "usage: $0 TARGET_TRIPLE [OUTPUT_DIRECTORY]" >&2
    exit 64
fi

TARGET_TRIPLE=$1
OUTPUT_DIRECTORY=${2:-"$PROJECT_ROOT/dist"}
RATIONALE_BINARY=${RATIONALE_BINARY:-"$PROJECT_ROOT/target/release/rationale"}
RATIONALE_WORKER_BINARY=${RATIONALE_WORKER_BINARY:-"$PROJECT_ROOT/ocaml/_build/default/worker/main.exe"}

case "$TARGET_TRIPLE" in
    ""|*[!A-Za-z0-9._-]*)
        echo "invalid target triple: $TARGET_TRIPLE" >&2
        exit 64
        ;;
esac

for executable in "$RATIONALE_BINARY" "$RATIONALE_WORKER_BINARY"; do
    if [ ! -x "$executable" ]; then
        echo "release executable not found: $executable" >&2
        exit 66
    fi
done

VERSION_OUTPUT=$("$RATIONALE_BINARY" --version)
VERSION=${VERSION_OUTPUT#rationale }
case "$VERSION" in
    ""|*[!0-9A-Za-z.+-]*)
        echo "could not determine a safe release version from: $VERSION_OUTPUT" >&2
        exit 65
        ;;
esac

PACKAGE_NAME="rationale-v${VERSION}-${TARGET_TRIPLE}"
ARCHIVE="$OUTPUT_DIRECTORY/$PACKAGE_NAME.tar.gz"
CHECKSUM="$ARCHIVE.sha256"

mkdir -p "$OUTPUT_DIRECTORY"
if [ -e "$ARCHIVE" ] || [ -e "$CHECKSUM" ]; then
    echo "release output already exists: $ARCHIVE" >&2
    exit 73
fi

TEMPORARY_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/rationale-release.XXXXXX")
trap 'rm -rf "$TEMPORARY_ROOT"' EXIT HUP INT TERM
PACKAGE_ROOT="$TEMPORARY_ROOT/$PACKAGE_NAME"
mkdir -p "$PACKAGE_ROOT/scripts" "$PACKAGE_ROOT/fixtures"

install -m 0755 "$RATIONALE_BINARY" "$PACKAGE_ROOT/rationale"
install -m 0755 "$RATIONALE_WORKER_BINARY" "$PACKAGE_ROOT/rationale-kernel-worker"
install -m 0755 "$SCRIPT_DIRECTORY/run-demo.sh" "$PACKAGE_ROOT/scripts/run-demo.sh"
install -m 0755 "$SCRIPT_DIRECTORY/create-demo-repo.sh" "$PACKAGE_ROOT/scripts/create-demo-repo.sh"
install -m 0755 "$SCRIPT_DIRECTORY/replay-demo-github.py" "$PACKAGE_ROOT/scripts/replay-demo-github.py"
install -m 0755 "$SCRIPT_DIRECTORY/smoke-mcp.py" "$PACKAGE_ROOT/scripts/smoke-mcp.py"
install -m 0755 "$SCRIPT_DIRECTORY/verify-release.py" "$PACKAGE_ROOT/scripts/verify-release.py"
install -m 0644 "$PROJECT_ROOT/README.md" "$PACKAGE_ROOT/README.md"
install -m 0644 "$PROJECT_ROOT/SECURITY.md" "$PACKAGE_ROOT/SECURITY.md"
cp -R "$PROJECT_ROOT/fixtures/demo" "$PACKAGE_ROOT/fixtures/demo"
install -m 0644 "$PROJECT_ROOT/LICENSE" "$PACKAGE_ROOT/LICENSE"
install -m 0644 "$PROJECT_ROOT/NOTICE" "$PACKAGE_ROOT/NOTICE"

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

SOURCE_COMMIT=$(git -C "$PROJECT_ROOT" rev-parse HEAD 2>/dev/null || printf 'unknown')
if [ -n "$(git -C "$PROJECT_ROOT" status --porcelain --untracked-files=normal 2>/dev/null)" ]; then
    SOURCE_STATE=working_tree_changes
else
    SOURCE_STATE=clean
fi
LICENSE_ID=$(sed -n 's/^license = "\([^"]*\)"/\1/p' "$PROJECT_ROOT/Cargo.toml" | head -n 1)
case "$LICENSE_ID" in
    ""|*[!0-9A-Za-z.+-]*)
        echo "workspace license is missing or unsafe for the release manifest" >&2
        exit 65
        ;;
esac
RATIONALE_SHA256=$(sha256_file "$PACKAGE_ROOT/rationale")
WORKER_SHA256=$(sha256_file "$PACKAGE_ROOT/rationale-kernel-worker")
DEMO_SHA256=$(sha256_file "$PACKAGE_ROOT/scripts/run-demo.sh")
LICENSE_SHA256=$(sha256_file "$PACKAGE_ROOT/LICENSE")
NOTICE_SHA256=$(sha256_file "$PACKAGE_ROOT/NOTICE")

{
    printf '{\n'
    printf '  "schema_version": 1,\n'
    printf '  "package": "rationale",\n'
    printf '  "version": "%s",\n' "$VERSION"
    printf '  "target": "%s",\n' "$TARGET_TRIPLE"
    printf '  "source_commit": "%s",\n' "$SOURCE_COMMIT"
    printf '  "source_state": "%s",\n' "$SOURCE_STATE"
    printf '  "protocol_version": 1,\n'
    printf '  "license": "%s",\n' "$LICENSE_ID"
    printf '  "files": [\n'
    printf '    {"path": "rationale", "sha256": "%s"},\n' "$RATIONALE_SHA256"
    printf '    {"path": "rationale-kernel-worker", "sha256": "%s"},\n' "$WORKER_SHA256"
    printf '    {"path": "scripts/run-demo.sh", "sha256": "%s"},\n' "$DEMO_SHA256"
    printf '    {"path": "LICENSE", "sha256": "%s"},\n' "$LICENSE_SHA256"
    printf '    {"path": "NOTICE", "sha256": "%s"}\n' "$NOTICE_SHA256"
    printf '  ]\n'
    printf '}\n'
} > "$PACKAGE_ROOT/RELEASE-MANIFEST.json"

tar -C "$TEMPORARY_ROOT" -czf "$ARCHIVE" "$PACKAGE_NAME"
ARCHIVE_SHA256=$(sha256_file "$ARCHIVE")
printf '%s  %s\n' "$ARCHIVE_SHA256" "$(basename "$ARCHIVE")" > "$CHECKSUM"

printf '%s\n%s\n' "$ARCHIVE" "$CHECKSUM"
