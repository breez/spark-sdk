# USDC/USDT to Cash App

{{#name bridge_to_cash_app}} lets someone holding USDC or USDT on an external chain pay a Cash App user. The payer deposits the stablecoin, and the cross-chain provider pays the recipient in Bitcoin over Lightning. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. Get a [bridging client](./bridging.md#getting-a-bridging-client): build one from your API key with {{#name new_bridging_client}}, or call {{#name get_bridging_client}} on a connected SDK.
2. [Select a source route](./cash_app.md#selecting-a-route) for the Cash App user.
3. Call {{#name bridge_to_cash_app}} with the recipient, the route, the USD amount and the payer's refund address. The SDK checks the recipient and gets a quote from the cross-chain provider.
4. Show the payer the returned {{#name payment_request}}, and store the {{#name bridge_id}} and {{#name status_token}} to [track the bridge](#tracking-the-bridge). Once their deposit lands, the provider pays the recipient.

{{#tabs cash_app:bridge-to-cash-app}}

## Recipient

The {{#name recipient}} is a Cash App username, given as `$alice`, or the user's Cash App Lightning address, `alice@cash.app`. Both reach the same user, who is paid at that Lightning address. {{#name bridge_to_cash_app}} also takes the bare `alice`, but {{#name get_routes}} needs the `$`.

{{#name get_routes}} looks the user up, so an unknown username fails before anything is quoted. {{#name bridge_to_cash_app}} fails with {{#enum SdkError::InvalidInput}} for a recipient who isn't a Cash App user, or an amount outside what the recipient accepts.

## Refund address

The {{#name refund_address}} is the payer's own address on the source chain. If the recipient can't be paid, the provider refunds the deposit there. It must belong to the {{#name source}} chain's family (e.g. an EVM `0x...` address for USDC on Base).

## Response fields

{{#name bridge_to_cash_app}} returns a {{#name CreateBridgeResponse}}, described in [Bridging USDC/USDT](./bridging.md#response-fields). Its {{#name payment_request}} is what the payer pays: an EIP-681 URI on EVM chains, otherwise the bare deposit address.

## Tracking the bridge

Pass the {{#name bridge_id}} and {{#name status_token}} from the response to {{#name get_bridge_status}}. The bridge reads:

- {{#enum BridgeStatus::Created}} until the provider sees the deposit. It stays {{#enum BridgeStatus::Created}} after its {{#name expires_at}}, because a late deposit may still be accepted at a new price. Stop polling when you no longer expect one.
- {{#enum BridgeStatus::Pending}} from the deposit to the Lightning payment. {{#name deposit_tx_hash}} is the payer's transaction.
- {{#enum BridgeStatus::Completed}} once the Lightning payment reached the recipient. {{#name amount_out}} is in sats. {{#name delivery_tx_hash}} is the provider's reference for the payment, not a Lightning payment hash.
- {{#enum BridgeStatus::Refunding}}, then {{#enum BridgeStatus::Refunded}}, if the recipient couldn't be paid (see [Limitations](#limitations)) and the deposit is returned to the {{#name refund_address}}.

{{#tabs bridging:get-bridge-status}}

See [Tracking a bridge](./bridging.md#tracking-a-bridge) for every status.

## Limitations

- **Delivery can still fail after funding.** The provider looks the recipient up again when it pays. If the account was closed, its limit lowered, or slippage moved the amount across a limit since the quote, the payment fails and the deposit is refunded to {{#name refund_address}}.
- **Lightning delivery only.** A completed bridge means the Lightning payment was delivered. Whether Cash App credits it as Bitcoin or converts it to dollars depends on the recipient's Cash App settings.
