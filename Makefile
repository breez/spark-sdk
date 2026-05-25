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

check: fmt-check clippy-check test flutter-check

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

wasm-test: wasm-test-browser wasm-test-node wasm-test-mysql-timezone

wasm-test-browser:
	cargo xtask wasm-test

wasm-test-node:
	cargo xtask wasm-test --node

# Regression test for the JS mysql-{token,tree}-store TZ-handling bug. Runs
# the spent-marker / spent-leaf scenarios under several host TZs (positive
# and negative offsets) so the bug surfaces on any CI runner regardless of
# system clock. Depends on wasm-test-node for the upstream `npm install` of
# mysql-storage / mysql-{token,tree}-store deps that these tests reuse.
wasm-test-mysql-timezone: wasm-test-node
	cd crates/breez-sdk/wasm/js/mysql-token-store && npm test
	cd crates/breez-sdk/wasm/js/mysql-tree-store && npm test

flutter-check:
	cargo xtask flutter-check

itest:
	cargo xtask itest

spark-itest-pg:
	USE_POSTGRES_BACKEND=true cargo xtask itest

spark-itest-mysql:
	USE_MYSQL_BACKEND=true cargo xtask itest

breez-itest:
	cargo xtask test --package breez-sdk-itest -- --test-threads=8

breez-itest-pg-tree-store:
	USE_POSTGRES_TREE_STORE=true cargo xtask test --package breez-sdk-itest -- --test-threads=8

breez-itest-mysql-tree-store:
	USE_MYSQL_TREE_STORE=true cargo xtask test --package breez-sdk-itest -- --test-threads=8
claude-check:
	make fmt-check clippy-check cargo-test

open-core-rustdocs:
	cd crates/breez-sdk/core && cargo doc --no-deps --open

update-lockfiles:
	./scripts/update-lock-files.sh
