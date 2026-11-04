# Cash App to USDC/USDT

{{#name bridge_from_cash_app}} lets a user send USDC or USDT to a recipient on an external chain. The user pays through Cash App over Lightning, and the cross-chain provider delivers the stablecoin to the recipient. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. [Select a destination route](./cash_app.md#selecting-a-route) for the recipient's external-chain address.
2. Call {{#name bridge_from_cash_app}} with the recipient, the route and the USD amount. The SDK gets a quote from the cross-chain provider and returns a Cash App deep link plus the quote details.
3. Open the returned {{#name payment_request}}. The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

The SDK builds a `cash.app/launch/lightning/<bolt11>` deep link from the provider's invoice and returns it as the response {{#name payment_request}}. The invoice itself is {{#name info.deposit_address}}.

{{#tabs cash_app:bridge-from-cash-app}}

## Response fields

{{#name bridge_from_cash_app}} returns a {{#name CreateBridgeResponse}}, described in [Bridging USDC/USDT](./bridging.md#response-fields). Its {{#name payment_request}} is the Cash App deep link to open.

## Opening the payment link

On devices with Cash App installed the URL opens the app directly. Otherwise it falls back to the Cash App website. The same UX guidance as the [Buying Bitcoin](./buy_bitcoin.md#recommended-ux) Cash App flow applies (mobile redirect, desktop QR code, pre-opening a tab on web to avoid popup blockers).
