FLUTTER := flutter
DART := dart
FLUTTER_PKG := ../../../../../../../packages/flutter

.PHONY: help setup setup-published run run-mainnet clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

setup: ## Build local Flutter bindings and install dependencies
	cd $(FLUTTER_PKG) && just codegen
	cd $(FLUTTER_PKG)/rust && cargo build --release
	$(FLUTTER) pub get

setup-published: ## Install dependencies from published SDK
	$(FLUTTER) pub get

run: ## Run CLI on regtest (default)
	$(DART) run bin/breez_cli.dart

run-mainnet: ## Run CLI on mainnet
	$(DART) run bin/breez_cli.dart --network mainnet

clean: ## Remove build artifacts
	rm -rf .dart_tool build
