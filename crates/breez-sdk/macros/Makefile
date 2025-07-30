UNAME := $(shell uname)

ifeq ($(UNAME), Darwin)
	CLANG_PREFIX += AR=$(shell brew --prefix llvm)/bin/llvm-ar CC=$(shell brew --prefix llvm)/bin/clang
endif

default: check

build:
	cargo build

build-release:
	cargo build --release

build-wasm: install-wasm32-unknown-unknown
	$(CLANG_PREFIX) cargo build --target=wasm32-unknown-unknown

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

install-wasm32-unknown-unknown:
	$(CLANG_PREFIX) rustup target add wasm32-unknown-unknown

test:
	cargo test

wasm-clippy-check: install-wasm32-unknown-unknown
	$(CLANG_PREFIX) cargo clippy --all-targets --target=wasm32-unknown-unknown -- -D warnings
