GRADLE := ./gradlew
KMP_CLI_DIR := ../../bindings/examples/cli/langs/kotlin-multiplatform

.PHONY: help setup build smoke-test run-server fund seed-senders loadgen clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

setup: ## Build and publish KMP bindings to mavenLocal (delegates to CLI's setup)
	cd $(KMP_CLI_DIR) && $(MAKE) setup

build: ## Compile the benchmark harness
	$(GRADLE) build

smoke-test: build ## Run per-request smoke (requires MYSQL_URL + MASTER_SECRET env vars)
	$(GRADLE) run --console=plain --args="--mode=smoke --mysql-url=$$MYSQL_URL --master-secret=$$MASTER_SECRET"

run-server: build ## Start the HTTP server (PORT defaults to 8080; needs MYSQL_URL + MASTER_SECRET)
	$(GRADLE) run --console=plain --args="--mode=server --mysql-url=$$MYSQL_URL --master-secret=$$MASTER_SECRET --port=$${PORT:-8080}"

fund: build ## Top up the treasurer (TARGET_SATS defaults to 5000000; needs MYSQL_URL + MASTER_SECRET + FAUCET_USERNAME + FAUCET_PASSWORD)
	$(GRADLE) run --console=plain --args="--mode=fund --mysql-url=$$MYSQL_URL --master-secret=$$MASTER_SECRET --target-sats=$${TARGET_SATS:-5000000}"

seed-senders: build ## Top up the K sender wallets from the treasurer (SENDERS=50 MIN_SATS=10000 TARGET_SATS=50000 by default)
	$(GRADLE) run --console=plain --args="--mode=seed-senders --mysql-url=$$MYSQL_URL --master-secret=$$MASTER_SECRET --senders=$${SENDERS:-50} --min-sats=$${MIN_SATS:-10000} --target-sats=$${TARGET_SATS:-50000}"

loadgen: build ## Run the load generator against the bench server (TARGET_RPS+DURATION required; BASE_URL defaults to http://localhost:8080)
	$(GRADLE) run --console=plain --args="--mode=loadgen --base-url=$${BASE_URL:-http://localhost:8080} --target-rps=$$TARGET_RPS --duration=$$DURATION --users=$${USERS:-10000} --mix=$${MIX:-info=40,receive=30,send=30} --user-distribution=$${DIST:-uniform} --warmup-secs=$${WARMUP_SECS:-60} --senders=$${SENDERS:-50} --payment-sats=$${PAYMENT_SATS:-1}$${RUN_ID:+ --run-id=$$RUN_ID}"

clean: ## Remove build artifacts
	$(GRADLE) clean
