# nip65-publisher

Internal tool to publish the Breez NIP-65 relay list.

SDK clients fetch the authoritative relay list from a well-known Breez pubkey's kind-10002 event. This tool signs and broadcasts that event.

## Usage

```bash
cargo run -p nip65-publisher -- \
  --private-key <breez-identity-hex-privkey> \
  --api-key <breez-api-key-base64> \
  --relay wss://relay.primal.net \
  --relay wss://relay.damus.io \
  --relay wss://relay.nostr.watch \
  --relay wss://relaypag.es \
  --relay wss://monitorlizard.nostr1.com
```

## Arguments

| Argument | Env variable | Description |
|---|---|---|
| `--private-key` | `NIP65_PRIVATE_KEY` | Breez identity private key (hex or nsec1 bech32). Signs the NIP-65 event. The corresponding public key should match `BREEZ_NIP65_PUBKEY` in the SDK. |
| `--api-key` | `NIP65_API_KEY` | Breez API key (base64). Used for NIP-42 authentication with the Breez relay. |
| `--relay` | — | Recommended relay URL (repeatable). The Breez relay (`wss://nr1.breez.technology`) is always included first automatically. |

Both `--private-key` and `--api-key` can be set via environment variable or a `.env` file instead of passing them on the command line.

## Standalone binary

```bash
cargo build -p nip65-publisher --release
```

The binary is output to `target/release/nip65-publisher` and can be run independently without the source tree:

```bash
./nip65-publisher \
  --private-key <key> \
  --api-key <api-key> \
  --relay wss://relay.primal.net
```

## How it works

1. Signs a NIP-65 (kind-10002) relay list event with the Breez identity key
2. Authenticates with the Breez relay using NIP-42 (keypair derived from the API key)
3. Publishes the event to the Breez relay and all specified recommended relays
