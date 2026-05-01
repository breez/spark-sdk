# Path to the SDK React Native package (relative to this directory)
SDK_RN_DIR := ../../../../../../../packages/react-native

# React Native version must match package.json
RN_VERSION := 0.79.2

# Gradle version compatible with RN 0.79
GRADLE_VERSION := 8.13

.PHONY: help setup build build-sdk build-sdk-android build-sdk-ios run run-ios run-android typecheck clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

# ---------------------------------------------------------------------------
# Setup
# ---------------------------------------------------------------------------

setup: setup-deps setup-native setup-ios-pods ## Full setup: deps, native projects, pods
	@echo ""
	@echo "✅ Setup complete. Next steps:"
	@echo "  make build-sdk   Build SDK native binaries (interactive)"
	@echo "  make run         Run the app (interactive)"

setup-deps: ## Install npm dependencies
	EXPO_PUBLIC_SKIP_POSTINSTALL=1 npm install

setup-native: ## Scaffold iOS and Android native projects (pinned to RN $(RN_VERSION))
	@if [ ! -f ios/BreezSdkSparkCli.xcodeproj/project.pbxproj ] || [ ! -f android/settings.gradle ]; then \
		echo "Scaffolding native projects (RN $(RN_VERSION))..."; \
		rm -rf /tmp/rn-scaffold; \
		npx @react-native-community/cli@$(RN_VERSION) init BreezSdkSparkCli \
			--version $(RN_VERSION) \
			--directory /tmp/rn-scaffold \
			--skip-install --skip-git-init 2>/dev/null; \
		if [ ! -f ios/BreezSdkSparkCli.xcodeproj/project.pbxproj ]; then \
			rm -rf ios; \
			cp -r /tmp/rn-scaffold/ios . && echo "  iOS project generated."; \
		fi; \
		if [ ! -f android/settings.gradle ]; then \
			rm -rf android; \
			cp -r /tmp/rn-scaffold/android . && echo "  Android project generated."; \
		fi; \
		[ -f /tmp/rn-scaffold/Gemfile ] && cp /tmp/rn-scaffold/Gemfile . ; \
		rm -rf /tmp/rn-scaffold; \
		echo "Patching Gemfile for Ruby 4.0 compatibility..."; \
		if [ -f Gemfile ]; then \
			grep -q 'concurrent-ruby' Gemfile && \
				sed -i '' "/gem 'concurrent-ruby'/d" Gemfile; \
			grep -q 'nkf' Gemfile || \
				echo "gem 'nkf'" >> Gemfile; \
			grep -q 'ostruct' Gemfile || \
				echo "gem 'ostruct'" >> Gemfile; \
		fi; \
		echo "Pinning Gradle wrapper to $(GRADLE_VERSION)..."; \
		sed -i '' 's|gradle-[0-9.]*-bin.zip|gradle-$(GRADLE_VERSION)-bin.zip|' \
			android/gradle/wrapper/gradle-wrapper.properties; \
		echo "Adding usesCleartextTraffic manifest placeholder..."; \
		grep -q 'manifestPlaceholders' android/app/build.gradle || \
			sed -i '' '/versionName/a\
        manifestPlaceholders = [usesCleartextTraffic: "true"]' \
			android/app/build.gradle 2>/dev/null || true; \
		echo "Patching MainApplication.kt for RN $(RN_VERSION) compatibility..."; \
		MAIN_APP="android/app/src/main/java/com/breezsdksparkcli/MainApplication.kt"; \
		if grep -q 'ReactNativeApplicationEntryPoint\|loadReactNative' "$$MAIN_APP" 2>/dev/null; then \
			python3 -c "open('$$MAIN_APP','w').write('''package com.breezsdksparkcli\n\nimport android.app.Application\nimport com.facebook.react.PackageList\nimport com.facebook.react.ReactApplication\nimport com.facebook.react.ReactNativeHost\nimport com.facebook.react.ReactPackage\nimport com.facebook.react.defaults.DefaultNewArchitectureEntryPoint.load\nimport com.facebook.react.defaults.DefaultReactNativeHost\nimport com.facebook.soloader.SoLoader\n\nclass MainApplication : Application(), ReactApplication {\n\n  override val reactNativeHost: ReactNativeHost =\n    object : DefaultReactNativeHost(this) {\n      override fun getPackages(): List<ReactPackage> =\n        PackageList(this).packages.apply {}\n\n      override fun getJSMainModuleName(): String = \"index\"\n\n      override fun getUseDeveloperSupport(): Boolean = BuildConfig.DEBUG\n\n      override val isNewArchEnabled: Boolean = BuildConfig.IS_NEW_ARCHITECTURE_ENABLED\n      override val isHermesEnabled: Boolean = BuildConfig.IS_HERMES_ENABLED\n    }\n\n  override fun onCreate() {\n    super.onCreate()\n    SoLoader.init(this, false)\n    if (BuildConfig.IS_NEW_ARCHITECTURE_ENABLED) {\n      load()\n    }\n  }\n}\n''')" && \
			echo "  MainApplication.kt patched for RN $(RN_VERSION)."; \
		fi; \
	else \
		echo "Native projects already exist. Use 'make clean' first to regenerate."; \
	fi

