#!/bin/sh
set -eu

SCRIPT_DIRECTORY=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
PROJECT_ROOT=$(CDPATH='' cd -- "$SCRIPT_DIRECTORY/.." && pwd -P)

if [ "$#" -gt 1 ]; then
    echo "usage: $0 [TARGET_DIRECTORY]" >&2
    exit 64
fi
if [ "$#" -eq 1 ]; then
    DEMO_REPOSITORY=$1
else
    DEMO_PARENT=$(mktemp -d "${TMPDIR:-/tmp}/rationale-public-demo.XXXXXX")
    DEMO_REPOSITORY="$DEMO_PARENT/repository"
fi

"$SCRIPT_DIRECTORY/create-demo-repo.sh" "$DEMO_REPOSITORY"
READY_FILE="$DEMO_REPOSITORY/.git/rationale-demo-fixtures/server-url"
python3 "$SCRIPT_DIRECTORY/replay-demo-github.py" \
    "$DEMO_REPOSITORY" --ready-file "$READY_FILE" &
SERVER_PID=$!
trap 'kill "$SERVER_PID" 2>/dev/null || true' EXIT HUP INT TERM

attempt=0
while [ ! -s "$READY_FILE" ]; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 100 ]; then
        echo "fixture server did not start" >&2
        exit 70
    fi
    sleep 0.05
done
IFS= read -r API_BASE < "$READY_FILE"

if [ -n "${RATIONALE_BIN:-}" ]; then
    RATIONALE_BINARY=$RATIONALE_BIN
    RATIONALE_WORKER_BINARY=${RATIONALE_KERNEL_WORKER:-"$(dirname -- "$RATIONALE_BINARY")/rationale-kernel-worker"}
elif [ -x "$PROJECT_ROOT/rationale" ] && [ -x "$PROJECT_ROOT/rationale-kernel-worker" ]; then
    RATIONALE_BINARY="$PROJECT_ROOT/rationale"
    RATIONALE_WORKER_BINARY="$PROJECT_ROOT/rationale-kernel-worker"
else
    opam exec --switch=rationale-5.5.1 -- dune build --root "$PROJECT_ROOT/ocaml" @all
    cargo build --quiet --manifest-path "$PROJECT_ROOT/Cargo.toml" --bin rationale
    RATIONALE_BINARY="$PROJECT_ROOT/target/debug/rationale"
    RATIONALE_WORKER_BINARY="$PROJECT_ROOT/ocaml/_build/default/worker/main.exe"
fi
if [ ! -x "$RATIONALE_BINARY" ] || [ ! -x "$RATIONALE_WORKER_BINARY" ]; then
    echo "rationale release executables are unavailable" >&2
    exit 66
fi
DATABASE=.rationale/demo.db

git -C "$DEMO_REPOSITORY" checkout --quiet demo-conflict
(
    cd "$DEMO_REPOSITORY"
    "$RATIONALE_BINARY" --database "$DATABASE" --github-api-base "$API_BASE" sync
    printf '\nComplete — code to recorded intent\n'
    "$RATIONALE_BINARY" --database "$DATABASE" --worker "$RATIONALE_WORKER_BINARY" why src/complete.rs:2
    printf '\nIncomplete — candidate is separate from proof\n'
    "$RATIONALE_BINARY" --database "$DATABASE" --worker "$RATIONALE_WORKER_BINARY" why src/incomplete.rs:2 || test "$?" -eq 2
    printf '\nAbsent — no relationship leaves the commit\n'
    "$RATIONALE_BINARY" --database "$DATABASE" --worker "$RATIONALE_WORKER_BINARY" why commit:demo-absent || test "$?" -eq 2
    printf '\nConflict — two current outcomes for one subject\n'
    "$RATIONALE_BINARY" --database "$DATABASE" --worker "$RATIONALE_WORKER_BINARY" why src/conflict.rs:2 || test "$?" -eq 3
)

git -C "$DEMO_REPOSITORY" checkout --quiet demo-resolved
(
    cd "$DEMO_REPOSITORY"
    "$RATIONALE_BINARY" --database "$DATABASE" --github-api-base "$API_BASE" sync
    printf '\nResolved — explicit supersession restores one current outcome\n'
    "$RATIONALE_BINARY" --database "$DATABASE" --worker "$RATIONALE_WORKER_BINARY" why src/conflict.rs:2
)

printf '\nDemo repository retained at %s\n' "$DEMO_REPOSITORY"
