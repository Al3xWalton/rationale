# Release process

Rationale ships the Rust CLI and OCaml proof worker as adjacent executables. The
archive also contains the deterministic offline demonstration, its GitHub API
replay fixture, a release manifest, and the security policy.

## Supported version-one packages

| Runner | Target | Archive |
| --- | --- | --- |
| Ubuntu 24.04 x86-64 | `x86_64-unknown-linux-gnu` | `rationale-v0.1.0-x86_64-unknown-linux-gnu.tar.gz` |
| macOS 15 Apple silicon | `aarch64-apple-darwin` | `rationale-v0.1.0-aarch64-apple-darwin.tar.gz` |

Windows, Linux ARM64, and Intel macOS are not version-one release targets. A
source build may work there, but it is not a packaged-support claim.

## Build and verify locally

From a clean checkout with the pinned Rust and OCaml toolchains installed:

```sh
make release-smoke
```

The command builds both languages in release mode, creates the host archive,
verifies its SHA-256 manifest after extraction, runs every offline demo verdict,
and connects to the packaged MCP server over stdio. It deliberately uses the
two extracted executables rather than build-tree paths.

`scripts/package-release.sh` refuses to overwrite an existing archive. Use a
new output directory for another candidate so evidence from separate builds is
not silently mixed.

## CI and publication

The `Release packages` workflow runs the same extracted-package smoke on clean
Ubuntu and macOS runners. A manual run produces 14-day workflow artifacts. A
signed or annotated `v0.1.0` tag additionally combines the checksums and creates
a GitHub release.

Public publication requires matching Apache-2.0 metadata in the Rust workspace
and OCaml package, plus the complete `LICENSE` and attribution `NOTICE`. The
workflow checks all four before it receives write permission to publish assets.

Before tagging:

1. run `make verify`, `make audit`, `make benchmark`, and `make release-smoke`;
2. manually review tracked fixtures, documentation, and Git history for private
   AVA material or credentials;
3. perform the opt-in live GitHub smoke against a disposable public repository
   with a read-only token, without preserving the token or response bodies;
4. push the reviewed commit and an annotated `v0.1.0` tag;
5. confirm both clean-runner package jobs pass before the publish job creates
   the release; and
6. download both release archives independently and verify them against the
   combined `SHA256SUMS` file.

The workflow checks that the tag version matches the compiled CLI. Every archive
contains `RELEASE-MANIFEST.json` with its target, source commit, protocol version,
source state, license identifier, and hashes for the two executables, demo entry
point, license, and attribution notice. The clean-runner workflow refuses to
package a dirty source tree; local development archives record that state rather
than claiming a clean release.
