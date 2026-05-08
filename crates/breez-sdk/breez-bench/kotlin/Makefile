GRADLE := ./gradlew
KMP_CLI_DIR := ../../bindings/examples/cli/langs/kotlin-multiplatform

# Standard env defaults — see plan + README. Reusing MASTER_SECRET across
# sessions keeps treasurer + senders funded between runs. MYSQL_URL points
# at the container `mysql-up` brings up.
export MASTER_SECRET ?= breez-bench
export MYSQL_URL ?= mysql://root:password@127.0.0.1:3306/breez_bench

.PHONY: help setup mysql-up build smoke-test run-server fund seed-senders loadgen sweep aggregate run trace-sync clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

setup: ## Build and publish KMP bindings to mavenLocal (one-time per branch)
	cd $(KMP_CLI_DIR) && $(MAKE) setup

mysql-up: ## Start the bench MySQL container + create the bench DB (idempotent)
	@if ! docker ps --format '{{.Names}}' | grep -q '^bench-mysql$$'; then \
	  echo "Starting bench-mysql container…"; \
	  docker run --rm -d --name bench-mysql -p 3306:3306 \
	    -e MYSQL_ROOT_PASSWORD=password mysql:8.0 >/dev/null; \
	fi
	@echo "Waiting for MySQL to accept connections…"
	@until docker exec bench-mysql mysqladmin ping -uroot -ppassword --silent 2>/dev/null; do sleep 1; done
	@docker exec bench-mysql mysql -uroot -ppassword \
	  -e "CREATE DATABASE IF NOT EXISTS breez_bench" 2>/dev/null
	@echo "MySQL ready at $$MYSQL_URL"

build: ## Compile the benchmark harness
	$(GRADLE) build

smoke-test: build ## Single SDK connect→getInfo→disconnect; sanity check the binding + MySQL path
	$(GRADLE) run --console=plain --args="--mode=smoke --mysql-url=$$MYSQL_URL --master-secret=$$MASTER_SECRET"

run-server: build ## Start the HTTP server only (PORT defaults to 8080; RUN_ID optional)
	$(GRADLE) run --console=plain --args="--mode=server --mysql-url=$$MYSQL_URL --master-secret=$$MASTER_SECRET --port=$${PORT:-8080}$${RUN_ID:+ --run-id=$$RUN_ID}"

fund: build ## Top up the treasurer (TARGET_SATS=N; needs FAUCET_USERNAME + FAUCET_PASSWORD)
	$(GRADLE) run --console=plain --args="--mode=fund --mysql-url=$$MYSQL_URL --master-secret=$$MASTER_SECRET --target-sats=$${TARGET_SATS:-5000000}"

seed-senders: build ## Top each of K sender wallets up to PER_SENDER_SATS from the treasurer
	$(GRADLE) run --console=plain --args="--mode=seed-senders --mysql-url=$$MYSQL_URL --master-secret=$$MASTER_SECRET --senders=$${SENDERS:-50} --per-sender-sats=$${PER_SENDER_SATS:-5000}"

loadgen: build ## Run the load generator against an already-running server (TARGET_RPS + DURATION required)
	$(GRADLE) run --console=plain --args="--mode=loadgen --base-url=$${BASE_URL:-http://localhost:8080} --target-rps=$$TARGET_RPS --duration=$$DURATION --users=$${USERS:-10000} --mix=$${MIX:-info=40,receive=30,send=30} --user-distribution=$${DIST:-uniform} --senders=$${SENDERS:-50} --payment-sats=$${PAYMENT_SATS:-1}$${RUN_ID:+ --run-id=$$RUN_ID}"

sweep: build ## RPS sweep only (no aggregation). See scripts/sweep.sh for env knobs.
	./scripts/sweep.sh

aggregate: ## Aggregate a sweep into summary.json + RESULTS.md (SWEEP_ID required)
	@test -n "$$SWEEP_ID" || (echo "SWEEP_ID is required (e.g. SWEEP_ID=2026-05-08T15-00-00)"; exit 2)
	python3 scripts/aggregate.py --sweep-dir out/$$SWEEP_ID

trace-sync: build ## Trace ensureSynced=true on a wallet with full Rust tracing (USER_ID defaults to __treasurer__; LOG_FILTER=debug)
	$(GRADLE) run --console=plain --args="--mode=trace-sync --mysql-url=$$MYSQL_URL --master-secret=$$MASTER_SECRET --user-id=$${USER_ID:-__treasurer__} --log-filter=$${LOG_FILTER:-debug}"

run: build ## Sweep + aggregate in one shot. Auto-funds treasurer + senders. Default fast pass (~7 min).
	@SWEEP_ID=$${SWEEP_ID:-$$(date +%Y-%m-%dT%H-%M-%S)}; \
	  export SWEEP_ID; \
	  : "$${SWEEP_RPS:=50,100,250}"; export SWEEP_RPS; \
	  : "$${DURATION:=2m}"; export DURATION; \
	  ./scripts/sweep.sh && \
	  python3 scripts/aggregate.py --sweep-dir out/$$SWEEP_ID && \
	  echo && echo "Done. See out/$$SWEEP_ID/RESULTS.md"

clean: ## Remove build artifacts
	$(GRADLE) clean