setup-ios-pods: ## Install CocoaPods dependencies
	@if [ -d ios ]; then \
		echo "Installing CocoaPods..."; \
		cd ios && pod install && cd ..; \
		echo "  CocoaPods installed."; \
	fi

# ---------------------------------------------------------------------------
# SDK Native Binaries
# ---------------------------------------------------------------------------

build-sdk: ## Build SDK native binaries (interactive prompt)
	@printf "Select platform:\n  1) iOS\n  2) Android\n  3) Both\n> "; \
	read choice; \
	case "$$choice" in \
		1|ios)     $(MAKE) build-sdk-ios ;; \
		2|android) $(MAKE) build-sdk-android ;; \
		3|both)    $(MAKE) build-sdk-ios build-sdk-android ;; \
		*) echo "Invalid choice. Use 'make build-sdk-ios' or 'make build-sdk-android'." ;; \
	esac

build-sdk-android: ## Build SDK native .so libraries for Android (requires Rust + Android NDK)
	@echo "Building SDK native libraries for Android..."
	cd $(SDK_RN_DIR) && npx ubrn build android --release --config ubrn.config.yaml
	@echo "Android SDK binaries ready."

build-sdk-ios: ## Build SDK native xcframework for iOS (requires Rust + Xcode)
	@echo "Building SDK native libraries for iOS..."
	cd $(SDK_RN_DIR) && npx ubrn build ios --release --config ubrn.config.yaml
	@echo "iOS SDK binaries ready."

# ---------------------------------------------------------------------------
# Run
# ---------------------------------------------------------------------------

run: ## Run on iOS or Android (interactive prompt)
	@printf "Select platform:\n  1) iOS\n  2) Android\n> "; \
	read choice; \
	case "$$choice" in \
		1|ios)   $(MAKE) run-ios ;; \
		2|android) $(MAKE) run-android ;; \
		*) echo "Invalid choice. Use 'make run-ios' or 'make run-android'." ;; \
	esac

run-ios: ## Run on iOS device/simulator
	./node_modules/.bin/react-native run-ios

run-android: adb-reverse ## Run on Android device/emulator
	./node_modules/.bin/react-native run-android

adb-reverse: ## Forward Metro port to connected Android device
	@adb reverse tcp:8081 tcp:8081 2>/dev/null || true

dev: adb-reverse ## Start Metro dev server with port forwarding
	./node_modules/.bin/react-native start --port 8081 --reset-cache

# ---------------------------------------------------------------------------
# Utilities
# ---------------------------------------------------------------------------

build: typecheck ## Validate TypeScript compilation
	@echo "TypeScript compilation check passed."

typecheck: ## Run TypeScript type checking
	npx tsc --noEmit

clean: ## Remove build artifacts, native projects, and dependencies
	rm -rf node_modules ios android Gemfile Gemfile.lock
