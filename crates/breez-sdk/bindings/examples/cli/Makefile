PYTHON_SRC   := langs/python/src/breez_cli
GO_SRC       := langs/golang
FLUTTER_SRC  := langs/flutter
SWIFT_SRC    := langs/swift
WASM_SRC     := langs/wasm
CSHARP_SRC   := langs/csharp
KOTLIN_SRC   := langs/kotlin-multiplatform
RN_SRC       := langs/react-native

.PHONY: help check-python check-golang check-flutter check-swift check-wasm check-csharp check-kotlin-multiplatform check-react-native

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-30s %s\n", $$1, $$2}'

check-python: ## Syntax-check Python CLI source files
	python3 -m py_compile $(PYTHON_SRC)/*.py

check-golang: ## Check Go CLI (build with local bindings)
	cd $(GO_SRC) && $(MAKE) setup
	cd $(GO_SRC) && gofmt -e *.go > /dev/null
	cd $(GO_SRC) && go build ./...
	cd $(GO_SRC) && go vet ./...

check-flutter: ## Check Flutter CLI (format + analyze)
	cd $(FLUTTER_SRC) && dart pub get
	cd $(FLUTTER_SRC) && dart format -l 110 --set-exit-if-changed lib/ bin/
	cd $(FLUTTER_SRC) && dart analyze --fatal-infos lib/ bin/

check-swift: ## Check Swift CLI (build with local bindings)
	cd $(SWIFT_SRC) && $(MAKE) setup
	cd $(SWIFT_SRC) && $(MAKE) build

check-wasm: ## Check WASM/TypeScript CLI (syntax validation)
	cd $(WASM_SRC) && node --check src/*.js

check-csharp: ## Check C# CLI (build with local bindings)
	cd $(CSHARP_SRC) && $(MAKE) setup
	cd $(CSHARP_SRC) && dotnet build

check-kotlin-multiplatform: ## Check Kotlin CLI (build with local bindings)
	cd $(KOTLIN_SRC) && $(MAKE) setup
	cd $(KOTLIN_SRC) && ./gradlew build

check-react-native: ## Check React Native CLI (type check only)
	@# npm file: protocol runs prepare on the target (bob build), so we
	@# strip the SDK dep, install everything else, and let tsconfig paths
	@# resolve the SDK import directly from source.
	cd $(RN_SRC) && cp package.json package.json.bak && \
		node -e "var p=require('./package.json'); \
			delete p.dependencies['@breeztech/breez-sdk-spark-react-native']; \
			require('fs').writeFileSync('package.json',JSON.stringify(p,null,2));" && \
		npm install --ignore-scripts --no-package-lock; \
		mv package.json.bak package.json
	cd $(RN_SRC) && npx tsc --noEmit
