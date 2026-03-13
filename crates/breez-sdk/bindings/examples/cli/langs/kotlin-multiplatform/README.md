# Breez SDK - Spark CLI (Kotlin)

Interactive CLI client for the [Breez SDK](../../../../../../../README.md) with Spark, written in Kotlin (JVM).

> **Note:** The [Rust CLI](../../../../../cli/) is the source of truth. This CLI is an automated port that mirrors its commands, arguments, and behavior. Changes should be made to the Rust CLI first.

## Prerequisites

- JDK >= 17
- Gradle (or use the Gradle wrapper after running `gradle wrapper`)

## Quick Start

```bash
# 1. Generate Gradle wrapper (one-time)
gradle wrapper

# 2. Set API key (required for mainnet)
export BREEZ_API_KEY="<your-api-key>"

# 3. Run (regtest by default, no API key needed)
make run
```

## Using Published SDK

The default `build.gradle.kts` fetches the published SDK from the Breez Maven repository. Just build and run:

```bash
# 1. Generate Gradle wrapper (one-time)
gradle wrapper

# 2. Download dependencies
make setup-published

# 3. Build and run
make run
```

## Makefile Targets

```
make setup            Build local Kotlin bindings from Rust source
make setup-published  Download dependencies from published SDK
make build            Build the CLI
make run              Build + run on regtest (default)
make run-mainnet      Build + run on mainnet
make clean            Remove build artifacts
```

## CLI Options

| Option | Default | Description |
|--------|---------|-------------|
| `-d`, `--data-dir` | `./.data` | Path to the data directory |
| `--network` | `regtest` | Network to use (`regtest` or `mainnet`) |
| `--account-number` | | Account number for the Spark signer |
| `--postgres-connection-string` | | PostgreSQL connection string (uses SQLite by default) |
| `--stable-balance-token-identifier` | | Stable balance token identifier |
| `--stable-balance-threshold` | | Stable balance threshold in sats |
| `--passkey` | | Use passkey with PRF provider (`file`, `yubikey`, or `fido2`) |
| `--label` | | Label for seed derivation (requires `--passkey`) |
| `--list-labels` | | List and select from labels on Nostr (requires `--passkey`) |
| `--store-label` | | Publish the label to Nostr (requires `--passkey` + `--label`) |
| `--rpid` | | Relying party ID for FIDO2 provider (requires `--passkey`) |

## Environment Variables

| Variable | Description |
|----------|-------------|
| `BREEZ_API_KEY` | Breez API key (required for mainnet, get one at [breez.technology](https://breez.technology/request-api-key/)) |

## Passkey Support

The CLI supports passkey-based seed derivation using the `--passkey` flag. Three providers are available:

- **`file`** -- File-based PRF using HMAC-SHA256 with a secret stored in the data directory. Suitable for development and testing.
- **`yubikey`** -- YubiKey hardware key (not yet supported, stub only).
- **`fido2`** -- FIDO2/WebAuthn PRF (not yet supported, stub only).

### Examples

```bash
# Use file-based passkey (generates/reuses secret in data dir)
./gradlew run --console=plain --args="--passkey file"

# Use passkey with a custom label
./gradlew run --console=plain --args="--passkey file --label personal"

# List labels from Nostr and select one
./gradlew run --console=plain --args="--passkey file --list-labels"

# Publish a label to Nostr
./gradlew run --console=plain --args="--passkey file --label personal --store-label"
```

## Features

- **Tab completion** -- Press Tab to auto-complete command names (powered by JLine3).
- **Command history** -- Previous commands are saved and accessible with arrow keys.
- **Passkey seed derivation** -- Derive wallet seeds from passkey PRF providers instead of BIP39 mnemonics.

## Available Commands

Once inside the REPL, type `help` to see all commands. The CLI supports:

**Wallet**: `get-info`, `sync`, `get-payment`, `list-payments`, `recommended-fees`

**Payments**: `receive`, `pay`, `lnurl-pay`, `lnurl-withdraw`, `lnurl-auth`, `claim-htlc-payment`

**On-chain**: `claim-deposit`, `refund-deposit`, `list-unclaimed-deposits`, `buy-bitcoin`

**Lightning address**: `get-lightning-address`, `register-lightning-address`, `delete-lightning-address`, `check-lightning-address-available`

**Tokens**: `get-tokens-metadata`, `fetch-conversion-limits`

**Fiat**: `list-fiat-currencies`, `list-fiat-rates`

**Settings**: `get-user-settings`, `set-user-settings`, `get-spark-status`

**Token issuer**: `issuer create-token`, `issuer mint-token`, `issuer burn-token`, `issuer token-balance`, `issuer token-metadata`, `issuer freeze-token`, `issuer unfreeze-token`

**Contacts**: `contacts add`, `contacts update`, `contacts delete`, `contacts list`

**Input parsing**: `parse`

Each command supports its own usage help.

## Development

```bash
# Build and run directly
./gradlew run --console=plain --args="--network regtest"

# Build a fat JAR
./gradlew jar
java -jar build/libs/breez-sdk-spark-cli-1.0-SNAPSHOT.jar --network regtest
```
