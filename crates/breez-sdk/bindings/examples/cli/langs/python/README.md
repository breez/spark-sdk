# Breez SDK - Spark CLI (Python)

Interactive CLI client for the [Breez SDK](../../../../../../../README.md) with Spark, written in Python.

> **Note:** The [Rust CLI](../../../../../cli/) is the source of truth. This CLI is an automated port that mirrors its commands, arguments, and behavior. Changes should be made to the Rust CLI first — a [sync workflow](../../../../../../../.github/workflows/sync-python-cli.yml) will open a PR to update this CLI automatically.

## Prerequisites

- Python >= 3.9

## Quick Start

```bash
# 1. Build and install local Python bindings + CLI
make setup

# 2. Set API key (required for mainnet)
export BREEZ_API_KEY="<your-api-key>"

# 3. Run (regtest by default, no API key needed)
make run
```

## Using Published SDK

To use the published SDK from PyPI instead of local bindings:

```bash
make setup-published
make run
```

## Makefile Targets

```
make setup            Build and install local Python bindings + CLI
make setup-published  Install published SDK + CLI from PyPI
make run              Run CLI on regtest (default)
make run-mainnet      Run CLI on mainnet
make clean            Remove venv and build artifacts
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

Each command supports `--help` for detailed usage, e.g. `receive --help`.

## Development

For development with live code reloading, the `make setup` target already installs in editable mode (`pip install -e .`). Changes to source files take effect immediately without reinstalling.

To install manually without make:

```bash
python3 -m venv .venv
source .venv/bin/activate
pip install -e .
breez-cli --help
```
