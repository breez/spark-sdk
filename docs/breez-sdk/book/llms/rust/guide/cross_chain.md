# USDC/USDT payments

The SDK can send and receive USDC and USDT between a Spark wallet and several supported external chains: Ethereum-family chains (Arbitrum, Base, and similar EVM networks), Solana, and Tron. The Spark side is either BTC sats or [USDB](./stable_balance.md), the 6-decimal USD-pegged token on Spark. Each payment runs as two legs (a Spark-side transfer and the provider-driven external delivery) reconciled onto a single `Payment` row.

The send flow is documented on the [Sending payments](./send_payment.md#usdc-usdt) page; the receive flow on the [Receiving payments](./receive_payment.md#usdc-usdt) page. This page covers shared concepts: providers, lifecycle, retry safety, and limitations.

## Supported address formats

`parse` recognizes cross-chain destinations in the following forms, returning `InputType::CrossChainAddress` with the parsed `CrossChainAddressDetails` — address family, bare address, and optional token contract address, chain id, and amount.

Parsing is for send only. Receives don't take a counterparty address: the receiver chooses a route and the SDK returns a provider-controlled deposit address for the sender to pay.

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

`get_cross_chain_routes` returns the routes offered by each provider, tagged with `CrossChainRoutePair.provider`.

| Provider     | Direction        | Spark side       | External side                                                | Mechanism                            |
| ------------ | ---------------- | ---------------- | ------------------------------------------------------------ | ------------------------------------ |
| **Orchestra** (Flashnet) | Send + Receive  | BTC sats + USDB | USDC / USDT on Ethereum chains (Arbitrum, Base), Solana, Tron | Spark transfer to a deposit address, then provider bridges to the destination chain |

The provider tag on each `CrossChainRoutePair` is the source of truth. When the same destination is offered by multiple providers, every route is returned; the caller picks one based on supported source/destination assets, fees, or other preferences.

## Slippage

Cross-chain slippage protects against price movement between quote and delivery. Values are expressed in basis points (1 bps = 0.01%).

Resolution at prepare/receive time:

1. The per-request `max_slippage_bps` on the request if set.
2. Otherwise, the SDK falls back to `default_slippage_bps` on `CrossChainConfig` from [the SDK configuration](./config.md#usdc-usdt).
3. Otherwise, the built-in default of 100 bps (1%) is used.

Values outside 10 to 500 are rejected at both config validation and per-request validation.

## Target overpay

On `CrossChainFeeMode::FeesExcluded` sends and receives the SDK pads the user's target amount by `target_overpay_bps` so the realized delivery lands at or above target despite provider slippage.

Resolution at prepare/receive time:

1. The per-request `target_overpay_bps` on the request if set.
2. Otherwise, the SDK falls back to `default_target_overpay_bps` on `CrossChainConfig`.
3. Otherwise, the built-in default of 15 bps is used.

Values outside 0 to 500 are rejected at both config validation and per-request validation. `CrossChainFeeMode::FeesIncluded` requests ignore this field.

## Status lifecycle

The Spark-side transfer and the external cross-chain leg have distinct status fields. They are tracked separately on the persisted `Payment` row so each can settle independently.

| Field                                                 | Reflects                                                                    |
| ----------------------------------------------------- | --------------------------------------------------------------------------- |
| `status`                                      | The Spark-side transfer (outbound on send; inbound claim on receive)        |
| `conversion_info.status`                      | The provider-driven cross-chain leg                                         |
| `conversion_info.delivered_amount`            | Final amount delivered to the recipient, set when terminal                  |
| `conversion_info.external_tx_hash`            | Transaction on the non-Spark chain: the delivery, or the funding deposit    |

The cross-chain status walks one of:

- **`ConversionStatus::Pending`**: deposit/transfer submitted, provider working on the cross-chain leg.
- **`ConversionStatus::Completed`**: provider reports the order terminal-successful; `delivered_amount` is set, and `external_tx_hash` identifies the transaction on the non-Spark chain: the delivery on a send, the funding deposit on a receive. Boltz-provider conversions leave the hash unset.
- **`ConversionStatus::RefundNeeded`**: the cross-chain leg was rejected after deposit (typically because the realized rate exceeded `max_slippage_bps`); the deposit is awaiting refund. *(send only)*
- **`ConversionStatus::Refunded`**: the funds have been refunded back to the wallet.
- **`ConversionStatus::Failed`**: terminal failure with no refund pending.

A background monitor runs while the SDK is active and reconciles non-terminal payments by polling the provider.

## Send

### Quote expiry

Each cross-chain send prepare response carries an `expires_at` quote-expiry timestamp on `SendPaymentMethod::CrossChainAddress`. If the quote has expired by the time you call `send_payment`, you must re-prepare to obtain a fresh quote (with a new `expires_at`) and try again.

### Retry safety

Calling `send_payment` is safe to retry on transient errors **only when the send has no token-transfer leg.** Whether the source asset displayed on the route is BTC or USDB is not the determinant — what matters is the actual first leg the SDK executes.

#### Sends with no token leg

When the first leg is a Spark sats transfer (Orchestra with a BTC source), the SDK threads a deterministic transfer id through to the underlying Spark transfer. Retrying with the same `PrepareSendPaymentResponse` produces the same transfer id, and the Spark protocol returns the original transfer instead of firing a new one — no double-deposit.

Two ways to drive idempotency:

1. **Pass a caller-supplied `idempotency_key`** on `SendPaymentRequest`. The top-level dispatcher first looks for an existing payment with that id and short-circuits the retry if found; otherwise the key is used as the Spark transfer id.
2. **Omit `idempotency_key`** — the SDK derives a deterministic UUIDv5 from the provider's quote/swap id. Re-sending the same prepared shape produces the same id and dedupes at the Spark protocol layer even if the first attempt's persistence step never completed.

#### Sends with a token leg

When the first leg is a token transfer at the Spark protocol layer, there is no upstream idempotency hook. The dispatcher rejects a caller-supplied `idempotency_key` with `SdkError::InvalidInput`, and a retry can fire a second token transfer and overpay.

This arises in two ways for a cross-chain send:

- **Direct token send** — USDB source on Orchestra. The first leg is a USDB transfer to the provider deposit address.
- **Token conversion** — USDB balance routed over a route whose Spark side takes only sats. The SDK auto-converts USDB → BTC via the [stable-balance](./stable_balance.md) flow before the provider leg; that conversion is itself a token transfer.

This matches the existing contract for direct token sends.

If you need at-most-once semantics in either of these cases, debounce retries at the application layer until the SDK either returns a payment or a terminal error.

## Receive

The receiver picks a route, sets an `amount` in the source asset's base units (`route.decimals`; USD-stable parity, so `1_000_000` is one USD on any 6-decimal route), and gets back a provider-controlled deposit address to share with the sender. See [Receive payment](./receive_payment.md#usdc-usdt) for the full request shape and fee-mode semantics.

### Destination selection

The `destination` on `ReceivePaymentMethod::CrossChain` is a `SparkAsset` indicating which Spark-side asset the receiver wants to land. When unset, the SDK auto-picks: the wallet's active stable-balance token if the route supports landing it, otherwise BTC. An explicit choice must appear in the selected `CrossChainRoutePair.accepted_assets`.

To force a specific destination, build a `SparkAsset` variant: `SparkAsset::Bitcoin` for BTC, or `SparkAsset::Token` carrying the Spark `token_identifier` (the `btkn1...` bech32m id from `CrossChainRoutePair.accepted_assets`) for USDB or any other supported token.

### Quote expiry

The receive prepare response carries an `expires_at` timestamp. The SDK does not gate on it: Orchestra reprices late deposits at the live rate. The receive monitor keeps probing for the deposit for 24 hours past `expires_at` before locally closing the row as unfunded; late deposits inside that window still link correctly.

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
