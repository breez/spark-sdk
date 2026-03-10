DOTNET := dotnet
PROJECT := BreezCli.csproj
BINDINGS_DIR := ../../../..

.PHONY: help setup build run run-mainnet clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

setup: ## Install .NET SDK (if needed) and build local C# bindings from Rust source
	@command -v dotnet >/dev/null 2>&1 || { echo "Installing .NET SDK..."; brew install dotnet; }
	cd $(BINDINGS_DIR) && $(MAKE) bindings-csharp
	cp $(BINDINGS_DIR)/ffi/csharp/breez_sdk_spark.cs $(BINDINGS_DIR)/langs/csharp/src/
	@echo "Local bindings ready. Run 'make build' to compile the CLI."

build: ## Build the CLI project
	$(DOTNET) build $(PROJECT) -c Release

run: build ## Build + run on regtest (default)
	$(DOTNET) run --project $(PROJECT) -c Release

run-mainnet: build ## Build + run on mainnet
	$(DOTNET) run --project $(PROJECT) -c Release -- --network mainnet

clean: ## Clean build artifacts
	$(DOTNET) clean $(PROJECT)
	rm -rf bin obj
