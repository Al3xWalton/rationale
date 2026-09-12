#!/usr/bin/env python3
"""Verify the bounded file checksums declared by a Rationale release."""

from __future__ import annotations

import hashlib
import json
import pathlib
import sys


def fail(message: str) -> "NoReturn":
    raise SystemExit(f"release verification failed: {message}")


def digest(path: pathlib.Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(block)
    return hasher.hexdigest()


def main() -> None:
    if len(sys.argv) != 2:
        fail("usage: verify-release.py PACKAGE_DIRECTORY")

    package_root = pathlib.Path(sys.argv[1]).resolve()
    manifest_path = package_root / "RELEASE-MANIFEST.json"
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"invalid manifest: {error}")

    if manifest.get("schema_version") != 1 or manifest.get("package") != "rationale":
        fail("unsupported manifest identity")
    files = manifest.get("files")
    if not isinstance(files, list) or not files:
        fail("manifest has no files")

    declared: set[str] = set()
    for entry in files:
        if not isinstance(entry, dict):
            fail("file entry is not an object")
        relative = entry.get("path")
        expected = entry.get("sha256")
        if not isinstance(relative, str) or not isinstance(expected, str):
            fail("file entry is missing path or sha256")
        candidate = pathlib.PurePosixPath(relative)
        if candidate.is_absolute() or ".." in candidate.parts:
            fail(f"unsafe manifest path: {relative}")
        path = (package_root / pathlib.Path(*candidate.parts)).resolve()
        try:
            path.relative_to(package_root)
        except ValueError:
            fail(f"manifest path escapes package: {relative}")
        if not path.is_file():
            fail(f"declared file is missing: {relative}")
        if digest(path) != expected:
            fail(f"checksum mismatch: {relative}")
        declared.add(relative)

    required = {
        "rationale",
        "rationale-kernel-worker",
        "scripts/run-demo.sh",
        "LICENSE",
        "NOTICE",
    }
    if not required.issubset(declared):
        fail("manifest omits a required executable")
    print(f"verified {len(files)} release files for {manifest['target']}")


if __name__ == "__main__":
    main()
