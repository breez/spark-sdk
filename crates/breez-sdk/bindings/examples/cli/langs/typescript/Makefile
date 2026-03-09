NPM := npm
NODE := node

.PHONY: help setup setup-published build run run-mainnet clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

setup: ## Install dependencies using the published SDK package
	$(NPM) install

setup-published: setup ## Alias for setup (Node.js uses npm packages directly)

build: ## No-op for Node.js (interpreted language)
	@echo "No build step needed for Node.js"

run: setup ## Install dependencies + run on regtest (default)
	$(NODE) src/main.js

run-mainnet: setup ## Install dependencies + run on mainnet
	$(NODE) src/main.js --network mainnet

clean: ## Remove node_modules and data directory
	rm -rf node_modules .data
