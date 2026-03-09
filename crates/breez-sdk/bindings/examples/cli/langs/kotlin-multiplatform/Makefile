GRADLE := ./gradlew
BINDINGS_DIR := ../../../..
KMP_DIR := $(BINDINGS_DIR)/langs/kotlin-multiplatform

.PHONY: help setup build run run-mainnet clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

setup: ## Build local Kotlin bindings from Rust source and publish to mavenLocal
	cd $(BINDINGS_DIR) && $(MAKE) build-release install-uniffi-bindgen-gobley
	@echo "Generating KMP bindings..."
	cd $(BINDINGS_DIR) && gobley-uniffi-bindgen --library \
		-o langs/kotlin-multiplatform/breez-sdk-spark-kmp/src \
		--crate-configs breez_sdk_spark_bindings=./uniffi.kotlin-multiplatform.toml \
		--crate-configs breez_sdk_spark=../core/uniffi.kotlin-multiplatform.toml \
		../../../target/release/libbreez_sdk_spark_bindings.dylib
	cd $(KMP_DIR) && ./gradlew :breez-sdk-spark-kmp:compileKotlinJvm :breez-sdk-spark-kmp:publishJvmPublicationToMavenLocal
	@echo "Local bindings published to mavenLocal. Run 'make build' to compile the CLI."

build: ## Build the CLI
	$(GRADLE) build

run: build ## Build + run on regtest (default)
	$(GRADLE) run --console=plain

run-mainnet: build ## Build + run on mainnet
	$(GRADLE) run --console=plain --args="--network mainnet"

clean: ## Remove build artifacts
	$(GRADLE) clean
