# Cash App and USDC/USDT

The SDK can bridge between Cash App and USDC/USDT on external chains (Ethereum-family, Solana, or Tron), in either direction:

- [Cash App to USDC/USDT](./cash_app_to_usdc_usdt.md): a Cash App user pays over Lightning, and the recipient gets the stablecoin on an external chain.
- [USDC/USDT to Cash App](./usdc_usdt_to_cash_app.md): a payer deposits the stablecoin on an external chain, and a Cash App user gets Bitcoin.

In both, the SDK only sets the bridge up. No wallet funds move, and nothing is written to the payment history: the payer funds the order outside the wallet and the cross-chain provider delivers it autonomously. There is no {{#name Payment}} row, no status tracking, and no event. Show the quote returned to the user so they know what to expect before they pay.

<div class="warning">
<h4>Developer note</h4>
Cash App is available in the US and UK only (excluding New York State for Bitcoin/Lightning features). Cash App handles region restrictions on their end, so no client-side gating is needed.
</div>

## Selecting a route

Both bridges move over Lightning on the Cash App side. List the routes with {{#name get_cross_chain_routes}}, setting the filter's {{#name delivery_method}} to {{#enum DeliveryMethod::Lightning}}:

- {{#enum CrossChainRouteFilter::Send}} lists the stablecoin destinations Cash App can fund.
- {{#enum CrossChainRouteFilter::Receive}} lists the stablecoin sources that can pay a Cash App user.

Pick the {{#name CrossChainRoutePair}} whose {{#name CrossChainRoutePair.asset}} and {{#name CrossChainRoutePair.chain}} match the stablecoin you want. A route that can't move over Lightning fails fast, before any funds move.

## Amounts and fees

The {{#name amount}} is in the stablecoin's base units, per the route's {{#name CrossChainRoutePair.decimals}}. These routes carry USD-pegged stablecoins, so at parity it is the USD value: `1_000_000` is 1 USDC (6 decimals), about $1.

{{#name fee_policy}} controls who absorbs the provider fee, reusing the same {{#name FeePolicy}} as the send flow:

- {{#enum FeePolicy::FeesExcluded}} (default) delivers about the amount to the recipient and adds the fee on top of what the payer pays.
- {{#enum FeePolicy::FeesIncluded}} takes the fee out of the amount, so the payer pays exactly the amount and the recipient receives less.

The fee includes your [partner fee](./cross_chain_partner_fees.md) if you set one. {{#name max_slippage_bps}} bounds the price movement tolerated between quote and delivery, in basis points (1 bps = 0.01%). Left unset, the SDK default applies.

## Paying Cash App from the wallet

To pay a Cash App user from the wallet's own balance, no bridge is needed. A Cash App username prefixed with `$` (e.g. `$alice`) [parses](./parse.md) to the user's Lightning address, `alice@cash.app`, which you pay with [LNURL-Pay](./lnurl_pay.md). A wallet holding a [stable balance](./stable_balance.md) converts it to Bitcoin for the payment automatically.

## Limitations

- **Mainnet only.** Cash App and the cross-chain providers operate against live networks. There is no testnet equivalent.
- **Not tracked.** The bridge is funded outside the wallet, so it produces no {{#name Payment}} row and no event. If you need delivery confirmation, observe the recipient's side directly.
