BINARY := breez-cli
GO := go
BINDINGS_DIR := ../../../..
FFI_GOLANG := $(BINDINGS_DIR)/ffi/golang

.PHONY: help setup setup-published build run run-mainnet clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

setup: ## Build local Go bindings from Rust source
	cd $(BINDINGS_DIR) && $(MAKE) bindings-golang
	@printf 'module github.com/breez/breez-sdk-spark-go\n\ngo 1.19\n' > $(FFI_GOLANG)/go.mod
	@printf 'package breez_sdk_spark\n\n/*\n#cgo LDFLAGS: -lbreez_sdk_spark_bindings\n#cgo darwin LDFLAGS: -Wl,-rpath,$${SRCDIR}/../../../../../../target/release -L$${SRCDIR}/../../../../../../target/release\n#cgo linux LDFLAGS: -Wl,-rpath,$${SRCDIR}/../../../../../../target/release -L$${SRCDIR}/../../../../../../target/release\n*/\nimport "C"\n' > $(FFI_GOLANG)/breez_sdk_spark/cgo.go
	@echo "Local bindings ready. Run 'make build' to compile the CLI."

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
