# Spark CLI

A command-line tool for interacting with Spark wallets, including performing unilateral exits.

## Prerequisites

### Install Rust

If you don't have Rust installed, install it via rustup:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
```

Verify installation:
```bash
rustc --version
cargo --version
```

## Running the CLI

Clone the repository and run:

```bash
git clone https://github.com/breez/spark-sdk.git
cd spark-sdk
cargo run -p spark-cli
```

### Command-Line Options

| Option | Description | Default |
|--------|-------------|---------|
| `--network` | Network: `mainnet` or `regtest` | `mainnet` |
| `-d`, `--data-dir` | Data directory path | `.spark` |

Example with options:
```bash
cargo run -p spark-cli -- --network regtest -d ~/.spark-regtest
```

## Entering Your Mnemonic

When the CLI starts, you'll be prompted to enter your wallet's recovery phrase:

```
Enter your BIP-39 mnemonic phrase: word1 word2 word3 word4 word5 word6 word7 word8 word9 word10 word11 word12

Enter passphrase (or press Enter for none):
```

- Enter your 12 or 24 word mnemonic phrase
- If you used a passphrase when creating your wallet, enter it; otherwise press Enter

## Unilateral Exit

A unilateral exit allows you to withdraw your funds on-chain without cooperation from the Spark operators. Use this as a last resort if operators are unresponsive.

### Step 1: Check Your Leaves

First, list your wallet's leaves to see what can be exited:

```
spark-cli [mainnet]> leaves list --compact
```

This shows your leaf IDs and their values.

### Step 2: Prepare a UTXO for Fees

You need a Bitcoin UTXO from a separate Bitcoin wallet to pay for transaction fees. This cannot come from your Spark wallet - it must be an on-chain UTXO you control in another wallet (e.g., Sparrow, Electrum, Bitcoin Core).

The UTXO format is:
```
txid:vout:value_sats:pubkey_hex
```

- `txid` - Transaction ID of the UTXO
- `vout` - Output index (usually 0)
- `value_sats` - Value in satoshis
- `pubkey_hex` - Public key of the external wallet that owns this UTXO (hex format)

Example:
```
abc123...def:0:50000:02abc123...
```

### Step 3: Execute Unilateral Exit

Run the unilateral exit command:

```
spark-cli [mainnet]> withdraw unilateral-exit <fee_rate> --leaf <leaf_id> --utxo <utxo>
```

Parameters:
- `<fee_rate>` - Fee rate in sats/vbyte (e.g., `10`)
- `--leaf <leaf_id>` - The leaf ID to exit (from step 1)
- `--utxo <utxo>` - Your fee-paying UTXO (from step 2)

Example:
```
spark-cli [mainnet]> withdraw unilateral-exit 10 --leaf abc123 --utxo def456:0:50000:02abc...
```

### Step 4: Broadcast Transactions

The CLI will output transaction data and PSBTs in order:

1. **Node TX(s)** - Parent node transactions (if any)
2. **Leaf TX** - The leaf node transaction
3. **Refund TX** - The transaction that pays your funds to your address

For each transaction, you need to:
1. Sign the PSBT with your key (or use `-s` flag for auto-signing)
2. Broadcast the TX and signed PSBT together
3. Wait for confirmation before broadcasting the next

**Important about the Refund TX:** The refund transaction has a CSV (CheckSequenceVerify) relative timelock. The CLI will display the timelock value, e.g., "CSV Timelock: 144 blocks after Leaf TX confirms". You must wait for this many blocks after the Leaf TX confirms before you can broadcast the Refund TX.

## Advanced: Auto-signing PSBTs

If you're comfortable providing your private key, the CLI can sign the PSBTs for you. Add the `-s` flag with your hex-encoded private key:

```
spark-cli [mainnet]> withdraw unilateral-exit 10 --leaf abc123 --utxo def456:0:50000:02abc... -s <private_key_hex>
```

**Warning:** Only use this on a secure, private machine. Never share your private key.

Without the signing key, you'll need to sign the PSBTs manually using your external Bitcoin wallet before broadcasting.

## Other Useful Commands

```
spark-cli [mainnet]> balance          # Check your balance
spark-cli [mainnet]> info             # Wallet information
spark-cli [mainnet]> help             # List all commands
```

## Troubleshooting

### "Invalid mnemonic"
- Ensure words are separated by single spaces
- Check for typos in mnemonic words
- Verify you're using a valid BIP-39 word list

### Connection errors
- Check your internet connection
- Verify the network setting matches your wallet

### Exit command fails
- Ensure you have a valid UTXO for fees
- Check that the leaf ID exists and has funds
