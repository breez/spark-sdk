XTASK := cargo run -p xtask --

default: check

build:
	$(XTASK) build

build-wasm:
	$(XTASK) build --target wasm32-unknown-unknown

# This adds a make command to install a target, e.g. `make install-target-wasm32-unknown-unknown`
install-target-%:
	$(CLANG_PREFIX) rustup target add $*

build-target-%: install-target-%
	$(XTASK) build --target $*

build-release:
	$(XTASK) build --release

check: fmt-check clippy-check test

clippy-fix: cargo-clippy-fix wasm-clippy-fix

cargo-clippy-fix:
	$(XTASK) clippy --fix

wasm-clippy-fix:
	$(XTASK) wasm-clippy --fix

clippy-check: cargo-clippy-check wasm-clippy-check

cargo-clippy-check:
	$(XTASK) clippy

wasm-clippy-check:
	$(XTASK) wasm-clippy

fix: fmt-fix clippy-fix

fmt-fix:
	$(XTASK) fmt

fmt-check:
	$(XTASK) fmt --check

test: cargo-test wasm-test

cargo-test:
	$(XTASK) test

wasm-test: wasm-test-browser wasm-test-node

wasm-test-browser:
	$(XTASK) wasm-test

wasm-test-node:
	$(XTASK) wasm-test --node

itest:
	$(XTASK) itest