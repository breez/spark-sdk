# Breez SDK CLI

Command-line interface for testing and interacting with the Breez SDK (Spark).

## Usage

```bash
cargo run -- [OPTIONS]
```

### CLI Options

| Option | Default | Description |
|--------|---------|-------------|
| `-d`, `--data-dir` | `./.data` | Path to the data directory |
| `--network` | `regtest` | Network to use (`regtest` or `mainnet`) |
| `--account-number` | - | Account number for the Spark signer |
| `--postgres-connection-string` | - | PostgreSQL connection string (uses SQLite by default) |
| `--stable-balance-token-identifier` | - | Stable balance token identifier |
| `--stable-balance-threshold` | - | Stable balance threshold in sats |

### Data Directory

The `--data-dir` (`-d`) option sets where wallet data is stored (default: `./.data`). Each wallet instance needs its own unique data directory.

```bash
cargo run -- --data-dir ~/.breez/my-wallet
```

### Network

The `--network` option selects which network to use (default: `regtest`):

```bash
# Regtest (no API key needed)
cargo run -- --network regtest

# Mainnet
cargo run -- --network mainnet
```

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

**Other**: `parse`, `list-fiat-currencies`, `list-fiat-rates`, `get-user-settings`, `set-user-settings`, `get-spark-status`

Each command supports `--help` for detailed usage, e.g. `receive --help`.

## Language Ports

This Rust CLI is the source of truth. Automated ports are maintained in [`bindings/examples/cli/`](../bindings/examples/cli/):

| Language | Path |
|----------|------|
| [Python](../bindings/examples/cli/langs/python/) | `bindings/examples/cli/langs/python/` |
| [Go](../bindings/examples/cli/langs/go/) | `bindings/examples/cli/langs/go/` |

Changes to this CLI trigger a [sync workflow](../../../.github/workflows/sync-cli-langs.yml) that automatically opens PRs to update each language port.
