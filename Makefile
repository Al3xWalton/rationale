OPAM_SWITCH := rationale-5.5.1
BENCH_SCALE ?= ava-like
BENCH_SAMPLES ?= 30
BENCH_OUTPUT ?= benchmarks/results/latest.json

.PHONY: audit benchmark build demo fmt fuzz-check lint test verify

audit:
	cargo deny check
	cargo deny --manifest-path fuzz/Cargo.toml check

benchmark:
	opam exec --switch=$(OPAM_SWITCH) -- dune build --root ocaml @all
	cargo build --release --workspace
	target/release/rationale-bench --scale $(BENCH_SCALE) --samples $(BENCH_SAMPLES) --output $(BENCH_OUTPUT)

build:
	opam exec --switch=$(OPAM_SWITCH) -- dune build --root ocaml @all
	cargo build --workspace

demo:
	scripts/run-demo.sh

fmt:
	cargo fmt --all
	opam exec --switch=$(OPAM_SWITCH) -- dune fmt --root ocaml

fuzz-check:
	cargo check --manifest-path fuzz/Cargo.toml --bins

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
	cargo check --manifest-path fuzz/Cargo.toml --bins
	git diff --check
