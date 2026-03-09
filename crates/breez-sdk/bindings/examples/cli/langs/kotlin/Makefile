GRADLE := gradle

.PHONY: help setup setup-published build run run-mainnet clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

setup: ## Build local Kotlin bindings from Rust source (builds KMP JVM jar)
	cd ../../../.. && $(MAKE) bindings-kotlin
	@echo "Local bindings ready. Run 'make build' to compile the CLI."

setup-published: ## Download dependencies from published SDK
	$(GRADLE) dependencies
	@echo "Dependencies downloaded. Run 'make build' to compile the CLI."

build: ## Build the CLI
	$(GRADLE) build

run: build ## Build + run on regtest (default)
	$(GRADLE) run --console=plain

run-mainnet: build ## Build + run on mainnet
	$(GRADLE) run --console=plain --args="--network mainnet"

clean: ## Remove build artifacts
	$(GRADLE) clean
