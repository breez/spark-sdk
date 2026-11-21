DART := dart

.PHONY: help setup run run-mainnet clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

setup: ## Install dependencies and build the SDK's native library
	$(DART) pub get
	$(DART) run bin/breez_cli.dart --help > /dev/null

run: ## Run CLI on regtest (default)
	$(DART) run bin/breez_cli.dart

run-mainnet: ## Run CLI on mainnet
	$(DART) run bin/breez_cli.dart --network mainnet

clean: ## Remove build artifacts
	rm -rf .dart_tool build
