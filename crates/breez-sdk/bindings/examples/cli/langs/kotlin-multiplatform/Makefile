GRADLE := gradle
BINDINGS_DIR := ../../../..
KMP_DIR := $(BINDINGS_DIR)/langs/kotlin-multiplatform

.PHONY: help setup build run run-mainnet clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

setup: ## Build local Kotlin bindings from Rust source and publish to mavenLocal
	cd $(BINDINGS_DIR) && $(MAKE) bindings-kotlin
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
