FLUTTER := flutter
DART := dart
BINDINGS_DIR := ../../../..

.PHONY: help setup setup-published run run-mainnet clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

setup: ## Install dependencies from local Flutter bindings
	$(FLUTTER) pub get

setup-published: ## Install dependencies from published SDK
	$(FLUTTER) pub get

run: ## Run CLI on regtest (default)
	$(DART) run bin/breez_cli.dart

run-mainnet: ## Run CLI on mainnet
	$(DART) run bin/breez_cli.dart --network mainnet

clean: ## Remove build artifacts
	rm -rf .dart_tool build
