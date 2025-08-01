include ../../../Makefile.common

default: check

build:
	cargo build

build-release:
	cargo build --release

build-target-%: install-target-%
	$(CLANG_PREFIX) cargo build --target=$*

build-wasm: build-target-wasm32-unknown-unknown

check: fmt-check clippy-check wasm-clippy-check

clippy: clippy-check

clippy-fix:
	cargo clippy --fix -- -D warnings
	cargo clippy --fix --tests -- -D warnings

clippy-check:
	cargo clippy -- -D warnings
	cargo clippy --tests -- -D warnings

fix: fmt-fix clippy-fix

fmt: fmt-fix

fmt-fix:
	cargo fmt

fmt-check:
	cargo fmt -- --check

test:
	cargo test

wasm-clippy-check: install-target-wasm32-unknown-unknown
	$(CLANG_PREFIX) cargo clippy --all-targets --target=wasm32-unknown-unknown -- -D warnings
