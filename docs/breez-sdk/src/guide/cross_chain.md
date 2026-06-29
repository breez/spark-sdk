# USDC/USDT payments

The SDK can send and receive USDC and USDT between a Spark wallet and several supported external chains: Ethereum-family chains (Arbitrum, Base, and similar EVM networks), Solana, and Tron. The Spark side is either BTC sats or USDB. The SDK orchestrates two legs per payment — a Spark-side transfer and the provider-driven external delivery — and reconciles both onto a single {{#name Payment}} row.

The send flow is documented on the [Sending payments](./send_payment.md#usdc-usdt) page; the receive flow on the [Receiving payments](./receive_payment.md#usdc-usdt) page. This page covers shared concepts: providers, lifecycle, retry safety, and limitations.

## Supported address formats

{{#name parse}} recognizes cross-chain destinations in the following forms, returning {{#enum InputType::CrossChainAddress}} with the parsed {{#name CrossChainAddressDetails}} — address family, bare address, and optional token contract address, chain id, and amount.

Parsing is for send only. Receives don't take a counterparty address: the receiver chooses a route and the SDK returns a provider-controlled deposit address for the sender to pay.

### Bare addresses

The SDK detects three address families from format alone. A bare address parses with no `contract_address`, `chain_id`, or `amount` — the caller selects the destination chain and asset via {{#name get_cross_chain_routes}}.

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

The SDK ships with two cross-chain providers. {{#name get_cross_chain_routes}} returns the union of routes offered by each, tagged with {{#name CrossChainRoutePair.provider}}.

| Provider     | Direction        | Spark side       | External side                                                | Mechanism                            |
| ------------ | ---------------- | ---------------- | ------------------------------------------------------------ | ------------------------------------ |
| **Orchestra** (Flashnet) | Send + Receive  | BTC sats + USDB | USDC / USDT on Ethereum chains (Arbitrum, Base), Solana, Tron | Spark transfer to a deposit address, then provider bridges to the destination chain |
| **Boltz**    | Send             | BTC sats only    | USDC / USDT on Ethereum chains (Arbitrum, Base), Solana, Tron                 | Lightning reverse swap: SDK pays a hold invoice, provider claims the on-chain leg |

The provider tag on each {{#name CrossChainRoutePair}} is the source of truth. When the same destination is offered by multiple providers, both routes are returned; the caller picks one based on supported source/destination assets, fees, or other preferences.

## Slippage

Cross-chain slippage protects against price movement between quote and delivery. Values are expressed in basis points (1 bps = 0.01%).

Resolution at prepare/receive time:

1. The per-request {{#name max_slippage_bps}} on the request if set.
2. Otherwise, the SDK falls back to {{#name default_slippage_bps}} on {{#name CrossChainConfig}} from [the SDK configuration](./config.md#send-usdc-usdt).
3. Otherwise, the built-in default of 100 bps (1%) is used.

Values outside 10 to 500 are rejected at both config validation and per-request validation.

## Status lifecycle

The Spark-side transfer and the external cross-chain leg have distinct status fields. They are tracked separately on the persisted {{#name Payment}} row so each can settle independently.

| Field                                                 | Reflects                                                                    |
| ----------------------------------------------------- | --------------------------------------------------------------------------- |
| {{#name status}}                                      | The Spark-side transfer (outbound on send; inbound claim on receive)        |
| {{#name conversion_info.status}}                      | The provider-driven cross-chain leg                                         |
| {{#name conversion_info.delivered_amount}}            | Final amount delivered to the recipient, set when terminal                  |

The cross-chain status walks one of:

- **{{#enum ConversionStatus::Pending}}** — deposit/transfer submitted, provider working on the cross-chain leg.
- **{{#enum ConversionStatus::Completed}}** — provider reports the order terminal-successful; {{#name delivered_amount}} is set.
- **{{#enum ConversionStatus::RefundNeeded}}** — the cross-chain leg was rejected after deposit (typically because the realized rate exceeded {{#name max_slippage_bps}}); the deposit is awaiting refund. *(send only)*
- **{{#enum ConversionStatus::Refunded}}** — the funds have been refunded back to the wallet.
- **{{#enum ConversionStatus::Failed}}** — terminal failure with no refund pending.

A background monitor runs while the SDK is active and reconciles non-terminal payments by polling the provider.

## Send

### Quote expiry

Each cross-chain send prepare response carries an {{#name expires_at}} quote-expiry timestamp on {{#enum SendPaymentMethod::CrossChainAddress}}. If the quote has expired by the time you call {{#name send_payment}}, you must re-prepare to obtain a fresh quote (with a new {{#name expires_at}}) and try again.

<h3 id="retry-safety">Retry safety</h3>

Calling {{#name send_payment}} is safe to retry on transient errors **only when the send has no token-transfer leg.** Whether the source asset displayed on the route is BTC or USDB is not the determinant — what matters is the actual first leg the SDK executes.

#### Sends with no token leg

When the first leg is a Spark sats transfer (Orchestra with BTC source, or Boltz funded directly from the sats balance), the SDK threads a deterministic transfer id through to the underlying Spark transfer. Retrying with the same {{#name PrepareSendPaymentResponse}} produces the same transfer id, and the Spark protocol returns the original transfer instead of firing a new one — no double-deposit.

Two ways to drive idempotency:

1. **Pass a caller-supplied {{#name idempotency_key}}** on {{#name SendPaymentRequest}}. The top-level dispatcher first looks for an existing payment with that id and short-circuits the retry if found; otherwise the key is used as the Spark transfer id.
2. **Omit {{#name idempotency_key}}** — the SDK derives a deterministic UUIDv5 from the provider's quote/swap id. Re-sending the same prepared shape produces the same id and dedupes at the Spark protocol layer even if the first attempt's persistence step never completed.

#### Sends with a token leg

When the first leg is a token transfer at the Spark protocol layer, there is no upstream idempotency hook. The dispatcher rejects a caller-supplied {{#name idempotency_key}} with {{#enum SdkError::InvalidInput}}, and a retry can fire a second token transfer and overpay.

This arises in two ways for a cross-chain send:

- **Direct token send** — USDB source on Orchestra. The first leg is a USDB transfer to the provider deposit address.
- **Token conversion** — USDB balance routed through a sats-only provider (e.g. Boltz). The SDK auto-converts USDB → BTC via the [stable-balance](./stable_balance.md) flow before the provider leg; that conversion is itself a token transfer.

This matches the existing contract for direct token sends.

If you need at-most-once semantics in either of these cases, debounce retries at the application layer until the SDK either returns a payment or a terminal error.

## Receive

The receiver picks a route, chooses the deposit amount in source-asset base units, and gets back a provider-controlled deposit address to share with the sender.

### Destination selection

The {{#name destination}} on {{#enum ReceivePaymentMethod::CrossChain}} is a {{#name SparkAsset}} indicating which Spark-side asset the receiver wants to land. When unset, the SDK auto-picks: the wallet's active stable-balance token if the route supports landing it, otherwise BTC. An explicit choice must appear in the selected {{#name CrossChainRoutePair.spark_assets}}.

### Quote expiry

The receive prepare response carries an {{#name expires_at}} timestamp. The SDK does not gate on it: Orchestra reprices late deposits at the live rate. The receive monitor keeps probing for the deposit for 24 hours past {{#name expires_at}} before locally closing the row as unfunded — late deposits inside that window still link correctly.

## Limitations

- **Mainnet only.** Cross-chain providers operate against live external networks; there is no testnet equivalent in the SDK today.
- **Background tasks required.** Both providers depend on background monitors to reconcile delivery status. {{#name cross_chain_config}} is incompatible with {{#name background_tasks_enabled}} disabled.
- **Token-leg sends have no idempotency guarantee.** Applies to a direct USDB send and to any USDB-funded send that auto-converts through bitcoin. See [Retry safety](#retry-safety) above.
