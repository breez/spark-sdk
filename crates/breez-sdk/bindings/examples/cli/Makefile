PYTHON_SRC   := langs/python/src/breez_cli
GO_SRC       := langs/go
DART_SRC     := langs/dart
SWIFT_SRC    := langs/swift

.PHONY: help check-python check-go check-dart check-swift

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-24s %s\n", $$1, $$2}'

check-python: ## Syntax-check Python CLI source files
	python3 -m py_compile $(PYTHON_SRC)/*.py

check-go: ## Check Go CLI (format + build + vet)
	cd $(GO_SRC) && gofmt -e *.go > /dev/null
	cd $(GO_SRC) && go build ./...
	cd $(GO_SRC) && go vet ./...

check-dart: ## Check Dart CLI (format + analyze)
	cd $(DART_SRC) && dart pub get
	cd $(DART_SRC) && dart format -l 110 --set-exit-if-changed lib/ bin/
	cd $(DART_SRC) && dart analyze --fatal-infos lib/ bin/

check-swift: ## Check Swift CLI (build with local bindings)
	cd $(SWIFT_SRC) && $(MAKE) setup
	cd $(SWIFT_SRC) && $(MAKE) build
