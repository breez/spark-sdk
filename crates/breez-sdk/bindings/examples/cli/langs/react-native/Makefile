.PHONY: help setup build run run-ios run-android typecheck clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

setup: ## Install dependencies and generate native projects
	EXPO_PUBLIC_SKIP_POSTINSTALL=1 npm install
	@if [ ! -d ios ] || [ ! -d android ]; then \
		echo "Generating native projects..."; \
		npx @react-native-community/cli init BreezSdkSparkCli --directory /tmp/rn-scaffold --skip-install --skip-git-init 2>/dev/null; \
		[ ! -d ios ] && cp -r /tmp/rn-scaffold/ios . && echo "iOS project generated."; \
		[ ! -d android ] && cp -r /tmp/rn-scaffold/android . && echo "Android project generated."; \
		[ -f /tmp/rn-scaffold/Gemfile ] && cp /tmp/rn-scaffold/Gemfile . ; \
		rm -rf /tmp/rn-scaffold; \
	fi
	@echo "Dependencies installed. Run 'make run' to start."

run: ## Run on iOS or Android (interactive prompt)
	@printf "Select platform:\n  1) iOS\n  2) Android\n> "; \
	read choice; \
	case "$$choice" in \
		1|ios)   $(MAKE) run-ios ;; \
		2|android) $(MAKE) run-android ;; \
		*) echo "Invalid choice. Use 'make run-ios' or 'make run-android'." ;; \
	esac

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
