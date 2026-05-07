GRADLE := ./gradlew
KMP_CLI_DIR := ../../bindings/examples/cli/langs/kotlin-multiplatform

.PHONY: help setup build smoke-test clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

setup: ## Build and publish KMP bindings to mavenLocal (delegates to CLI's setup)
	cd $(KMP_CLI_DIR) && $(MAKE) setup

build: ## Compile the benchmark harness
	$(GRADLE) build

smoke-test: build ## Run per-request smoke (requires MYSQL_URL + MASTER_SECRET env vars)
	$(GRADLE) run --console=plain --args="--mode=smoke --mysql-url=$$MYSQL_URL --master-secret=$$MASTER_SECRET"

clean: ## Remove build artifacts
	$(GRADLE) clean
