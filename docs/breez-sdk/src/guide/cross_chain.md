# Send USDC/USDT

The SDK can send USDC or USDT from a Spark wallet to a recipient on any of several supported chains: Ethereum-family chains (Arbitrum, Base, and similar EVM networks), Solana, and Tron. The source on the Spark side is either BTC sats or USDB. The SDK orchestrates two legs — a Spark-side transfer to a provider-controlled deposit and the provider-driven delivery of the destination asset — and reconciles both onto a single {{#name Payment}} row.

The send flow itself lives in the [Sending payments](./send_payment.md#send-usdc-usdt) page. This page covers how it works under the hood: the providers, the lifecycle, retry semantics, and limitations.

## Supported address formats

{{#name parse}} recognizes cross-chain destinations in the following forms, returning {{#enum InputType::CrossChainAddress}} with the parsed {{#name CrossChainAddressDetails}} — address family, bare address, and optional token contract address, chain id, and amount.

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

| Provider     | Source assets    | Destinations                                                | Mechanism                            |
| ------------ | ---------------- | ----------------------------------------------------------- | ------------------------------------ |
| **Orchestra** (Flashnet) | BTC sats + USDB | USDC / USDT on Ethereum chains (Arbitrum, Base), Solana, Tron | Spark transfer to a deposit address, then provider bridges to the destination chain |
| **Boltz**    | BTC sats only    | USDC / USDT on Ethereum chains (Arbitrum, Base), Solana, Tron                 | Lightning reverse swap: SDK pays a hold invoice, provider claims the on-chain leg |

The provider tag on each {{#name CrossChainRoutePair}} is the source of truth. When the same destination is offered by multiple providers, both routes are returned; the caller picks one based on supported source assets, fees, or other preferences.

## Slippage

Cross-chain slippage protects the recipient from price movement between quote and delivery. Values are expressed in basis points (1 bps = 0.01%).

Resolution at prepare time:

1. The per-request {{#name max_slippage_bps}} on {{#enum PaymentRequest::CrossChain}} wins if set.
2. Otherwise, the SDK falls back to {{#name default_slippage_bps}} on {{#name CrossChainConfig}} from [the SDK configuration](./config.md#cross-chain-payments).
3. Otherwise, the built-in default of 100 bps (1%) is used.

Values outside 10 to 500 are rejected at both config validation and per-request validation.

## Quote expiry

Each cross-chain prepare response carries an {{#name expires_at}} quote-expiry timestamp on {{#enum SendPaymentMethod::CrossChainAddress}}. If the quote has expired by the time you call {{#name send_payment}}, you must re-prepare to obtain a fresh quote (with a new {{#name expires_at}}) and try again.

## Status lifecycle

The Spark/USDB token transfer and the cross-chain delivery have distinct status fields. They are tracked separately on the persisted {{#name Payment}} row so each can settle independently.

| Field                                                 | Reflects                                                       |
| ----------------------------------------------------- | -------------------------------------------------------------- |
| {{#name status}}                                      | The Spark or USDB token transfer (sender-side settlement)      |
| {{#name conversion_info.status}}                      | The provider-driven cross-chain leg                            |
| {{#name conversion_info.delivered_amount}}            | Final amount delivered to the recipient, set when terminal     |

The cross-chain status walks one of:

- **{{#enum ConversionStatus::Pending}}** — deposit transfer submitted, provider working on the cross-chain leg.
- **{{#enum ConversionStatus::Completed}}** — provider reports the order terminal-successful; {{#name delivered_amount}} is set.
- **{{#enum ConversionStatus::RefundNeeded}}** — provider rejected the submit or order failed before delivery; the local Spark transfer is settled and the deposit is sitting at the provider awaiting refund.
- **{{#enum ConversionStatus::Refunded}}** — the funds have been refunded back to the wallet.
- **{{#enum ConversionStatus::Failed}}** — terminal failure with no refund pending.

A background monitor runs while the SDK is active and reconciles {{#enum ConversionStatus::RefundNeeded}} and {{#enum ConversionStatus::Pending}} rows onto their terminal state by polling the provider.

<h2 id="retry-safety">Retry safety</h2>

Calling {{#name send_payment}} is safe to retry on transient errors, **with one caveat that depends on the source asset.**

### BTC-source sends

For sends where the source is BTC sats (Boltz reverse swap, or Orchestra with BTC source), the SDK threads a deterministic transfer id through to the underlying Spark transfer. Retrying with the same {{#name PrepareSendPaymentResponse}} produces the same transfer id, and the Spark protocol returns the original transfer instead of firing a new one — no double-deposit.

Two ways to drive idempotency:

1. **Pass a caller-supplied {{#name idempotency_key}}** on {{#name SendPaymentRequest}}. The top-level dispatcher first looks for an existing payment with that id and short-circuits the retry if found; otherwise the key is used as the Spark transfer id.
2. **Omit {{#name idempotency_key}}** — the SDK derives a deterministic UUIDv5 from the provider's quote/swap id. Re-sending the same prepared shape produces the same id and dedupes at the Spark protocol layer even if the first attempt's persistence step never completed.

### USDB-source sends

For sends where the source is USDB (Orchestra), the underlying token transfer does not accept an idempotency token at the Spark protocol layer. There is no upstream dedup. Callers that retry on transient errors can fire a second deposit and overpay.

This matches the existing contract for direct token sends.

If you need at-most-once semantics for a USDB-source cross-chain send, debounce retries at the application layer until the SDK either returns a payment or a terminal error.

## Limitations

- **Mainnet only.** Cross-chain providers operate against live external networks; there is no testnet equivalent in the SDK today.
- **Background tasks required.** Both providers depend on background monitors to reconcile delivery status. {{#name cross_chain_config}} is incompatible with {{#name background_tasks_enabled}} disabled.
- **USDB-source retries have no idempotency guarantee.** See [Retry safety](#retry-safety) above.

