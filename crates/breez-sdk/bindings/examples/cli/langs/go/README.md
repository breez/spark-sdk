# Breez SDK - Spark CLI (Go)

Interactive CLI client for the [Breez SDK](../../../../../../../README.md) with Spark, written in Go.

> **Note:** The [Rust CLI](../../../../../cli/) is the source of truth. This CLI is an automated port that mirrors its commands, arguments, and behavior. Changes should be made to the Rust CLI first — a [sync workflow](../../../../../../../.github/workflows/sync-go-cli.yml) will open a PR to update this CLI automatically.

## Prerequisites

- Go >= 1.19

## Quick Start

```bash
# 1. Generate local Go bindings from Rust source
make setup

# 2. Uncomment the replace directive in go.mod
# replace github.com/breez/breez-sdk-spark-go => ../../../../ffi/golang

# 3. Set API key (required for mainnet)
export BREEZ_API_KEY="<your-api-key>"

# 4. Run (regtest by default, no API key needed)
make run
```

## Using Published SDK

To use the published Go SDK instead of local bindings:

```bash
# 1. Comment out the replace directive in go.mod
# 2. Download published dependencies
make setup-published
# 3. Build and run
make run
```

## Makefile Targets

```
make setup            Generate local Go bindings from Rust source
make setup-published  Download dependencies from published SDK
make build            Build the CLI binary
make run              Build + run on regtest (default)
make run-mainnet      Build + run on mainnet
make clean            Remove binary
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
| `--wallet-name` | `Default` | Requires `--passkey`. The wallet name to use |
| `--list-wallet-names` | false | Requires `--passkey`. Select wallet name from Nostr |
| `--store-wallet-name` | false | Requires `--passkey`. Publish wallet name to Nostr |
| `--rpid` | `keys.breez.technology` | Requires `--passkey`. Relying party ID for FIDO2 provider |

## Environment Variables

| Variable | Description |
|----------|-------------|
| `BREEZ_API_KEY` | Breez API key (required for mainnet, get one at [breez.technology](https://breez.technology/request-api-key/)) |

## Available Commands

Once inside the REPL, type `help` to see all commands. The CLI supports:

**Wallet**: `get-info`, `sync`, `get-payment`, `list-payments`, `recommended-fees`

**Payments**: `receive`, `pay`, `lnurl-pay`, `lnurl-withdraw`, `lnurl-auth`, `claim-htlc-payment`

**On-chain**: `claim-deposit`, `refund-deposit`, `list-unclaimed-deposits`, `buy-bitcoin`

**Lightning address**: `get-lightning-address`, `register-lightning-address`, `delete-lightning-address`, `check-lightning-address-available`

**Tokens**: `get-tokens-metadata`, `fetch-conversion-limits`, `issuer <subcommand>`

**Contacts**: `contacts add`, `contacts update`, `contacts delete`, `contacts list`

**Other**: `parse`, `list-fiat-currencies`, `list-fiat-rates`, `get-user-settings`, `set-user-settings`, `get-spark-status`

Each command supports `--help` for detailed usage.

## Passkey

Using a passkey enables a deterministic seed to be derived without storing a mnemonic on disk. Instead, a file-based secret (or hardware key) is used to deterministically derive wallet seeds via HMAC challenge-response.

Wallet names are stored on Nostr relays, allowing discovery during restore. If no `--wallet-name` is specified, the default wallet name ("Default") is used.

### PRF Providers

#### File Provider

Uses a random 32-byte secret stored in `<data-dir>/seedless-restore-secret`. The secret is generated on first use. Suitable for development and testing.

```bash
# Use passkey with the default wallet name
./breez-cli --passkey file

# Use passkey with a specific wallet name
./breez-cli --passkey file --wallet-name personal

# Use passkey after selecting a wallet name published to Nostr
./breez-cli --passkey file --list-wallet-names

# Use passkey with a specific wallet name and publish the wallet name to Nostr
./breez-cli --passkey file --wallet-name personal --store-wallet-name
```

#### YubiKey Provider

Not yet available in Go CLI. See the [Rust CLI README](../../../../../cli/README.md) for details.

#### FIDO2 Provider

Not yet available in Go CLI. See the [Rust CLI README](../../../../../cli/README.md) for details.

## Development

```bash
# Build and run directly
go build -o breez-cli . && ./breez-cli --help

# Run without building a binary
go run . --network regtest
```
