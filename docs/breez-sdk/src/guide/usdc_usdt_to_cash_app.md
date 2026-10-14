# USDC/USDT to Cash App

{{#name bridge_to_cash_app}} lets someone holding USDC or USDT on an external chain pay a Cash App user. The payer deposits the stablecoin, and the cross-chain provider pays the recipient in Bitcoin over Lightning. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. [Select a source route](./cash_app.md#selecting-a-route) with {{#enum CrossChainRouteFilter::Receive}}.
2. Call {{#name bridge_to_cash_app}} with the recipient, the route, the USD amount and the payer's refund address. The SDK checks the recipient and gets a quote from the cross-chain provider.
3. Show the payer the returned {{#name payment_request}}. Once their deposit lands, the provider pays the recipient.

{{#tabs cash_app:bridge-to-cash-app}}

## Recipient

The {{#name recipient}} is a Cash App username, given as `alice` or `$alice`, or the user's Cash App Lightning address, `alice@cash.app`. All three reach the same user, who is paid at that Lightning address.

The SDK looks the recipient up before quoting, so an unknown username fails with {{#enum SdkError::InvalidInput}} before the payer funds anything. So does an amount outside what the recipient accepts.

## Refund address

The {{#name refund_address}} is the payer's own address on the source chain. If the recipient can't be paid, the provider refunds the deposit there. It must belong to the route's chain family (e.g. an EVM `0x...` address for USDC on Base).

## Response fields

{{#name BridgeToCashAppResponse}} carries what the payer pays and the quote:

| Field | Meaning |
| ----- | ------- |
| {{#name payment_request}} | What the payer pays: an EIP-681 URI on EVM chains, otherwise the bare deposit address. |
| {{#name info.deposit_address}} | The deposit address on the source chain. |
| {{#name info.deposit_amount}} | The amount the payer deposits, in the route asset's base units. |
| {{#name info.expected_received_amount}} | The sats the recipient is expected to receive. |
| {{#name info.service_fee_amount}} | Provider fee, including your [partner fee](./cross_chain_partner_fees.md) if you set one, in {{#name info.service_fee_asset}} units. |
| {{#name info.expires_at}} | Quote expiry, in seconds since the Unix epoch. Call {{#name bridge_to_cash_app}} again for a fresh quote if it lapses before the payer deposits. |

## Limitations

- **Delivery can still fail after funding.** The provider looks the recipient up again when it pays. If the account was closed, its limit lowered, or slippage moved the amount across a limit since the quote, the payment fails and the deposit is refunded to {{#name refund_address}}.
- **Lightning delivery only.** A completed bridge means the Lightning payment was delivered. Whether Cash App credits it as Bitcoin or converts it to dollars depends on the recipient's Cash App settings.
