SYNC_PROMPTS := sync-prompts/generate.py
PYTHON_SRC   := langs/python/src/breez_cli
GO_SRC       := langs/go
DART_SRC     := langs/dart

.PHONY: help sync-prompts-check sync-prompts-generate check-python check-go check-dart

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-24s %s\n", $$1, $$2}'

sync-prompts-check: ## Verify generated workflow files are up-to-date
	python3 $(SYNC_PROMPTS) --check

sync-prompts-generate: ## Regenerate workflow files from templates
	python3 $(SYNC_PROMPTS)

check-python: ## Syntax-check Python CLI source files
	python3 -m py_compile $(PYTHON_SRC)/*.py

check-go: ## Syntax-check Go CLI source files
	cd $(GO_SRC) && gofmt -e *.go > /dev/null

check-dart: ## Syntax-check Dart CLI source files
	cd $(DART_SRC) && dart format --set-exit-if-changed lib/ bin/
