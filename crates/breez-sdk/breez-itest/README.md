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
| `RECOVERY_TEST_MNEMONIC` | BIP-39 mnemonic for recovery testing | None |
| `RECOVERY_TEST_EXPECTED_PAYMENTS` | JSON spec of expected payments | None |

## Wallet Recovery Tests

The `recovery.rs` test file contains tests for wallet recovery from mnemonic. These tests verify
that a wallet can be restored from its seed phrase and correctly sync all historical payments.

### Setup Test (One-time)

The setup test creates a new wallet with all payment variants and outputs the credentials needed
for the recovery test. Run it manually:

```bash
cargo test -p breez-sdk-itest test_setup_recovery_wallet -- --ignored --nocapture
```

This will output:
1. A 12-word mnemonic (for `RECOVERY_TEST_MNEMONIC`)
2. A JSON spec of expected payments (for `RECOVERY_TEST_EXPECTED_PAYMENTS`)

**Payment variants created:**
- Deposit (receive via faucet)
- Spark send (regular)
- Spark receive (regular)
- Spark send (HTLC)
- Spark receive (HTLC)
- Lightning send (BOLT11)
- Lightning receive (BOLT11)
- Token send
- Token receive
- Withdraw (on-chain)

### Recovery Test

The recovery test loads a wallet from mnemonic and verifies all payments match the expected spec.
It runs automatically in CI when the environment variables are set, or skips gracefully if not.

```bash
# Run with credentials
RECOVERY_TEST_MNEMONIC="your mnemonic here" \
RECOVERY_TEST_EXPECTED_PAYMENTS='{"min_balance_sats":50000,"payments":[...]}' \
cargo test -p breez-sdk-itest test_wallet_recovery_from_mnemonic -- --nocapture
```

### Setting Up GitHub Secrets

After running the setup test, add the following secrets to your GitHub repository:

1. `RECOVERY_TEST_MNEMONIC` - The 12-word mnemonic output by the setup test
2. `RECOVERY_TEST_EXPECTED_PAYMENTS` - The JSON output by the setup test (single line)

### Regenerating the Test Wallet

If new payment test cases need to be added, you can regenerate the test wallet by running the setup test again.

1. Run the setup test again to create a new wallet
2. Update both GitHub secrets with the new values
