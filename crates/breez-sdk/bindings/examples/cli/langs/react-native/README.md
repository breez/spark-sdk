# Breez SDK CLI - React Native

A terminal/REPL-like React Native app that mirrors the same command structure as the Rust CLI.

Since React Native requires a mobile runtime (iOS/Android), this CLI is implemented as an app
with a text input at the bottom for typing commands and a scrollable log output area showing results.

## Prerequisites

- Node.js >= 22
- React Native development environment ([setup guide](https://reactnative.dev/docs/set-up-your-environment))
- For iOS: Xcode and CocoaPods
- For Android: Android Studio and Android SDK

## Setup

```bash
make setup
```

## Usage

```bash
# Run on iOS simulator
make run-ios

# Run on Android emulator
make run-android
```

## Environment Variables

| Variable | Description |
|----------|-------------|
| `BREEZ_API_KEY` | Breez API key (required for mainnet) |

The API key can be set within the app settings or hardcoded in the config.

## Passkey Support

The CLI supports passkey-based seed derivation, matching the Rust CLI's `--passkey` flag.
Three PRF providers are available:

| Provider | Description |
|----------|-------------|
| `file` | File-based HMAC-SHA256 provider (for development/testing) |
| `yubikey` | YubiKey hardware key (stub - not yet implemented) |
| `fido2` | FIDO2/WebAuthn PRF (stub - not yet implemented) |

### Configuration

To enable passkey support, edit the `PASSKEY_CONFIG` constant in `src/App.tsx`:

```typescript
const PASSKEY_CONFIG: PasskeyConfig = {
  provider: PasskeyProvider.File,
  walletName: 'personal',        // Optional wallet name
  listWalletNames: false,        // Query Nostr for wallet names
  storeWalletName: false,        // Publish wallet name to Nostr
}
```

When `PASSKEY_CONFIG` is `undefined` (the default), the CLI uses a mnemonic-based seed
stored in the app's data directory, matching the default behavior of the Rust CLI.

### Wallet Name Management

When using passkey with Nostr relay support:

- **Store**: Set `storeWalletName: true` and `walletName: '<name>'` to publish a wallet name to Nostr
- **List**: Set `listWalletNames: true` to query Nostr for wallet names associated with the passkey
- **Select**: When listing, the available names are displayed in the log output

## Available Commands

Once the app is running, type commands in the text input at the bottom:

**Wallet**: `get-info`, `sync`, `get-payment`, `list-payments`, `recommended-fees`

**Payments**: `receive`, `pay`, `lnurl-pay`, `lnurl-withdraw`, `lnurl-auth`, `claim-htlc-payment`

**On-chain**: `claim-deposit`, `refund-deposit`, `list-unclaimed-deposits`, `buy-bitcoin`

**Lightning address**: `get-lightning-address`, `register-lightning-address`, `delete-lightning-address`, `check-lightning-address-available`

**Tokens**: `get-tokens-metadata`, `fetch-conversion-limits`, `issuer <subcommand>`

**Contacts**: `contacts <subcommand>`

**Other**: `parse`, `list-fiat-currencies`, `list-fiat-rates`, `get-user-settings`, `set-user-settings`, `get-spark-status`

Type `help` for a full list of commands. Each command mirrors the Rust CLI behavior.

### HODL Invoices

Create a HODL (hold) invoice using the `--hodl` flag with bolt11:

```
receive -m bolt11 -a 1000 --hodl
```

This generates a preimage and payment hash. Use `claim-htlc-payment <preimage>` to settle.

### HTLC Transfers

Send a payment as an HTLC transfer (Spark address, Bitcoin only):

```
pay -r <spark_address> -a 1000 --htlc-payment-hash <hex> --htlc-expiry-secs 3600
```

### Payment Filtering

The `list-payments` command supports rich filtering:

```
list-payments --type-filter send,receive --status-filter completed --limit 20
list-payments --asset-filter bitcoin
list-payments --spark-htlc-status-filter pending
list-payments --tx-hash <hash>
list-payments --tx-type mint
list-payments --from-timestamp 1700000000 --to-timestamp 1710000000
list-payments --sort-ascending true
```

## TypeScript Validation

```bash
make typecheck
```
