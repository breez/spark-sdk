GRADLE := ./gradlew
BINDINGS_DIR := ../../../..
KMP_DIR := $(BINDINGS_DIR)/langs/kotlin-multiplatform

UNAME := $(shell uname)
ifeq ($(UNAME), Darwin)
	LIB_EXT := dylib
	ARCH := $(shell uname -m)
	ifeq ($(ARCH), arm64)
		JVM_RESOURCE_ARCH := darwin-aarch64
	else
		JVM_RESOURCE_ARCH := darwin-x86-64
	endif
else
	LIB_EXT := so
	ARCH := $(shell uname -m)
	ifeq ($(ARCH), aarch64)
		JVM_RESOURCE_ARCH := linux-aarch64
	else
		JVM_RESOURCE_ARCH := linux-x86-64
	endif
endif
JVM_RESOURCES_DIR := $(KMP_DIR)/breez-sdk-spark-kmp/src/jvmMain/resources

.PHONY: help setup build run run-mainnet clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

setup: ## Build local Kotlin bindings from Rust source and publish to mavenLocal
	cd $(BINDINGS_DIR) && $(MAKE) build-release install-uniffi-bindgen-gobley
	@echo "Copying native library for JVM ($(JVM_RESOURCE_ARCH))..."
	mkdir -p $(JVM_RESOURCES_DIR)/$(JVM_RESOURCE_ARCH)
	cp $(BINDINGS_DIR)/../../../target/release/libbreez_sdk_spark_bindings.$(LIB_EXT) \
		$(JVM_RESOURCES_DIR)/$(JVM_RESOURCE_ARCH)/libbreez_sdk_spark_bindings.$(LIB_EXT)
	@echo "Generating KMP bindings..."
	cd $(BINDINGS_DIR) && gobley-uniffi-bindgen --library \
		-o langs/kotlin-multiplatform/breez-sdk-spark-kmp/src \
		--crate-configs breez_sdk_spark_bindings=./uniffi.kotlin-multiplatform.toml \
		--crate-configs breez_sdk_spark=../core/uniffi.kotlin-multiplatform.toml \
		../../../target/release/libbreez_sdk_spark_bindings.$(LIB_EXT)
	cd $(KMP_DIR) && ./gradlew :breez-sdk-spark-kmp:compileKotlinJvm :breez-sdk-spark-kmp:publishJvmPublicationToMavenLocal
	@echo "Local bindings published to mavenLocal. Run 'make build' to compile the CLI."

build: ## Build the CLI
	$(GRADLE) build

run: build ## Build + run on regtest (default)
	$(GRADLE) run --console=plain

run-mainnet: build ## Build + run on mainnet
	$(GRADLE) run --console=plain --args="--network mainnet"

clean: ## Remove build artifacts
	$(GRADLE) clean
