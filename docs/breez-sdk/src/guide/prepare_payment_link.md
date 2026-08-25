# Cash App to USDC/USDT payments

{{#name prepare_payment_link}} lets a user send USDC or USDT to a recipient on an external chain (Ethereum-family, Solana, or Tron). The user pays through Cash App (over Lightning) and a cross-chain provider delivers the stablecoin to the recipient.

The SDK acts as orchestrator only. No wallet funds move, and nothing is written to the payment history: Cash App funds the purchase externally and the provider delivers autonomously. This is distinct from the wallet-funded, tracked flow on the [Send USDC/USDT](./cross_chain.md) page.

<div class="warning">
<h4>Developer note</h4>
Cash App is available in the US and UK only (excluding New York State for Bitcoin/Lightning features). Cash App handles region restrictions on their end, so no client-side gating is needed.
</div>

## How it works

1. Parse the recipient's external-chain address and select a destination route.
2. Call {{#name prepare_payment_link}} with the route and the USD amount. The SDK gets a quote from the cross-chain provider and returns a Cash App deep link plus the quote details.
3. Open the returned {{#name url}}. The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

Because the SDK never sees the funds, {{#name prepare_payment_link}} returns and forgets: there is no {{#name Payment}} row, no status tracking, and no event. Surface the quote fields in the response so the app can show the user what to expect before they pay.

## Selecting a route

Parse the recipient address, then list the stablecoin destinations you can send to with {{#name get_cross_chain_routes}} filtered by {{#enum CrossChainRouteFilter::PaymentLink}}. These routes are the ones fundable by an external rail (Cash App over Lightning), a different set from the wallet-funded send routes. Pick the {{#name CrossChainRoutePair}} whose {{#name CrossChainRoutePair.asset}} and {{#name CrossChainRoutePair.chain}} match the destination you want, and pass it to {{#name prepare_payment_link}}.

Calling {{#name prepare_payment_link}} with a route that can't be funded over Lightning fails fast, before any funds move.

## Preparing the payment link

Cash App funds the purchase over **Lightning**. The SDK builds a `cash.app/launch/lightning/<bolt11>` deep link from the provider's invoice and returns it as the response {{#name url}}.

{{#tabs prepare_payment_link:prepare-payment-link-cashapp}}

### Amounts and fees

The {{#name amount}} on {{#name PreparePaymentLinkRequest}} is in the destination asset's base units, per the route's {{#name CrossChainRoutePair.decimals}}. These routes deliver USD-pegged stablecoins, so at parity it is the USD value: `1_000_000` is 1 USDC (6 decimals), about $1.

{{#name fee_policy}} controls who absorbs the provider fee, reusing the same {{#name FeePolicy}} as the send flow:

- {{#enum FeePolicy::FeesExcluded}} (default) delivers the target amount to the recipient and adds the fee on top of what the user pays.
- {{#enum FeePolicy::FeesIncluded}} takes the fee out of the amount, so the user pays exactly the amount and the recipient receives less.

{{#name max_slippage_bps}} bounds the price movement tolerated between quote and delivery, in basis points (1 bps = 0.01%). Left unset, the provider default applies.

### Response fields

{{#name PreparePaymentLinkResponse}} carries the payment URL and the quote:

| Field | Meaning |
| ----- | ------- |
| {{#name url}} | The Cash App deep link to open. |
| {{#name amount_sats}} | Bitcoin amount the user deposits through Cash App. |
| {{#name estimated_out}} | Expected amount delivered to the recipient, in {{#name asset}} units. |
| {{#name asset}} | The delivered stablecoin (e.g. `USDC`). |
| {{#name service_fee_amount}} | Provider fee for the conversion. |
| {{#name service_fee_asset}} | Denomination of the fee. Absent means the fee is in sats. |
| {{#name expires_at}} | Quote expiry. Re-call {{#name prepare_payment_link}} for a fresh quote if it lapses before the user starts paying. |

## Opening the payment link

On devices with Cash App installed the URL opens the app directly; otherwise it falls back to the Cash App website. The same UX guidance as the [Buying Bitcoin](./buy_bitcoin.md#recommended-ux) Cash App flow applies (mobile redirect, desktop QR code, pre-opening a tab on web to avoid popup blockers).

## Limitations

- **Mainnet only.** Cash App and the cross-chain providers operate against live networks. There is no testnet equivalent.
- **Not tracked.** The purchase is funded outside the wallet, so it produces no {{#name Payment}} row and no event. If you need delivery confirmation, observe the recipient chain directly.
- **Orchestra routes only.** Payment links quote against Orchestra, whose orders deliver without the SDK online. A Boltz route would need the wallet to stay online to claim the swap before the payer's payment settles, so {{#name get_cross_chain_routes}} with {{#enum CrossChainRouteFilter::PaymentLink}} returns only Orchestra routes and {{#name prepare_payment_link}} rejects a Boltz route.
