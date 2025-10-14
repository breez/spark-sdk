default: check

build:
	cargo xtask build

build-wasm:
	cargo xtask build --target wasm32-unknown-unknown

# This adds a make command to install a target, e.g. `make install-target-wasm32-unknown-unknown`
install-target-%:
	$(CLANG_PREFIX) rustup target add $*

build-target-%: install-target-%
	cargo xtask build --target $*

build-release:
	cargo xtask build --release

check: fmt-check clippy-check test

clippy-fix: cargo-clippy-fix wasm-clippy-fix

cargo-clippy-fix:
	cargo xtask clippy --fix

wasm-clippy-fix:
	cargo xtask wasm-clippy --fix

clippy-check: cargo-clippy-check wasm-clippy-check

cargo-clippy-check:
	cargo xtask clippy

wasm-clippy-check:
	cargo xtask wasm-clippy

fix: fmt-fix clippy-fix

fmt-fix:
	cargo xtask fmt

fmt-check:
	cargo xtask fmt --check

test: cargo-test wasm-test

cargo-test:
	cargo xtask test

wasm-test: wasm-test-browser wasm-test-node

wasm-test-browser:
	cargo xtask wasm-test

wasm-test-node:
	cargo xtask wasm-test --node

itest:
	cargo xtask itest

breez-itest:
	cargo xtask test --package breez-sdk-itest -- --test-threads=1 --no-capture