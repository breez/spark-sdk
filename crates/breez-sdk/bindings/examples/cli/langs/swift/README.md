# Breez SDK - Spark CLI (Swift)

Interactive CLI client for the [Breez SDK](../../../../../../../README.md) with Spark, written in Swift.

> **Note:** The [Rust CLI](../../../../../cli/) is the source of truth. This CLI is an automated port that mirrors its commands, arguments, and behavior. Changes should be made to the Rust CLI first — a sync workflow will open a PR to update this CLI automatically.

## Prerequisites

- Swift >= 5.9
- macOS 15.0+
- Rust toolchain (for local bindings only)

## Quick Start

```bash
# 1. Set API key (required for mainnet)
export BREEZ_API_KEY="<your-api-key>"

# 2. Build local bindings from Rust source and resolve dependencies
make setup

# 3. Build and run (regtest by default, no API key needed)
make run
```

## Using Local Bindings (from Rust source)

`make setup` builds the Rust FFI library, generates Swift bindings, packages them into the local xcframework, and switches `Package.swift` to use the local path dependency — all in one step.

## Using Published SDK

To switch back to the published Swift SDK:

```bash
make setup-published
```

## Makefile Targets

```
make setup            Build local Swift bindings from Rust source
make setup-published  Download dependencies from published SDK
make build            Build the CLI binary
make run              Build + run on regtest (default)
make run-mainnet      Build + run on mainnet
make clean            Remove build artifacts
```

## CLI Options

| Option | Default | Description |
|--------|---------|-------------|
| `-d`, `--data-dir` | `./.data` | Path to the data directory |
| `--network` | `regtest` | Network to use (`regtest` or `mainnet`) |
| `--account-number` | - | Account number for the Spark signer |
| `--postgres-connection-string` | - | PostgreSQL connection string (uses SQLite by default) |
| `--stable-balance-token-identifier` | - | Stable balance token identifier |
| `--stable-balance-threshold` | - | Stable balance threshold in sats |
| `--passkey` | - | Use Passkey with PRF provider (`file`, `yubikey` or `fido2`) |
| `--label` | `Default` | Requires `--passkey`. The label to use |
| `--list-labels` | false | Requires `--passkey`. Select label from Nostr |
| `--store-label` | false | Requires `--passkey`. Publish label to Nostr |
| `--rpid` | `keys.breez.technology` | Requires `--passkey`. Relying party ID for FIDO2 provider |

## Environment Variables

| Variable | Description |
|----------|-------------|
| `BREEZ_API_KEY` | Breez API key (required for mainnet, get one at [breez.technology](https://breez.technology/request-api-key/)) |

## Available Commands

Once inside the REPL, type `help` to see all commands. The REPL supports **command history** (arrow keys, persisted across sessions) and **tab completion** for command names.

The CLI supports:

**Wallet**: `get-info`, `sync`, `get-payment`, `list-payments`

**Payments**: `receive`, `pay`, `lnurl-pay`, `lnurl-withdraw`, `lnurl-auth`, `claim-htlc-payment`

**On-chain**: `claim-deposit`, `refund-deposit`, `list-unclaimed-deposits`, `buy-bitcoin`

**Lightning address**: `get-lightning-address`, `register-lightning-address`, `delete-lightning-address`, `check-lightning-address-available`

**Tokens**: `get-tokens-metadata`, `fetch-conversion-limits`, `issuer <subcommand>`

**Contacts**: `contacts add`, `contacts update`, `contacts delete`, `contacts list`

**Other**: `parse`, `list-fiat-currencies`, `list-fiat-rates`, `get-user-settings`, `set-user-settings`, `get-spark-status`, `recommended-fees`

## Passkey

Using a passkey enables a deterministic seed to be derived without storing a mnemonic on disk. Instead, a file-based secret (or hardware key) is used to deterministically derive wallet seeds via HMAC challenge-response.

Labels are stored on Nostr relays, allowing discovery during restore. If no `--label` is specified, the default label ("Default") is used.

### PRF Providers

#### File Provider

Uses a random 32-byte secret stored in `<data-dir>/seedless-restore-secret`. The secret is generated on first use. Suitable for development and testing.

```bash
# Use passkey with the default label
make run ARGS="--passkey file"

# Use passkey with a specific label
make run ARGS="--passkey file --label personal"

# Use passkey after selecting a label published to Nostr
make run ARGS="--passkey file --list-labels"

# Use passkey with a specific label and publish the label to Nostr
make run ARGS="--passkey file --label personal --store-label"
```

#### YubiKey Provider

Not yet available in the Swift CLI. See the [Rust CLI](../../../../../cli/README.md) for the reference implementation.

#### FIDO2 Provider

Not yet available in the Swift CLI. See the [Rust CLI](../../../../../cli/README.md) for the reference implementation.

## Development

```bash
# Build and run directly
swift build && swift run breez-cli

# Run with specific network
swift run breez-cli --network mainnet
```
