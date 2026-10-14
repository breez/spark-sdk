# Cash App to USDC/USDT

{{#name bridge_from_cash_app}} lets a user send USDC or USDT to a recipient on an external chain. The user pays through Cash App over Lightning, and the cross-chain provider delivers the stablecoin to the recipient. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. Parse the recipient's external-chain address and [select a destination route](./cash_app.md#selecting-a-route) with {{#enum CrossChainRouteFilter::Send}}.
2. Call {{#name bridge_from_cash_app}} with the route and the USD amount. The SDK gets a quote from the cross-chain provider and returns a Cash App deep link plus the quote details.
3. Open the returned {{#name url}}. The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

The SDK builds a `cash.app/launch/lightning/<bolt11>` deep link from the provider's invoice and returns it as the response {{#name url}}.

{{#tabs cash_app:bridge-from-cash-app}}

## Response fields

{{#name BridgeFromCashAppResponse}} carries the payment URL and the quote:

| Field | Meaning |
| ----- | ------- |
| {{#name url}} | The Cash App deep link to open. |
| {{#name amount_sats}} | Bitcoin amount the user deposits through Cash App. |
| {{#name estimated_out}} | Expected amount delivered to the recipient, in {{#name asset}} units. |
| {{#name asset}} | The delivered stablecoin (e.g. `USDC`). |
| {{#name service_fee_amount}} | Provider fee for the conversion, including your [partner fee](./cross_chain_partner_fees.md) if you set one. |
| {{#name service_fee_asset}} | Denomination of the fee. Absent means the fee is in sats. |
| {{#name service_fee_asset_decimals}} | Decimals of {{#name service_fee_asset}}, for formatting the fee. Absent when the fee is in sats or the provider did not report them. |
| {{#name expires_at}} | Quote expiry, in seconds since the Unix epoch. Call {{#name bridge_from_cash_app}} again for a fresh quote if it lapses before the user starts paying. |

## Opening the payment link

On devices with Cash App installed the URL opens the app directly. Otherwise it falls back to the Cash App website. The same UX guidance as the [Buying Bitcoin](./buy_bitcoin.md#recommended-ux) Cash App flow applies (mobile redirect, desktop QR code, pre-opening a tab on web to avoid popup blockers).
