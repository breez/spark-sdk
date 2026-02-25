# Breez SDK CLI — Dart

A Dart port of the [Rust CLI](../../../cli/) for the Breez SDK with Spark.

This CLI uses the [breez_sdk_spark_flutter](../../../../packages/flutter) package (Flutter/Dart bindings via `flutter_rust_bridge`).

## Prerequisites

- **Dart** >= 3.7
- **Flutter** >= 3.27 (needed for `flutter pub get`)

## Quick Start

### Using published SDK (recommended)

```bash
make setup-published   # flutter pub get
make run               # dart run bin/breez_cli.dart (regtest)
```

### Using local bindings

```bash
make setup             # flutter pub get (uses local path dependency)
make run               # dart run bin/breez_cli.dart (regtest)
```

### Running on mainnet

```bash
export BREEZ_API_KEY="<your api key>"
make run-mainnet
```

## CLI Options

```
-d, --data-dir          Path to the data directory (default: ./.data)
    --network           Network to use: regtest, mainnet (default: regtest)
    --account-number    Account number for the Spark signer
-h, --help              Show usage
```

## Commands

Once the CLI is running, type `help` to see all available commands:

- `get-info` — Get balance information
- `get-payment` — Get payment by ID
- `sync` — Sync wallet state
- `list-payments` — List payments (with filters)
- `receive` — Receive a payment (spark address, spark invoice, bitcoin, bolt11)
- `pay` — Send a payment
- `lnurl-pay` — Pay via LNURL or Lightning address
- `lnurl-withdraw` — Withdraw via LNURL
- `lnurl-auth` — Authenticate via LNURL
- `claim-htlc-payment` — Claim an HTLC payment with preimage
- `claim-deposit` — Claim an on-chain deposit
- `parse` — Parse any input (invoice, address, LNURL)
- `refund-deposit` — Refund an on-chain deposit
- `list-unclaimed-deposits` — List unclaimed deposits
- `buy-bitcoin` — Get MoonPay URL to buy Bitcoin
- `check-lightning-address-available` — Check username availability
- `get-lightning-address` — Get registered lightning address
- `register-lightning-address` — Register a lightning address
- `delete-lightning-address` — Delete lightning address
- `list-fiat-currencies` — List fiat currencies
- `list-fiat-rates` — List fiat exchange rates
- `recommended-fees` — Get recommended BTC fees
- `get-tokens-metadata` — Get token metadata
- `fetch-conversion-limits` — Fetch conversion limits
- `get-user-settings` — Get user settings
- `set-user-settings` — Update user settings
- `get-spark-status` — Get Spark network status
- `issuer <subcommand>` — Token issuer commands

## Dart/FRB-Specific Notes

This CLI uses `flutter_rust_bridge` (FRB) bindings, which differ from UniFFI:

- **Init required**: `await BreezSdkSparkLib.init()` must be called before any SDK usage
- **camelCase methods**: `getInfo`, `prepareSendPayment`, etc.
- **Sealed class enums**: Pattern matching with `case SdkEvent_Synced():` or `is` checks
- **Unnamed fields**: Accessed via `.field0` (e.g., `InputType_LnurlPay.field0`)
- **BigInt**: Dart native `BigInt` for large integers (amounts, timestamps)
- **Events**: `Stream<SdkEvent>` from `sdk.addEventListener()`, not callbacks
- **Immutable config**: Use `.copyWith()` to modify config values
