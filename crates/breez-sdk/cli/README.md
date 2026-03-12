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
| `--passkey` | - | Use Passkey with PRF provider (`file`, `yubikey` or `fido2`) |
| `--label` | `Default` | Requires `--passkey`. The label to use |
| `--list-labels` | false | Requires `--passkey`. Select label from NOSTR |
| `--store-label` | false | Requires `--passkey`. Publish label to NOSTR |
| `rpid` | `keys.breez.technology` | Requires `--passkey`. Relying party ID for FIDO2 provider |

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
| [C#](../bindings/examples/cli/langs/csharp/) | `bindings/examples/cli/langs/csharp/` |
| [Flutter](../bindings/examples/cli/langs/flutter/) | `bindings/examples/cli/langs/flutter/` |
| [Go](../bindings/examples/cli/langs/golang/) | `bindings/examples/cli/langs/golang/` |
| [Kotlin Multiplatform](../bindings/examples/cli/langs/kotlin-multiplatform/) | `bindings/examples/cli/langs/kotlin-multiplatform/` |
| [Python](../bindings/examples/cli/langs/python/) | `bindings/examples/cli/langs/python/` |
| [React Native](../bindings/examples/cli/langs/react-native/) | `bindings/examples/cli/langs/react-native/` |
| [Swift](../bindings/examples/cli/langs/swift/) | `bindings/examples/cli/langs/swift/` |
| [WASM/TypeScript](../bindings/examples/cli/langs/wasm/) | `bindings/examples/cli/langs/wasm/` |

Changes to this CLI trigger a [sync workflow](../../../.github/workflows/sync-cli-langs.yml) that automatically opens PRs to update each language port.

## Passkey

Using a passkey enables a deterministic seed to be derived without storing a mnemonic on disk. Instead, a hardware key (YubiKey) or file-based secret is used to deterministically derive wallet seeds via HMAC challenge-response.

Labels are stored on Nostr relays, allowing discovery during restore. If no `--label` is specified, the default label ("Default") is used.

### How It Works

1. **Account master derivation**: `PRF(key, magic_salt)` produces a 32-byte account master used to derive a Nostr identity at `m/44'/1237'/55'/0/0`.
2. **Label storage**: Labels are published as Nostr events, allowing discovery during restore.
3. **Wallet seed derivation**: `PRF(key, user_salt)` produces 32 bytes that are converted to a 24-word BIP39 mnemonic.

The PRF function differs by provider:
- **File**: `HMAC-SHA256(file_secret, label)`
- **YubiKey**: `SHA256(HMAC-SHA1(slot2_secret, label))` - OTP challenge-response
- **FIDO2**: `HMAC-SHA256(credential_key, SHA256("WebAuthn PRF" || 0x00 || label))` - WebAuthn PRF

The FIDO2 provider applies the [WebAuthn PRF salt transformation](https://w3c.github.io/webauthn/#prf-extension) for browser compatibility.

Each `derive_prf_seed` call requires a physical touch. The `--list-labels` flow requires one derivation (for Nostr identity), and the seed derivation requires an additional derivation (for the seed).

### PRF Providers

#### File Provider

Uses a random 32-byte secret stored in `<data-dir>/seedless-restore-secret`. The secret is generated on first use. Suitable for development and testing.

```bash
# Use passkey with the default label
cargo run -- --passkey file

# Use passkey with a specific label
cargo run -- --passkey file --label personal

# Use passkey after selecting a label published to Nostr
cargo run -- --passkey file --list-labels

# Use passkey with a specific label and publish the label to Nostr
cargo run -- --passkey file --label personal --store-label
```

#### YubiKey Provider

Uses YubiKey HMAC-SHA1 challenge-response (Slot 2) as the PRF.

```bash
# Use passkey with the default label
cargo run -- --passkey yubikey

# Use passkey with a specific label
cargo run -- --passkey yubikey --label personal

# Use passkey after selecting a label published to Nostr
cargo run -- --passkey yubikey --list-labels

# Use passkey with a specific label and publish the label to Nostr
cargo run -- --passkey yubikey --label personal --store-label
```

> **Note**: This provider is **not compatible** with browser WebAuthn PRF. Use the FIDO2 provider for cross-platform compatibility.

#### FIDO2 Provider

Uses FIDO2/WebAuthn PRF via the CTAP2 hmac-secret extension. This is **compatible with browser-based passkeys** - the same credential can derive identical seeds in both CLI and browser when using the same relying party ID (rpId).

> **Note**: The FIDO2 provider requires the `fido2` feature flag (uses `hidapi` which needs system HID libraries).

```bash
# Use passkey with the default label (uses default rpId: keys.breez.technology)
cargo run --features fido2 -- --passkey fido2

# Use passkey with a specific label
cargo run --features fido2 -- --passkey fido2 --label personal

# Use custom rpId for compatibility with a specific web app
cargo run --features fido2 -- --passkey fido2 --rpid localhost --label personal

# Use passkey after selecting a label published to Nostr
cargo run --features fido2 -- --passkey fido2 --list-labels

# Use passkey with a specific label and publish the label to Nostr
cargo run --features fido2 -- --passkey fido2 --label personal --store-label
```

**Requirements:**
- YubiKey 5 series with **firmware 5.2+** (supports hmac-secret extension)
- Or any FIDO2 authenticator that supports the hmac-secret extension
- System HID libraries (libhidapi on Linux, included on macOS/Windows)

**PIN Configuration:**

The FIDO2 provider requires a PIN. You can provide it via:

1. **Interactive prompt** (default): Enter PIN when prompted
2. **Environment variable**: Set `FIDO2_PIN` for non-interactive/CI use

```bash
# Interactive (prompts for PIN)
cargo run --features fido2 -- --passkey fido2 --label personal

# Non-interactive via environment variable
FIDO2_PIN=123456 cargo run --features fido2 -- --passkey fido2 --label personal
```

**Cross-platform compatibility:**

For the CLI and browser to derive the same seed:
1. Use the same relying party ID (`--rpid` must match browser's `rpId`)
2. Use the same credential (registered on the same authenticator)
3. Use the same label

### Yubikey Setup

#### OTP Setup (for `--passkey yubikey`)

The YubiKey provider requires HMAC-SHA1 challenge-response to be configured on **Slot 2**.

##### Prerequisites

Install the YubiKey Manager CLI:

```bash
# macOS
brew install ykman

# Debian/Ubuntu
apt install yubikey-manager

# Arch Linux
pacman -S yubikey-manager
```

##### Check current slot configuration

```bash
ykman otp info
```

Example output:

```
Slot 1: programmed    # Typically Yubico OTP
Slot 2: empty         # Needs to be configured
```

##### Program Slot 2 for HMAC challenge-response

Generate a random secret key and program it into Slot 2:

```bash
# Without touch requirement (responds immediately)
ykman otp chalresp -g 2

# With touch requirement (requires physical touch for each challenge)
ykman otp chalresp -g -t 2
```

> **Warning**: This overwrites whatever is currently in Slot 2. If Slot 2 is already programmed, make sure you no longer need its current configuration.

> **Important**: The secret key programmed into the YubiKey is what makes your wallet derivation unique. If you reprogram Slot 2 with a different key, you will derive different wallets for the same labels. There is no way to recover the previous key.

##### Verify the configuration

```bash
ykman otp info
```

Both slots should now show `programmed`:

```
Slot 1: programmed
Slot 2: programmed
```

##### Disable OTP output (optional)

If your YubiKey outputs random characters (like `ccccc...`) when touched, you can disable Slot 1 OTP:

```bash
ykman otp delete 1
```

#### FIDO2 Setup (for `--passkey fido2`)

The FIDO2 provider uses the CTAP2 hmac-secret extension for WebAuthn-compatible PRF.

##### Requirements

- **YubiKey 5 series** with firmware **5.2 or later**
- Or any FIDO2 security key supporting the `hmac-secret` extension

Check your YubiKey firmware version:

```bash
ykman info
```

Look for `Firmware version: 5.x.x` (must be 5.2+).

##### Set a FIDO2 PIN

A PIN is required for hmac-secret operations. Set one if you haven't:

```bash
ykman fido access change-pin
```

##### Verify hmac-secret support

```bash
ykman fido info
```

Look for `hmac-secret` in the extensions list.

##### First use

On first use with a new rpId, the CLI will automatically register a discoverable credential (passkey) on your authenticator. This requires one touch. Subsequent uses only require one touch for PRF evaluation.
