# Breez SDK Integration Tests

This crate contains integration tests for the Breez SDK running against remote Lightspark regtest infrastructure.

## Overview

These tests verify end-to-end SDK functionality including:
- Deposit detection and auto-claiming
- Spark transfers between wallets
- Lightning invoice payments
- Balance tracking and synchronization

## Running Tests

### Prerequisites

1. **Access to Lightspark regtest infrastructure**:
   - Spark operators running at `https://0.spark.lightspark.com`, etc.
   - Lightspark API available at `https://api.lightspark.com`
   - Faucet GraphQL endpoint at `https://app.lightspark.com/graphql/spark/rc`

2. **No additional dependencies required**:
   - Tests use the Lightspark GraphQL faucet API directly
   - Fully automated - no manual interaction needed!

### Running All Tests

```bash
# From workspace root
cargo test -p breez-sdk-itest

# With trace logging
RUST_LOG=debug cargo test -p breez-sdk-itest -- --nocapture
```

### Running Specific Tests

```bash
# Run only deposit test
cargo test -p breez-sdk-itest test_breez_sdk_deposit_claim

# Run only payment test
cargo test -p breez-sdk-itest test_breez_sdk_send_payment_prefer_spark
```

## Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `FAUCET_URL` | URL of the regtest faucet GraphQL endpoint | `https://app.lightspark.com/graphql/spark/rc` |
| `FAUCET_USERNAME` | Username for basic authentication | None |
| `FAUCET_PASSWORD` | Password for basic authentication | None |
| `RUST_LOG` | Logging level (trace, debug, info, warn, error) | `info` |

### Example Configuration

The default configuration should work out of the box. If you need to customize:

```bash
# Optional: Override the faucet endpoint
export FAUCET_URL="https://custom-faucet.example.com/graphql"

# Optional: Set basic authentication credentials
export FAUCET_USERNAME="your_username"
export FAUCET_PASSWORD="your_password"

# Optional: Enable debug logging
export RUST_LOG=debug

# Run tests
cargo test -p breez-sdk-itest -- --nocapture
```

## Test Architecture

### Modules

- **`faucet.rs`**: Automated regtest faucet client
  - Supports multiple endpoint formats (REST, GraphQL)
  - Environment variable configuration
  - Automatic retries and fallbacks

- **`helpers.rs`**: Reusable test utilities
  - `build_sdk()`: Initialize SDK instance for testing
  - `wait_for_balance()`: Poll wallet balance until threshold met
  - `fund_address_and_wait()`: Fund address and wait for confirmation
  - `receive_and_fund()`: Generate deposit address and fund in one step

- **`tests/breez_sdk_tests.rs`**: Integration test cases
  - Clean, focused test logic using helper functions
  - Comprehensive logging for debugging
  - Proper resource cleanup

### Test Flow

1. **Setup**: Create temporary storage directory and initialize SDK
2. **Fund**: Generate deposit address and fund via automated faucet
3. **Wait**: Poll for balance update (SDK auto-claims deposits)
4. **Execute**: Perform test operations (transfers, payments, etc.)
5. **Verify**: Assert expected outcomes
6. **Cleanup**: Temporary directories are automatically cleaned up

## Faucet Implementation

The faucet client uses the Lightspark GraphQL API to fund addresses automatically:

- **Endpoint**: `https://app.lightspark.com/graphql/spark/rc`
- **Method**: GraphQL query `PolarityFaucet`
- **No manual interaction required** - fully automated!

The implementation:
1. Constructs a GraphQL request with the address and amount
2. Sends POST request to the faucet endpoint
3. Parses the response to extract the transaction hash
4. Returns the txid for tracking

## Troubleshooting

### Faucet Request Fails

If faucet funding fails with an error:

```
Faucet request failed with status 400: ...
```

**Possible causes**:
- Invalid Bitcoin address format
- Faucet is rate-limited or temporarily unavailable
- Network connectivity issues
- API endpoint has changed

**Solutions**:
1. Check the address is valid regtest format (starts with `bcrt1`)
2. Verify network connectivity: `curl https://app.lightspark.com/graphql/spark/rc`
3. Check if you're being rate-limited (wait a few minutes)
4. Run with `RUST_LOG=debug` to see detailed request/response
5. Verify the faucet endpoint is still correct

### Timeout Waiting for Balance

If tests timeout waiting for balance:

```
Timeout waiting for balance >= 100000 sats after 180 seconds
```

**Possible causes**:
- Faucet transaction didn't propagate
- SDK sync interval is too slow
- Spark operators are down
- Network connectivity issues

**Solutions**:
- Check Lightspark infrastructure status
- Increase timeout in test code
- Verify transaction on regtest explorer
- Check SDK logs for errors

### Connection Errors

If you see connection errors to Spark operators:

```
Error: Failed to connect to operator at https://0.spark.lightspark.com
```

**Solutions**:
- Verify network connectivity
- Check if Lightspark regtest infrastructure is accessible
- Ensure no firewall/proxy blocking requests
- Try from a different network

## Adding New Tests

To add a new integration test:

1. Create a new test function in `tests/breez_sdk_tests.rs`:

```rust
#[rstest]
#[test_log::test(tokio::test)]
async fn test_my_new_feature() -> Result<()> {
    info!("=== Starting test_my_new_feature ===");

    // Setup
    let data_dir = tempdir::TempDir::new("my-test")?;
    let sdk = build_sdk(data_dir.path().to_string_lossy().to_string(), [4u8; 32]).await?;

    // Fund if needed
    let (_addr, _txid) = receive_and_fund(&sdk, 50_000).await?;

    // Test logic here

    info!("=== Test test_my_new_feature PASSED ===");
    Ok(())
}
```

2. Use helper functions from `breez_sdk_itest::*` for common operations
3. Add comprehensive logging for debugging
4. Clean up resources (handled automatically with `TempDir`)

## Performance

- Test runtime: ~3-5 minutes per test (mostly waiting for faucet and confirmations)
- Can be parallelized: Tests use independent storage directories
- Background SDK sync: 5-second interval for faster test execution

## CI/CD Integration

For automated CI/CD pipelines:

1. Ensure `FAUCET_URL` and `FAUCET_API_KEY` are set as CI secrets
2. Run tests with timeout to prevent hanging:

```bash
timeout 600 cargo test -p breez-sdk-itest --  --test-threads=1
```

3. Use `--test-threads=1` to avoid parallel test conflicts if needed
4. Capture logs: `RUST_LOG=debug cargo test -p breez-sdk-itest -- --nocapture > test.log 2>&1`
