PYTHON_SRC   := langs/python/src/breez_cli
GO_SRC       := langs/go
DART_SRC     := langs/dart
SWIFT_SRC    := langs/swift
TS_SRC       := langs/typescript
CSHARP_SRC   := langs/csharp
KOTLIN_SRC   := langs/kotlin
RN_SRC       := langs/react-native

.PHONY: help check-python check-go check-dart check-swift check-typescript check-csharp check-kotlin check-react-native

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-24s %s\n", $$1, $$2}'

check-python: ## Syntax-check Python CLI source files
	python3 -m py_compile $(PYTHON_SRC)/*.py

check-go: ## Check Go CLI (build with local bindings)
	cd $(GO_SRC) && $(MAKE) setup
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

check-typescript: ## Check TypeScript CLI (syntax validation)
	cd $(TS_SRC) && node --check src/*.js

check-csharp: ## Check C# CLI (build with local bindings)
	cd $(CSHARP_SRC) && $(MAKE) setup
	cd $(CSHARP_SRC) && dotnet build

check-kotlin: ## Check Kotlin CLI (build with local bindings)
	cd $(KOTLIN_SRC) && $(MAKE) setup
	cd $(KOTLIN_SRC) && gradle build

check-react-native: ## Check React Native CLI (type check only)
	cd $(RN_SRC) && npm install --ignore-scripts
	cd $(RN_SRC) && npx tsc --noEmit
