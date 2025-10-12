# Breez SDK Integration Tests

This crate contains integration tests for the Breez SDK running against remote Lightspark regtest infrastructure.
These tests verify end-to-end SDK functionality.

## Running Tests

### Running All Tests

```bash
# Run all tests
cargo test -p breez-sdk-itest -- --nocapture

# With debug logging
RUST_LOG=debug cargo test -p breez-sdk-itest -- --nocapture
```

### Running Specific Tests

```bash
# Run only the Spark transfer test
cargo test -p breez-sdk-itest test_01_spark_transfer -- --nocapture --test-threads=1

# Run deposit claim test
cargo test -p breez-sdk-itest test_02_deposit_claim -- --nocapture --test-threads=1
```

## Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `FAUCET_URL` | URL of the regtest faucet GraphQL endpoint | `https://app.lightspark.com/graphql/spark/rc` |
| `FAUCET_USERNAME` | Username for basic authentication | None |
| `FAUCET_PASSWORD` | Password for basic authentication | None |
| `RUST_LOG` | Logging level (trace, debug, info, warn, error) | `info` |
