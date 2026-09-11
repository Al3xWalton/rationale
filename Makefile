OPAM_SWITCH := rationale-5.5.1

.PHONY: build fmt lint test verify

build:
	cargo build --workspace
	opam exec --switch=$(OPAM_SWITCH) -- dune build --root ocaml @all

fmt:
	cargo fmt --all
	opam exec --switch=$(OPAM_SWITCH) -- dune fmt --root ocaml

lint:
	cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
	cargo test --workspace --all-features
	RATIONALE_FIXTURE_ROOT="$(CURDIR)/fixtures/protocol" opam exec --switch=$(OPAM_SWITCH) -- dune runtest --root ocaml

verify:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --all-features -- -D warnings
	cargo test --workspace --all-features
	opam exec --switch=$(OPAM_SWITCH) -- dune build --root ocaml @all
	RATIONALE_FIXTURE_ROOT="$(CURDIR)/fixtures/protocol" opam exec --switch=$(OPAM_SWITCH) -- dune runtest --root ocaml
	git diff --check
