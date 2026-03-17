# Breez SDK CLI (C# / .NET)

C# port of the [Breez SDK Spark CLI](../../../../cli/README.md).

## Prerequisites

- [.NET 8 SDK](https://dotnet.microsoft.com/download/dotnet/8.0) or later
- [Breez.Sdk.Spark NuGet package](https://www.nuget.org/packages/Breez.Sdk.Spark)

## Setup

```bash
# Restore NuGet packages
make setup
```

## Usage

```bash
# Build and run on regtest (default)
make run

# Build and run on mainnet
make run-mainnet

# Or run directly with dotnet
dotnet run --project BreezCli.csproj -- [OPTIONS]
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
| `--passkey` | - | Use passkey with PRF provider (`file`, `yubikey`, or `fido2`) |
| `--label` | - | Label for seed derivation (requires `--passkey`) |
| `--list-labels` | - | List and select from labels published to Nostr (requires `--passkey`) |
| `--store-label` | - | Publish the label to Nostr (requires `--passkey` and `--label`) |
| `--rpid` | - | Relying party ID for FIDO2 provider (requires `--passkey`) |

### Passkey Support

The CLI supports seedless wallet creation using passkey-based PRF (Pseudo-Random Function) providers:

- **`file`** -- Uses HMAC-SHA256 with a secret stored in the data directory. Suitable for development and testing.
- **`yubikey`** -- YubiKey hardware key (not yet implemented).
- **`fido2`** -- FIDO2/WebAuthn PRF using CTAP2 hmac-secret extension (not yet implemented).

```bash
# Use file-based passkey provider
dotnet run --project BreezCli.csproj -- --passkey file

# Use a specific label
dotnet run --project BreezCli.csproj -- --passkey file --label "personal"

# List labels from Nostr and select one
dotnet run --project BreezCli.csproj -- --passkey file --list-labels

# Publish a label to Nostr
dotnet run --project BreezCli.csproj -- --passkey file --label "personal" --store-label
```

### Environment Variables

| Variable | Description |
|----------|-------------|
| `BREEZ_API_KEY` | Breez API key (required for mainnet) |

### Available Commands

Once inside the REPL, type `help` for all commands:

**Wallet**: `get-info`, `sync`, `get-payment`, `list-payments`, `recommended-fees`

**Payments**: `receive`, `pay`, `lnurl-pay`, `lnurl-withdraw`, `lnurl-auth`, `claim-htlc-payment`

**On-chain**: `claim-deposit`, `refund-deposit`, `list-unclaimed-deposits`, `buy-bitcoin`

**Lightning address**: `get-lightning-address`, `register-lightning-address`, `delete-lightning-address`, `check-lightning-address-available`

**Tokens**: `get-tokens-metadata`, `fetch-conversion-limits`, `issuer <subcommand>`

**Contacts**: `contacts <subcommand>`

**Other**: `parse`, `list-fiat-currencies`, `list-fiat-rates`, `get-user-settings`, `set-user-settings`, `get-spark-status`

## Build

```bash
make build
```

## Clean

```bash
make clean
```
