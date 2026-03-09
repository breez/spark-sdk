.PHONY: help setup build run-ios run-android typecheck clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

setup: ## Install dependencies
	npm install
	@echo "Dependencies installed. Run 'make run-ios' or 'make run-android' to start."

build: typecheck ## Validate TypeScript compilation
	@echo "TypeScript compilation check passed."

typecheck: ## Run TypeScript type checking
	npx tsc --noEmit

run-ios: ## Run on iOS simulator
	npx react-native run-ios

run-android: ## Run on Android emulator
	npx react-native run-android

clean: ## Remove build artifacts and dependencies
	rm -rf node_modules ios/Pods ios/build android/build
