# Cash App to USDC/USDT

{{#name bridge_from_cash_app}} lets a user send USDC or USDT to a recipient on an external chain. The user pays through Cash App over Lightning, and the cross-chain provider delivers the stablecoin to the recipient. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. [Select a destination route](./cash_app.md#selecting-a-route) for the recipient's external-chain address.
2. Call {{#name bridge_from_cash_app}} with the recipient, the route and the USD amount. The SDK creates an order with the cross-chain provider and returns a Cash App deep link plus the order details.
3. Open the returned {{#name payment_request}}, and store the {{#name bridge_id}} and {{#name status_token}} to [track the bridge](#tracking-the-bridge). The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

The SDK builds a `cash.app/launch/lightning/<bolt11>` deep link from the provider's invoice and returns it as the response {{#name payment_request}}. The invoice itself is {{#name info.deposit_address}}.

{{#tabs cash_app:bridge-from-cash-app}}

## Slippage and refunds

The provider allows these payments up to 10% of price movement between the quote and the delivery, whatever {{#name max_slippage_bps}} says. With fees excluded, {{#name max_slippage_bps}} still bounds how far the quoted delivery may fall short of the {{#name amount}}: a bridge quoted below it fails before the user sees a link. With fees included, the quoted delivery isn't checked.

If the payment fails before the swap, the provider refunds it to the [refund address](./bridging.md#refund-address): a Lightning address, Cash App `$cashtag` or amountless BOLT11 invoice. An invoice must not expire before the bridge, and must still be valid when the refund is paid. Without a refund address, the payment can't be refunded automatically.

## Response fields

{{#name bridge_from_cash_app}} returns a {{#name CreateBridgeResponse}}, described in [Bridging USDC/USDT](./bridging.md#response-fields). Its {{#name payment_request}} is the Cash App deep link to open.

## Opening the payment link

On devices with Cash App installed the URL opens the app directly. Otherwise it falls back to the Cash App website. The same UX guidance as the [Buying Bitcoin](./buy_bitcoin.md#recommended-ux) Cash App flow applies (mobile redirect, desktop QR code, pre-opening a tab on web to avoid popup blockers).

## Tracking the bridge

Pass the {{#name bridge_id}} and {{#name status_token}} from the response to {{#name get_bridge_status}}. The bridge reads:

- {{#enum BridgeStatus::Created}} until the user pays in Cash App.
- {{#enum BridgeStatus::Expired}} if nobody pays before its {{#name expires_at}}. The link can't be paid after that, so create a new bridge.
- {{#enum BridgeStatus::Pending}} once the payment arrives, while the provider swaps and delivers.
- {{#enum BridgeStatus::Completed}} once the recipient has the stablecoin. {{#name delivery_tx_hash}} is the delivery transaction on the recipient's chain.
- {{#enum BridgeStatus::Refunding}}, then {{#enum BridgeStatus::Refunded}}, if the payment failed before the swap and is returned to the [refund address](#slippage-and-refunds).

{{#tabs bridging:get-bridge-status}}

See [Tracking a bridge](./bridging.md#tracking-a-bridge) for every status.
