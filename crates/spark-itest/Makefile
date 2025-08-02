include ../../Makefile.common

default: check

build:
build-containers: build-so-container build-postgres-container build-bitcoind-container build-migrations-container

build-bitcoind-container:
	docker image pull lncm/bitcoind:v28.0

build-migrations-container:
	docker build -t spark-migrations -f docker/migrations.dockerfile ./docker

build-postgres-container:
	docker image pull postgres:11-alpine

build-so-container:
	docker build -t spark-so -f docker/spark-so.dockerfile ./docker

build-release:
build-target-%:
build-wasm:

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

itest: test

test: build-containers
	cargo test

wasm-clippy-check: install-target-wasm32-unknown-unknown
	$(CLANG_PREFIX) cargo clippy --all-targets --target=wasm32-unknown-unknown -- -D warnings
