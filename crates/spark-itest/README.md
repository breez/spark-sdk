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
1. Build the images a cluster runs, skipping those already built. Each is tagged
   by what it is built from, so worktrees never run one another's images.
2. Build or check the state snapshot every test restores
3. Run the test suite

To split a run across machines, run one group of suites: `make itest
GROUP=lightning`. The groups are unilateral-exit, timelocks, exits, lightning,
tokens and wallets. The suites that reach the deployed regtest rather than a local cluster
run separately, with `make deployed-itest`.

## Available Test Fixtures

The integration tests use several fixture components that set up the testing environment:

1. **BitcoindFixture**: A Bitcoin Core node running in regtest mode
2. **DatabaseFixture**: One PostgreSQL server holding a database per operator and
   one for the daemon, started from an image the state snapshot is restored into
3. **SparkSoFixture**: Multiple Spark Service Operators that work together using Threshold Signatures
4. **SspdFixture**: The service provider daemon the wallets pay through
5. **WaitForLogConsumer**: Utility for waiting for specific log patterns in container outputs

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

By default, Docker container stdout logs are suppressed to reduce noise during test execution. **Errors (stderr) are always logged**. To enable verbose logging of all output from Docker containers (bitcoind, operators, migrations), set the `SPARK_ITEST_VERBOSE` environment variable:

```bash
SPARK_ITEST_VERBOSE=1 cargo test -- --nocapture
```

To view detailed logs from the Spark SDK itself:

```bash
RUST_LOG=spark_wallet=trace,spark=trace cargo test -- --nocapture
```

To see everything (SDK + Docker containers):

```bash
SPARK_ITEST_VERBOSE=1 RUST_LOG=spark_wallet=trace,spark=trace cargo test -- --nocapture
```

This will show all container logs and test output, which is useful for diagnosing issues.
