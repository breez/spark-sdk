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

### YubiKey Setup

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

### How It Works

1. **Account master derivation**: `HMAC(key, magic_salt)` produces a 32-byte account master used to derive a Nostr identity at `m/44'/1237'/55'/0/0`.
2. **Salt storage**: User-provided salts are published as Nostr events, allowing discovery during restore.
3. **Wallet seed derivation**: `HMAC(key, user_salt)` produces 32 bytes that are converted to a 24-word BIP39 mnemonic.

If the YubiKey was programmed with `-t` (touch required), each `derive_prf_seed` call will require a physical touch (you will see a "Touch your YubiKey" prompt). The list-salts flow requires one derivation (for Nostr identity), and the restore flow requires an additional derivation (for seed).
