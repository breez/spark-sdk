# Breez SDK CLI

Command-line interface for testing and interacting with the Breez SDK (Spark).

## Usage

```bash
cargo run -- [OPTIONS]
```

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

### Seedless Restore

Seedless restore allows wallet recovery without storing a mnemonic on disk. Instead, a hardware key (YubiKey) or file-based secret is used to deterministically derive wallet seeds via HMAC challenge-response.

Salts are stored on Nostr relays, allowing discovery during restore.


#### File Provider

Uses a random 32-byte secret stored in `<data-dir>/seedless-restore-secret`. The secret is generated on first use. Suitable for development and testing.

```bash
# Create or restore with a salt
cargo run -- --seedless file --seedless-salt mysalt

# List and select from existing salts
cargo run -- --seedless file
```

#### YubiKey Provider

Uses YubiKey HMAC-SHA1 challenge-response (Slot 2) as the PRF.

```bash
# Create or restore with a salt
cargo run -- --seedless yubikey --seedless-salt mysalt

# List and select from existing salts
cargo run -- --seedless yubikey
```

> **Note**: This provider is **not compatible** with browser WebAuthn PRF. Use the FIDO2 provider for cross-platform compatibility.

#### FIDO2 Provider

Uses FIDO2/WebAuthn PRF via the CTAP2 hmac-secret extension. This is **compatible with browser-based passkeys** - the same credential can derive identical seeds in both CLI and browser when using the same relying party ID (rpId).

> **Note**: The FIDO2 provider requires the `fido2` feature flag (uses `hidapi` which needs system HID libraries).

```bash
# Create or restore with a salt (uses default rpId: keys.breez.technology)
cargo run --features fido2 -- --seedless fido2 --seedless-salt mysalt

# Use custom rpId for compatibility with a specific web app
cargo run --features fido2 -- --seedless fido2 --seedless-rpid localhost --seedless-salt mysalt

# List and select from existing salts
cargo run --features fido2 -- --seedless fido2
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
cargo run --features fido2 -- --seedless fido2 --seedless-salt mysalt

# Non-interactive via environment variable
FIDO2_PIN=123456 cargo run --features fido2 -- --seedless fido2 --seedless-salt mysalt
```

**Cross-platform compatibility:**

For the CLI and browser to derive the same seed:
1. Use the same relying party ID (`--seedless-rpid` must match browser's `rpId`)
2. Use the same credential (registered on the same authenticator)
3. Use the same salt

### YubiKey OTP Setup (for `--seedless yubikey`)

The YubiKey provider requires HMAC-SHA1 challenge-response to be configured on **Slot 2**.

#### Prerequisites

Install the YubiKey Manager CLI:

```bash
# macOS
brew install ykman

# Debian/Ubuntu
apt install yubikey-manager

# Arch Linux
pacman -S yubikey-manager
```

#### Check current slot configuration

```bash
ykman otp info
```

Example output:

```
Slot 1: programmed    # Typically Yubico OTP
Slot 2: empty         # Needs to be configured
```

#### Program Slot 2 for HMAC challenge-response

Generate a random secret key and program it into Slot 2:

```bash
# Without touch requirement (responds immediately)
ykman otp chalresp -g 2

# With touch requirement (requires physical touch for each challenge)
ykman otp chalresp -g -t 2
```

> **Warning**: This overwrites whatever is currently in Slot 2. If Slot 2 is already programmed, make sure you no longer need its current configuration.

> **Important**: The secret key programmed into the YubiKey is what makes your wallet derivation unique. If you reprogram Slot 2 with a different key, you will derive different wallets for the same salts. There is no way to recover the previous key.

#### Verify the configuration

```bash
ykman otp info
```

Both slots should now show `programmed`:

```
Slot 1: programmed
Slot 2: programmed
```

#### Disable OTP output (optional)

If your YubiKey outputs random characters (like `ccccc...`) when touched, you can disable Slot 1 OTP:

```bash
ykman otp delete 1
```

### FIDO2 Setup (for `--seedless fido2`)

The FIDO2 provider uses the CTAP2 hmac-secret extension for WebAuthn-compatible PRF.

#### Requirements

- **YubiKey 5 series** with firmware **5.2 or later**
- Or any FIDO2 security key supporting the `hmac-secret` extension

Check your YubiKey firmware version:

```bash
ykman info
```

Look for `Firmware version: 5.x.x` (must be 5.2+).

#### Set a FIDO2 PIN

A PIN is required for hmac-secret operations. Set one if you haven't:

```bash
ykman fido access change-pin
```

#### Verify hmac-secret support

```bash
ykman fido info
```

Look for `hmac-secret` in the extensions list.

#### First use

On first use with a new rpId, the CLI will automatically register a discoverable credential (passkey) on your authenticator. This requires one touch. Subsequent uses only require one touch for PRF evaluation.

### How It Works

1. **Account master derivation**: `PRF(key, magic_salt)` produces a 32-byte account master used to derive a Nostr identity at `m/44'/1237'/55'/0/0`.
2. **Salt storage**: User-provided salts are published as Nostr events, allowing discovery during restore.
3. **Wallet seed derivation**: `PRF(key, user_salt)` produces 32 bytes that are converted to a 24-word BIP39 mnemonic.

The PRF function differs by provider:
- **File**: `HMAC-SHA256(file_secret, salt)`
- **YubiKey**: `SHA256(HMAC-SHA1(slot2_secret, salt))` - OTP challenge-response
- **FIDO2**: `HMAC-SHA256(credential_key, SHA256("WebAuthn PRF" || 0x00 || salt))` - WebAuthn PRF

The FIDO2 provider applies the [WebAuthn PRF salt transformation](https://w3c.github.io/webauthn/#prf-extension) for browser compatibility.

Each `derive_prf_seed` call requires a physical touch. The list-salts flow requires one derivation (for Nostr identity), and the restore flow requires an additional derivation (for seed).
