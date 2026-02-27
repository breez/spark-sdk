BINARY := breez-cli
GO := go
BINDINGS_DIR := ../../../..

.PHONY: help setup setup-published build run run-mainnet clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

setup: ## Generate local Go bindings from Rust source
	cd $(BINDINGS_DIR) && $(MAKE) bindings-golang
	@echo "Local bindings generated. Ensure the replace directive in go.mod is uncommented."

setup-published: ## Download dependencies from published SDK
	$(GO) mod download

build: ## Build the CLI binary
	$(GO) build -o $(BINARY) .

run: build ## Build + run on regtest (default)
	./$(BINARY)

run-mainnet: build ## Build + run on mainnet
	./$(BINARY) --network mainnet

clean: ## Remove binary
	rm -f $(BINARY)
