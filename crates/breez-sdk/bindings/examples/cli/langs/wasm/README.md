# Breez SDK CLI (TypeScript/Node.js)

Command-line interface for testing and interacting with the Breez SDK (Spark) using the WASM/Node.js bindings.

> **Note:** The [Rust CLI](../../../../../cli/) is the source of truth. This CLI is an automated port that mirrors its commands, arguments, and behavior. Changes should be made to the Rust CLI first -- a sync workflow will open a PR to update this CLI automatically.

## Prerequisites

- **Node.js >= 22** (required by the WASM bindings)
- **npm** (comes with Node.js)

## Setup

```bash
# Install dependencies
make setup

# Or manually
npm install
```

## Environment Variables

| Variable | Description |
|----------|-------------|
| `BREEZ_API_KEY` | Breez API key (required for mainnet, get one at [breez.technology](https://breez.technology/request-api-key/)) |

You can create a `.env` file in this directory with your environment variables:

```env
BREEZ_API_KEY=your_api_key_here
```

## Usage

```bash
# Run on regtest (default)
make run

# Run on mainnet
make run-mainnet

# Or run directly with options
node src/main.js [OPTIONS]
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

### Examples

```bash
# Use a custom data directory
node src/main.js --data-dir ~/.breez/my-wallet

# Use PostgreSQL storage
node src/main.js --postgres-connection-string "host=localhost user=postgres dbname=spark"

# Use a custom account number
node src/main.js --account-number 21
```

## Available Commands

Once inside the REPL, type `help` to see all commands. The CLI supports:

**Wallet**: `get-info`, `sync`, `get-payment`, `list-payments`, `recommended-fees`

**Payments**: `receive`, `pay`, `lnurl-pay`, `lnurl-withdraw`, `lnurl-auth`, `claim-htlc-payment`

**On-chain**: `claim-deposit`, `refund-deposit`, `list-unclaimed-deposits`, `buy-bitcoin`

**Lightning address**: `get-lightning-address`, `register-lightning-address`, `delete-lightning-address`, `check-lightning-address-available`

**Tokens**: `get-tokens-metadata`, `fetch-conversion-limits`, `issuer <subcommand>`

**Contacts**: `contacts add`, `contacts update`, `contacts delete`, `contacts list`

**Other**: `parse`, `list-fiat-currencies`, `list-fiat-rates`, `get-user-settings`, `set-user-settings`, `get-spark-status`

Each command supports `--help` for detailed usage, e.g., `receive --help`.

## Passkey

Using a passkey enables a deterministic seed to be derived without storing a mnemonic on disk. Instead, a file-based secret (or hardware key) is used to deterministically derive wallet seeds via HMAC challenge-response.

Wallet names are stored on Nostr relays, allowing discovery during restore. If no `--wallet-name` is specified, the default wallet name ("Default") is used.

### PRF Providers

#### File Provider

Uses a random 32-byte secret stored in `<data-dir>/seedless-restore-secret`. The secret is generated on first use. Suitable for development and testing.

```bash
# Use passkey with the default wallet name
node src/main.js --passkey file

# Use passkey with a specific wallet name
node src/main.js --passkey file --wallet-name personal

# Use passkey after selecting a wallet name published to Nostr
node src/main.js --passkey file --list-wallet-names

# Use passkey with a specific wallet name and publish the wallet name to Nostr
node src/main.js --passkey file --wallet-name personal --store-wallet-name
```

#### YubiKey Provider

Not yet available in Node.js CLI. See the [Rust CLI README](../../../../../cli/README.md) for details.

#### FIDO2 Provider

Not yet available in Node.js CLI. See the [Rust CLI README](../../../../../cli/README.md) for details.
