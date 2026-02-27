VENV := .venv
PYTHON := $(VENV)/bin/python
PIP := $(VENV)/bin/pip
CLI := $(VENV)/bin/breez-cli
BINDINGS_DIR := ../../../..
LANGS_PYTHON := $(BINDINGS_DIR)/langs/python
RELEASE_DIR := $(BINDINGS_DIR)/../../../target/release

UNAME := $(shell uname)
ifeq ($(UNAME), Darwin)
	LIB_EXT := dylib
else ifeq ($(UNAME), Linux)
	LIB_EXT := so
endif
LIB_FILE := libbreez_sdk_spark_bindings.$(LIB_EXT)

.PHONY: help setup setup-published run run-mainnet clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

$(VENV)/bin/activate:
	python3 -m venv $(VENV)

setup: $(VENV)/bin/activate ## Build and install local Python bindings + CLI
	cd $(BINDINGS_DIR) && $(MAKE) bindings-python
	cp $(BINDINGS_DIR)/ffi/python/*.py $(LANGS_PYTHON)/src/breez_sdk_spark/
	cp $(RELEASE_DIR)/$(LIB_FILE) $(LANGS_PYTHON)/src/breez_sdk_spark/
	$(PIP) install $(LANGS_PYTHON)/
	$(PIP) install -e .
	@touch $(CLI)

setup-published: $(VENV)/bin/activate ## Install published SDK + CLI from PyPI
	$(PIP) install -e .
	@touch $(CLI)

run: $(CLI) ## Run CLI on regtest (default)
	$(CLI)

run-mainnet: $(CLI) ## Run CLI on mainnet
	$(CLI) --network mainnet

clean: ## Remove venv and build artifacts
	rm -rf $(VENV)
	find . -type d -name __pycache__ -exec rm -rf {} + 2>/dev/null || true
	find . -type d -name '*.egg-info' -exec rm -rf {} + 2>/dev/null || true
