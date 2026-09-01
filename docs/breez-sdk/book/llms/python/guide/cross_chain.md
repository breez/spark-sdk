# Send USDC/USDT

The SDK can send USDC or USDT from a Spark wallet to a recipient on any of several supported chains: Ethereum-family chains (Arbitrum, Base, and similar EVM networks), Solana, and Tron. The source on the Spark side is either BTC sats or USDB. The SDK orchestrates two legs — a Spark-side transfer to a provider-controlled deposit and the provider-driven delivery of the destination asset — and reconciles both onto a single `Payment` row.

The send flow itself lives in the [Sending payments](./send_payment.md#usdc-usdt) page. This page covers how it works under the hood: the providers, the lifecycle, retry semantics, and limitations.

## Supported address formats

`parse` recognizes cross-chain destinations in the following forms, returning `InputType.CROSS_CHAIN_ADDRESS` with the parsed `CrossChainAddressDetails` — address family, bare address, and optional token contract address, chain id, and amount.

### Bare addresses

The SDK detects three address families from format alone. A bare address parses with no `contract_address`, `chain_id`, or `amount` — the caller selects the destination chain and asset via `get_cross_chain_routes`.

- **EVM** — `0x` + 40 hex characters (lowercase or checksummed):
  ```
  0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913
  ```
- **Solana** — base58 encoding of a 32-byte public key:
  ```
  EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v
  ```
- **Tron** — base58check with a `T` prefix (34 characters total):
  ```
  TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t
  ```

### Canonical URIs

URIs let the recipient encode chain, token contract, and amount alongside the address. Unknown query parameters are ignored.

- **EVM** — [EIP-681](https://eips.ethereum.org/EIPS/eip-681). Native send or ERC-20 transfer; the optional `@<chain_id>` suffix is the EIP-681 chain identifier (e.g. `8453` for Base):
  ```
  ethereum:<addr>[@<chain_id>]?value=<wei>
  ethereum:<contract>[@<chain_id>]/transfer?address=<to>&uint256=<amount>
  ```
- **Solana** — Solana Pay-style. `spl-token=` carries the SPL mint when the destination is an SPL token rather than native SOL:
  ```
  solana:<addr>?amount=<amount>&spl-token=<mint>
  ```
- **Tron** — TRC-20 destinations carry the contract on `token=`:
  ```
  tron:<addr>?amount=<amount>&token=<contract>
  ```

URIs whose recipient address doesn't match the scheme's address family (e.g. a `solana:` URI carrying an EVM address) are not recognized as cross-chain. Unknown schemes are not recognized as cross-chain either — they may still be classified by another input type if the format matches.

## Providers

The SDK ships with two cross-chain providers. `get_cross_chain_routes` returns the union of routes offered by each, tagged with `CrossChainRoutePair.provider`.

| Provider     | Source assets    | Destinations                                                | Mechanism                            |
| ------------ | ---------------- | ----------------------------------------------------------- | ------------------------------------ |
| **Orchestra** (Flashnet) | BTC sats + USDB | USDC / USDT on Ethereum chains (Arbitrum, Base), Solana, Tron | Spark transfer to a deposit address, then provider bridges to the destination chain |
| **Boltz**    | BTC sats only    | USDC / USDT on Ethereum chains (Arbitrum, Base), Solana, Tron                 | Lightning reverse swap: SDK pays a hold invoice, provider claims the on-chain leg |

The provider tag on each `CrossChainRoutePair` is the source of truth. When the same destination is offered by multiple providers, both routes are returned; the caller picks one based on supported source assets, fees, or other preferences.

## Slippage

Cross-chain slippage protects the recipient from price movement between quote and delivery. Values are expressed in basis points (1 bps = 0.01%).

Resolution at prepare time:

1. The per-request `max_slippage_bps` on `PaymentRequest.CROSS_CHAIN` wins if set.
2. Otherwise, the SDK falls back to `default_slippage_bps` on `CrossChainConfig` from [the SDK configuration](./config.md#send-usdc-usdt).
3. Otherwise, the built-in default of 100 bps (1%) is used.

Values outside 10 to 500 are rejected at both config validation and per-request validation.

## Quote expiry

Each cross-chain prepare response carries an `expires_at` quote-expiry timestamp on `SendPaymentMethod.CROSS_CHAIN_ADDRESS`. If the quote has expired by the time you call `send_payment`, you must re-prepare to obtain a fresh quote (with a new `expires_at`) and try again.

## Status lifecycle

The Spark/USDB token transfer and the cross-chain delivery have distinct status fields. They are tracked separately on the persisted `Payment` row so each can settle independently.

| Field                                                 | Reflects                                                       |
| ----------------------------------------------------- | -------------------------------------------------------------- |
| `status`                                      | The Spark or USDB token transfer (sender-side settlement)      |
| `conversion_info.status`                      | The provider-driven cross-chain leg                            |
| `conversion_info.delivered_amount`            | Final amount delivered to the recipient, set when terminal     |

The cross-chain status walks one of:

- **`ConversionStatus.PENDING`** — deposit transfer submitted, provider working on the cross-chain leg.
- **`ConversionStatus.COMPLETED`** — provider reports the order terminal-successful; `delivered_amount` is set.
- **`ConversionStatus.REFUND_NEEDED`** — provider rejected the submit or order failed before delivery; the local Spark transfer is settled and the deposit is sitting at the provider awaiting refund.
- **`ConversionStatus.REFUNDED`** — the funds have been refunded back to the wallet.
- **`ConversionStatus.FAILED`** — terminal failure with no refund pending.

A background monitor runs while the SDK is active and reconciles `ConversionStatus.REFUND_NEEDED` and `ConversionStatus.PENDING` rows onto their terminal state by polling the provider.

## Retry safety

Calling `send_payment` is safe to retry on transient errors **only when the send has no token-transfer leg.** Whether the source asset displayed on the route is BTC or USDB is not the determinant — what matters is the actual first leg the SDK executes.

### Sends with no token leg

When the first leg is a Spark sats transfer (Orchestra with BTC source, or Boltz funded directly from the sats balance), the SDK threads a deterministic transfer id through to the underlying Spark transfer. Retrying with the same `PrepareSendPaymentResponse` produces the same transfer id, and the Spark protocol returns the original transfer instead of firing a new one — no double-deposit.

Two ways to drive idempotency:

1. **Pass a caller-supplied `idempotency_key`** on `SendPaymentRequest`. The top-level dispatcher first looks for an existing payment with that id and short-circuits the retry if found; otherwise the key is used as the Spark transfer id.
2. **Omit `idempotency_key`** — the SDK derives a deterministic UUIDv5 from the provider's quote/swap id. Re-sending the same prepared shape produces the same id and dedupes at the Spark protocol layer even if the first attempt's persistence step never completed.

### Sends with a token leg

When the first leg is a token transfer at the Spark protocol layer, there is no upstream idempotency hook. The dispatcher rejects a caller-supplied `idempotency_key` with `SdkError.INVALID_INPUT`, and a retry can fire a second token transfer and overpay.

This arises in two ways for a cross-chain send:

- **Direct token send** — USDB source on Orchestra. The first leg is a USDB transfer to the provider deposit address.
- **Token conversion** — USDB balance routed through a sats-only provider (e.g. Boltz). The SDK auto-converts USDB → BTC via the [stable-balance](./stable_balance.md) flow before the provider leg; that conversion is itself a token transfer.

This matches the existing contract for direct token sends.

If you need at-most-once semantics in either of these cases, debounce retries at the application layer until the SDK either returns a payment or a terminal error.

## Limitations

- **Mainnet only.** Cross-chain providers operate against live external networks; there is no testnet equivalent in the SDK today.
- **Background tasks required.** Both providers depend on background monitors to reconcile delivery status. `cross_chain_config` is incompatible with `background_tasks_enabled` disabled.
- **Token-leg sends have no idempotency guarantee.** Applies to a direct USDB send and to any USDB-funded send that auto-converts through bitcoin. See [Retry safety](#retry-safety) above.

## Supported chains

| Chain family | Asset | Chains |
| ------------ | ----- | ------ |
| EVM    | USDC | Arbitrum One, Avalanche, Base, BSC, Codex, Ethereum, HyperEVM, Ink, Linea, Monad, Optimism, Plume, Polygon PoS, Sei, Sonic, Tempo, Unichain, World Chain, XDC |
| EVM    | USDT | Arbitrum One, Berachain, BSC, Conflux eSpace, Corn, Ethereum, Flare, Hedera, HyperEVM, Ink, Mantle, MegaETH, Monad, Morph, Optimism, Plasma, Polygon PoS, Rootstock, Sei, Stable, Tempo, Unichain, XLayer |
| Solana | USDC | |
| Solana | USDT | |
| Tron   | USDT | |
