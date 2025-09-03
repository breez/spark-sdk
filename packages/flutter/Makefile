UNAME := $(shell uname)
TARGET_DIR := rust/target/
RELEASE_DIR := $(TARGET_DIR)release
INSTALL_PREFIX := CARGO_TARGET_DIR="$(TARGET_DIR)"

ifeq ($(UNAME), Darwin)
	CLANG_PREFIX += AR=$(shell brew --prefix llvm)/bin/llvm-ar CC=$(shell brew --prefix llvm)/bin/clang
	BIN_EXT := dylib
else ifeq ($(UNAME), Linux)
	BIN_EXT := so
endif

BIN_NAME := libbreez_sdk_spark_flutter
BIN_PATH := $(RELEASE_DIR)/$(BIN_NAME).$(BIN_EXT)

build-release: 
	cd rust && cargo build --release

generate-bindings: install-flutter-rust-bridge-codegen
	flutter_rust_bridge_codegen generate

generate-bindings-build-release: generate-bindings build-release

install-flutter-rust-bridge-codegen:
	$(INSTALL_PREFIX) cargo install flutter_rust_bridge_codegen --version 2.11.1

test: generate-bindings-build-release
	flutter test