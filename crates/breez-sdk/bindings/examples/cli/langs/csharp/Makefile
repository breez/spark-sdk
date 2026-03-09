DOTNET := dotnet
PROJECT := BreezCli.csproj

.PHONY: help setup setup-published build run run-mainnet clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

setup: ## Restore NuGet packages (local development)
	$(DOTNET) restore $(PROJECT)

setup-published: ## Restore NuGet packages from published SDK
	$(DOTNET) restore $(PROJECT)

build: ## Build the CLI project
	$(DOTNET) build $(PROJECT) -c Release

run: build ## Build + run on regtest (default)
	$(DOTNET) run --project $(PROJECT) -c Release

run-mainnet: build ## Build + run on mainnet
	$(DOTNET) run --project $(PROJECT) -c Release -- --network mainnet

clean: ## Clean build artifacts
	$(DOTNET) clean $(PROJECT)
	rm -rf bin obj
