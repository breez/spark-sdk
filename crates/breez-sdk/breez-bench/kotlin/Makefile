# Recipes use bash so `PIPESTATUS` (used by the `run` target to capture
# the exit code of sweep + aggregate through `tee`) is available.
SHELL := /bin/bash

GRADLE := ./gradlew
KMP_CLI_DIR := ../../bindings/examples/cli/langs/kotlin-multiplatform

# Standard env defaults — see plan + README. Reusing MASTER_SECRET across
# sessions keeps treasurer + senders funded between runs. MYSQL_URL points
# at the container `mysql-up` brings up.
export MASTER_SECRET ?= breez-bench
export MYSQL_URL ?= mysql://root:password@127.0.0.1:3306/breez_bench

.PHONY: help setup mysql-up build smoke-test run-server fund seed-senders mint-invoices loadgen sweep aggregate audit run trace-sync clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

setup: ## Build and publish KMP bindings (with the bench's `span-trace` cargo feature) to mavenLocal (one-time per branch)
	cd $(KMP_CLI_DIR) && $(MAKE) setup CARGO_FEATURES=span-trace

# Server-side `max_connections` for the bench container. The mysql:8.0 default
# is 151, which the bench saturates as concurrency rises (`HY000: Too many
# connections` once same-userId requests no longer serialize). The mysql:8.0
# entrypoint forwards trailing args to mysqld, so we pass `--max-connections`
# on container start. The bench's client-side pool (`MYSQL_MAX_POOL`,
# sweep.sh env) must stay under this so the server is never the limiter.
MYSQL_SERVER_MAX_CONNECTIONS ?= 500

mysql-up: ## Start the bench MySQL container + create the bench DB (idempotent)
	@if ! docker ps --format '{{.Names}}' | grep -q '^bench-mysql$$'; then \
	  echo "Starting bench-mysql container (max_connections=$(MYSQL_SERVER_MAX_CONNECTIONS))…"; \
	  docker run --rm -d --name bench-mysql -p 3306:3306 \
	    -e MYSQL_ROOT_PASSWORD=password mysql:8.0 \
	    --max-connections=$(MYSQL_SERVER_MAX_CONNECTIONS) >/dev/null; \
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

mint-invoices: build ## Pre-mint bolt11 invoice pool for LN sends (COUNT + OUT required; AMOUNT_SATS defaults to 1)
	@test -n "$$COUNT" || (echo "COUNT is required (number of invoices to mint)"; exit 2)
	@test -n "$$OUT"   || (echo "OUT is required (pool file path)"; exit 2)
	$(GRADLE) run --console=plain --args="--mode=mint-invoices --mysql-url=$$MYSQL_URL --master-secret=$$MASTER_SECRET --count=$$COUNT --amount-sats=$${AMOUNT_SATS:-1} --banks=$${BANKS:-50} --expiry-secs=$${EXPIRY_SECS:-604800} --parallelism=$${PARALLELISM:-20} --out=$$OUT"

loadgen: build ## Run the load generator against an already-running server (TARGET_RPS + DURATION required; INVOICE_POOL needed if MIX has send_ln)
	$(GRADLE) run --console=plain --args="--mode=loadgen --base-url=$${BASE_URL:-http://localhost:8080} --target-rps=$$TARGET_RPS --duration=$$DURATION --users=$${USERS:-10000} --mix=$${MIX:-info=40,receive=30,send=30} --user-distribution=$${DIST:-uniform} --senders=$${SENDERS:-50} --payment-sats=$${PAYMENT_SATS:-1}$${RUN_ID:+ --run-id=$$RUN_ID}$${INVOICE_POOL:+ --invoice-pool=$$INVOICE_POOL}$${TREASURER_SPARK_ADDR:+ --treasurer-spark-addr=$$TREASURER_SPARK_ADDR}"

sweep: build ## RPS sweep only (no aggregation). See scripts/sweep.sh for env knobs.
	./scripts/sweep.sh

aggregate: ## Aggregate a sweep into summary.json + RESULTS.md (SWEEP_ID required)
	@test -n "$$SWEEP_ID" || (echo "SWEEP_ID is required (e.g. SWEEP_ID=2026-05-08T15-00-00)"; exit 2)
	python3 scripts/aggregate.py --sweep-dir out/$$SWEEP_ID

audit: build ## Run the bolt11 settlement audit on a completed sweep (SWEEP_ID required). Auto-run by `make run` when MIX has send_ln.
	@test -n "$$SWEEP_ID" || (echo "SWEEP_ID is required"; exit 2)
	$(GRADLE) run --console=plain --args="--mode=audit-bolt11 --mysql-url=$$MYSQL_URL --master-secret=$$MASTER_SECRET --sweep-dir=out/$$SWEEP_ID --parallelism=$${AUDIT_PARALLELISM:-5}"

trace-sync: build ## Trace an explicit syncWallet() on a wallet with full Rust tracing (USER_ID defaults to __treasurer__; LOG_FILTER=debug)
	$(GRADLE) run --console=plain --args="--mode=trace-sync --mysql-url=$$MYSQL_URL --master-secret=$$MASTER_SECRET --user-id=$${USER_ID:-__treasurer__} --log-filter=$${LOG_FILTER:-debug}"

# Output naming convention (see README "Output naming"):
#   out/<sweep-id>/   where  <sweep-id> = <UTC-ISO-8601>[-<label>]
# e.g.  out/2026-05-20T11-45-00Z/
#       out/2026-05-20T11-45-00Z-mysql-recycle/   (LABEL=mysql-recycle)
# All sweep artifacts — including the live driver log — live inside
# this dir. Do NOT redirect `make run` to a sibling .log file; the
# recipe tees its own combined stdout/stderr into driver.log for you.
run: build ## Sweep + aggregate. Live output tee'd into out/<sweep-id>/driver.log. Use LABEL=<slug> for a labeled run.
	@SWEEP_ID=$${SWEEP_ID:-$$(date -u +%Y-%m-%dT%H-%M-%SZ)$${LABEL:+-$$LABEL}}; \
	  export SWEEP_ID; \
	  : "$${SWEEP_RPS:=50,100,250}"; export SWEEP_RPS; \
	  : "$${DURATION:=2m}"; export DURATION; \
	  mkdir -p out/$$SWEEP_ID; \
	  driver_log=out/$$SWEEP_ID/driver.log; \
	  echo "[run] sweep-id=$$SWEEP_ID  driver-log=$$driver_log"; \
	  { ./scripts/sweep.sh && \
	    python3 scripts/aggregate.py --sweep-dir out/$$SWEEP_ID ; } 2>&1 | tee "$$driver_log"; \
	  rc=$${PIPESTATUS[0]}; \
	  if [ $$rc -eq 0 ]; then echo && echo "Done. See out/$$SWEEP_ID/RESULTS.md"; else echo "Failed (rc=$$rc). See $$driver_log"; fi; \
	  exit $$rc

clean: ## Remove build artifacts
	$(GRADLE) clean
