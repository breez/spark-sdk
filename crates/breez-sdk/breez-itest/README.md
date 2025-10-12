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
   - Faucet GraphQL endpoint at `https://api.lightspark.com/graphql/spark/rc`

2. **No additional dependencies required**:
   - Tests use the Lightspark GraphQL faucet API directly
   - Fully automated - no manual interaction needed!

### Test Architecture

Tests use **persistent storage** with shared fixtures:
- Alice and Bob SDKs persist in `target/breez-itest-workspace/`
- First test (`test_01_fund_alice`) funds Alice with 100k sats
- Subsequent tests reuse Alice's balance, saving time and faucet requests
- Tests are numbered (01, 02, 03...) to suggest execution order

### Running All Tests

**Recommended: Run tests sequentially** to ensure proper ordering and avoid database contention:

```bash
# Sequential execution (recommended)
cargo test -p breez-sdk-itest -- --test-threads=1 --nocapture

# With debug logging
RUST_LOG=debug cargo test -p breez-sdk-itest -- --test-threads=1 --nocapture
```

**Parallel execution** (not recommended, may cause issues):
```bash
# Parallel mode (may fail due to shared storage)
cargo test -p breez-sdk-itest
```

### Running Specific Tests

```bash
# Run only the funding test
cargo test -p breez-sdk-itest test_01_fund_alice -- --nocapture

# Run only the Spark transfer test
cargo test -p breez-sdk-itest test_02_spark_transfer -- --nocapture

# Run deposit claim test
cargo test -p breez-sdk-itest test_03_deposit_claim -- --nocapture
```

### Cleaning Test State

If you need to start fresh (e.g., after SDK changes or test failures):

```bash
# Remove persistent test workspace
rm -rf target/breez-itest-workspace

# Next test run will recreate everything from scratch
cargo test -p breez-sdk-itest -- --test-threads=1 --nocapture
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

## Test Design Details

### Modules

- **`faucet.rs`**: Automated regtest faucet client
  - GraphQL-based API integration with Lightspark faucet
  - Basic authentication support via environment variables
  - Automatic transaction submission and tracking

- **`helpers.rs`**: Reusable test utilities
  - `build_sdk()`: Initialize SDK instance for testing
  - `wait_for_balance()`: Poll wallet balance until threshold met
  - `fund_address_and_wait()`: Fund address and wait for confirmation
  - `receive_and_fund()`: Generate deposit address and fund in one step

- **`tests/breez_sdk_tests.rs`**: Integration test cases
  - **Fixtures**: `alice_sdk`, `bob_sdk` with persistent storage
  - **Shared state**: Alice and Bob persist across test runs
  - **Smart funding**: `ensure_funded()` only funds when balance is low
  - Comprehensive logging for debugging

### Test Flow

1. **First Run** (test_01_fund_alice):
   - Initialize Alice and Bob SDKs with persistent storage
   - Fund Alice with 100k sats via faucet
   - Subsequent tests reuse this balance

2. **Subsequent Tests**:
   - Reuse existing Alice/Bob SDKs from `target/breez-itest-workspace/`
   - Check Alice's balance, fund only if needed
   - Execute test operations (transfers, payments, etc.)
   - Verify expected outcomes

3. **Benefits**:
   - **Fast**: Only 1 faucet request instead of N
   - **Reliable**: Tests work offline after initial funding
   - **Debuggable**: Persistent state helps reproduce issues

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

1. **Use the fixture-based pattern**:

```rust
/// Test 4: Your new feature test
#[rstest]
#[test_log::test(tokio::test)]
async fn test_04_my_new_feature(
    alice_sdk: Result<BreezSdk>,
    bob_sdk: Result<BreezSdk>,
) -> Result<()> {
    info!("=== Starting test_04_my_new_feature ===");

    let alice = alice_sdk?;
    let bob = bob_sdk?;

    // Ensure Alice is funded if needed
    ensure_funded(&alice, 50_000).await?;

    // Your test logic here...

    info!("=== Test test_04_my_new_feature PASSED ===");
    Ok(())
}
```

2. **Best practices**:
   - Number tests (04, 05, 06...) to suggest execution order
   - Use `alice_sdk` and `bob_sdk` fixtures instead of creating new SDKs
   - Call `ensure_funded()` if you need Alice to have a minimum balance
   - Use helper functions from `breez_sdk_itest::*` for common operations
   - Add comprehensive logging for debugging
   - Don't manually fund unless testing the funding flow itself

3. **When to create new SDK instances**:
   - Only create new temporary SDKs if testing initialization/setup logic
   - For most tests, reuse Alice/Bob fixtures for consistency and speed

## Performance

- **First run**: ~3-5 minutes (initial funding via faucet)
- **Subsequent runs**: ~30-60 seconds per test (no funding needed)
- **Sequential execution**: Required due to shared storage (use `--test-threads=1`)
- **Background SDK sync**: 5-second interval for faster test execution
- **Storage**: Persistent workspace in `target/breez-itest-workspace/`

## CI/CD Integration

For automated CI/CD pipelines:

1. **Set authentication secrets**:
   ```bash
   export FAUCET_USERNAME="your_ci_username"
   export FAUCET_PASSWORD="your_ci_password"
   ```

2. **Clean workspace before tests** (recommended for CI):
   ```bash
   rm -rf target/breez-itest-workspace
   ```

3. **Run tests sequentially with timeout**:
   ```bash
   timeout 600 cargo test -p breez-sdk-itest -- --test-threads=1 --nocapture
   ```

4. **Capture detailed logs**:
   ```bash
   RUST_LOG=debug cargo test -p breez-sdk-itest -- --test-threads=1 --nocapture > test.log 2>&1
   ```

5. **Cache workspace** (optional, for faster CI runs):
   - Cache `target/breez-itest-workspace/` between CI runs
   - First run: funds Alice (~3-5 min)
   - Cached runs: reuse balance (~30-60 sec per test)
