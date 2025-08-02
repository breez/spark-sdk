# Spark SDK Integration Tests

This directory contains the integration tests for the Spark SDK. These tests run against a complete system setup, including Docker containers for bitcoind, Postgres databases, and Spark Service Operators (SOs).

## Prerequisites

- **Docker**: Must be installed and running
- **Internet connection**: Only needed for pulling Docker images during the first run

## Running the Tests

To run all integration tests:

```bash
make itest
```

This command will:
1. Build all required Docker containers (if not already built)
2. Start the necessary containers for each test
3. Run the test suite

## Available Test Fixtures

The integration tests use several fixture components that set up the testing environment:

1. **BitcoindFixture**: A Bitcoin Core node running in regtest mode
2. **PostgresFixture**: PostgreSQL databases for the Spark Service Operators
3. **SparkSoFixture**: Multiple Spark Service Operators that work together using Threshold Signatures
4. **WaitForLogConsumer**: Utility for waiting for specific log patterns in container outputs

## Keeping Fixtures Alive

**IMPORTANT**: You must keep the fixtures alive during the entire test execution. If a fixture is dropped, the associated containers will be stopped and removed, which will cause your test to fail.

```rust
// ❌ WRONG: Fixture will be dropped at the end of this block
{
    let bitcoind = BitcoindFixture::new().await?;
    let wallet = create_wallet(&bitcoind).await?;
} // bitcoind is dropped here, containers are stopped!
// Using wallet here will fail!

// ✅ CORRECT: Keep the fixture until the end of the test
let bitcoind = BitcoindFixture::new().await?;
let wallet = create_wallet(&bitcoind).await?;
// Use wallet...
// bitcoind is kept alive
```

## Debugging Tests

To view detailed logs during test execution:

```bash
RUST_LOG=spark_wallet=trace,spark=trace cargo test -- --nocapture
```

This will show all container logs and test output, which is useful for diagnosing issues.
