NPM := npm
NODE := node
WASM_PKG := ../../../../../../../packages/wasm

.PHONY: help setup build run run-mainnet clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

setup: ## Build local WASM bindings and install dependencies
	cd $(WASM_PKG) && $(MAKE) build-node
	$(NPM) install
	@echo "Local bindings ready. Run 'make run' to start the CLI."

build: ## No-op for Node.js (interpreted language)
	@echo "No build step needed for Node.js"

run: ## Run on regtest (default)
	$(NODE) src/main.js

run-mainnet: ## Run on mainnet
	$(NODE) src/main.js --network mainnet

clean: ## Remove node_modules and data directory
	rm -rf node_modules .data
