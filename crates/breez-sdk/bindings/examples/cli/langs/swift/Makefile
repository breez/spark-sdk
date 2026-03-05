SWIFT := swift
BINDINGS_DIR := ../../../..
SDK_PKG_DIR  := $(BINDINGS_DIR)/langs/swift
RELEASE_DIR  := $(BINDINGS_DIR)/../../../target/release
MACOS_FW_DIR := $(SDK_PKG_DIR)/breez_sdk_sparkFFI.xcframework/macos-arm64_x86_64/breez_sdk_sparkFFI.framework
MACOS_FW     := $(MACOS_FW_DIR)/Versions/A

.PHONY: help setup setup-published build run run-mainnet clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

setup: ## Build local Swift bindings from Rust source
	cd $(BINDINGS_DIR) && $(MAKE) bindings-swift
	mkdir -p $(SDK_PKG_DIR)/Sources/BreezSdkSpark
	cp $(BINDINGS_DIR)/ffi/swift/*.swift $(SDK_PKG_DIR)/Sources/BreezSdkSpark/
	cp $(BINDINGS_DIR)/ffi/swift/breez_sdk_*FFI.h $(MACOS_FW)/Headers/
	cp $(RELEASE_DIR)/libbreez_sdk_spark_bindings.dylib $(MACOS_FW)/breez_sdk_sparkFFI
	install_name_tool -id @rpath/breez_sdk_sparkFFI.framework/Versions/A/breez_sdk_sparkFFI $(MACOS_FW)/breez_sdk_sparkFFI
	ln -sf Versions/Current/breez_sdk_sparkFFI $(MACOS_FW_DIR)/breez_sdk_sparkFFI
	$(SWIFT) package resolve
	$(SWIFT) package edit breez-sdk-spark-swift --path $(SDK_PKG_DIR)
	@echo "Local bindings built. Run 'make build' to compile the CLI."

setup-published: ## Switch to published SDK and resolve dependencies
	$(SWIFT) package unedit breez-sdk-spark-swift 2>/dev/null || true
	$(SWIFT) package resolve

build: ## Build the CLI binary
	$(SWIFT) build

run: build ## Build + run on regtest (default)
	$(SWIFT) run breez-cli

run-mainnet: build ## Build + run on mainnet
	$(SWIFT) run breez-cli --network mainnet

clean: ## Remove build artifacts
	rm -rf .build
