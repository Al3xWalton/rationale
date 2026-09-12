OPAM_SWITCH := rationale-5.5.1

.PHONY: build demo fmt lint test verify

build:
	opam exec --switch=$(OPAM_SWITCH) -- dune build --root ocaml @all
	cargo build --workspace

demo:
	scripts/run-demo.sh

fmt:
	cargo fmt --all
	opam exec --switch=$(OPAM_SWITCH) -- dune fmt --root ocaml

lint:
	cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
	opam exec --switch=$(OPAM_SWITCH) -- dune build --root ocaml @all
	RATIONALE_KERNEL_WORKER="$(CURDIR)/ocaml/_build/default/worker/main.exe" RATIONALE_FIXTURE_ROOT="$(CURDIR)/fixtures/protocol" cargo test --workspace --all-features
	RATIONALE_FIXTURE_ROOT="$(CURDIR)/fixtures/protocol" opam exec --switch=$(OPAM_SWITCH) -- dune runtest --root ocaml

verify:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --all-features -- -D warnings
	opam exec --switch=$(OPAM_SWITCH) -- dune build --root ocaml @all
	RATIONALE_KERNEL_WORKER="$(CURDIR)/ocaml/_build/default/worker/main.exe" RATIONALE_FIXTURE_ROOT="$(CURDIR)/fixtures/protocol" cargo test --workspace --all-features
	RATIONALE_FIXTURE_ROOT="$(CURDIR)/fixtures/protocol" opam exec --switch=$(OPAM_SWITCH) -- dune runtest --root ocaml
	git diff --check
