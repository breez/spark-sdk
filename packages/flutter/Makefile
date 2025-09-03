TARGET_DIR := ../../target/
INSTALL_PREFIX := CARGO_TARGET_DIR="$(TARGET_DIR)"

.PHONY: flutter
build-rust: 
	$(MAKE) -C rust build

build-rust-after-generate: generate-bindings
	$(MAKE) build-rust

flutter: build-rust-after-generate

generate-bindings: install-flutter-rust-bridge-codegen
	flutter_rust_bridge_codegen generate

install-flutter-rust-bridge-codegen:
	$(INSTALL_PREFIX) cargo install flutter_rust_bridge_codegen --version 2.11.1
