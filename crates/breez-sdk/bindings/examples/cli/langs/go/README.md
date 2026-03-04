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

## Development

```bash
# Build and run directly
go build -o breez-cli . && ./breez-cli --help

# Run without building a binary
go run . --network regtest
```
