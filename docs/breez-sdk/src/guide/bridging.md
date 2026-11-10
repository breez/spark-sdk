# Bridging USDC/USDT

A bridge is an order that pays a recipient from another chain or over Lightning, with no funds moving through the SDK. The payer pays the cross-chain provider, and the provider delivers to the recipient: a Spark address, a Lightning address, or a USDC/USDT address on another chain.

Bridging suits a server that facilitates payments: it needs only your API key, no seed and no storage.

## Getting a bridging client

Build a {{#name BridgingClient}} from your API key with {{#name new_bridging_client}}. Keep it for the life of your process: it caches the provider's routes and prices between calls.

{{#tabs bridging:new-bridging-client}}

An app that already runs the SDK, including in [server mode](./server_mode.md), can get a client from it instead. The client shares the SDK's API key, and its [cross-chain settings](./config.md#usdc-usdt) when they are set. Bridging doesn't need cross-chain payments enabled.

{{#tabs bridging:get-bridging-client}}

## Creating a bridge

1. List the routes that can pay the {{#name recipient}} with {{#name get_routes}}. Each {{#name BridgeRoutePair}} has a {{#name source}}, what the payer pays, and a {{#name destination}}, what the recipient receives. Pick one by the {{#name asset}} and {{#name chain}} of its sides.
2. Call {{#name create_bridge}} with the same recipient, the route, the amount and, when needed, the payer's [refund address](#refund-address).
3. Show the payer the returned {{#name payment_request}}, and store the {{#name bridge_id}} and {{#name status_token}} to look up the status later. The [response](#response-fields) also carries the deposit and the expected delivery.

{{#tabs bridging:create-bridge}}

### Recipients

| Recipient | Payer pays from | Recipient receives |
| --------- | --------------- | ------------------ |
| Spark address | USDC/USDT on another chain | Bitcoin, or a stablecoin token such as USDB |
| Lightning address, including a Cash App `$cashtag` | USDC/USDT on another chain | Bitcoin over Lightning |
| USDC/USDT address on another chain | Lightning, or USDC/USDT on any chain | USDC/USDT on that chain |

Pass the recipient as you have it: a Spark address, a Lightning address or `$cashtag`, or an address or EIP-681 URI on another chain. A recipient that can't be bridged fails with {{#enum SdkError::InvalidInput}}.

Routes move only USD stablecoins and Bitcoin. For a recipient on another chain, the address decides which routes apply: an EVM address is paid on any EVM chain, unless it names a chain or token (as an EIP-681 URI does). That can be hundreds of routes, so filter them by the sources you accept.

### Amount and fees

The {{#name amount}} is in the base units of the stablecoin the recipient receives, per its side's {{#name BridgeRoutePairSide.decimals}}. When the recipient receives Bitcoin, it's in the stablecoin the payer pays instead. These routes carry USD-pegged stablecoins, so at parity it is the USD value: `1_000_000` is 1 USDC (6 decimals), about $1. USDC and USDT on BNB Chain have 18 decimals.

{{#name fee_policy}} decides who absorbs the provider fee. {{#enum FeePolicy::FeesExcluded}} (the default) adds it on top of what the payer pays, and {{#enum FeePolicy::FeesIncluded}} takes it out of the amount. The fee includes your [partner fee](./cross_chain_partner_fees.md) if you set one.

### Refund address

When the payer pays from another chain, {{#name refund_address}} is their address on that chain. If the recipient can't be paid, the provider refunds the payment there. It must belong to the {{#name source}} chain's family, e.g. an EVM `0x...` address for USDC on Base.

When the payer pays over Lightning, it's a Lightning address, Cash App `$cashtag` or amountless BOLT11 invoice, refunded if the payment fails before the swap. An invoice must not expire before the bridge's {{#name expires_at}}, and must still be valid when the refund is paid. Anything else is rejected with {{#enum SdkError::InvalidInput}}.

It's required to pay a Lightning address, and recommended otherwise: without one, the provider has no address to refund to.

### Response fields

{{#name CreateBridgeResponse}} carries what the payer pays, the quote, and what [tracks the bridge](#tracking-a-bridge):

| Field | Meaning |
| ----- | ------- |
| {{#name bridge_id}} | Identifies the bridge. Store it to look its status up. |
| {{#name status_token}} | Pass it to {{#name get_bridge_status}} with the {{#name bridge_id}}. |
| {{#name payment_request}} | What the payer pays: an EIP-681 URI on EVM chains, a BOLT11 invoice over Lightning, otherwise the bare deposit address. |
| {{#name info.deposit_address}} | The deposit address on the source chain, or the BOLT11 invoice over Lightning. |
| {{#name info.deposit_amount}} | The amount the payer pays, in the source asset's base units (sats over Lightning). |
| {{#name info.expected_received_amount}} | The amount the recipient is expected to receive, in {{#name info.destination_asset}} base units (sats for Bitcoin). |
| {{#name info.destination_asset}} | The asset the recipient receives, e.g. `USDC`, `USDB` or `BTC`. |
| {{#name info.token_identifier}} | The Spark token identifier when the recipient receives a Spark token such as USDB. |
| {{#name info.service_fee_amount}} | Provider fee, including your [partner fee](./cross_chain_partner_fees.md) if you set one, in {{#name info.service_fee_asset}} units. |
| {{#name info.service_fee_asset}} | Denomination of the fee. Absent means the fee is in sats. |
| {{#name info.service_fee_asset_decimals}} | Decimals of {{#name info.service_fee_asset}}, for formatting the fee. Absent when the fee is in sats or the provider did not report them. |
| {{#name info.expires_at}} | Quote expiry, in seconds since the Unix epoch. Create the bridge again for a fresh quote if it lapses before the payer pays. |

## Tracking a bridge

Read a bridge's status with {{#name get_bridge_status}}, passing the {{#name bridge_id}} and {{#name status_token}} that {{#name create_bridge}} returned.

{{#tabs bridging:get-bridge-status}}

| Status | Meaning |
| ------ | ------- |
| {{#enum BridgeStatus::Created}} | The bridge was created, and the payer hasn't paid yet. A bridge from Cash App becomes {{#enum BridgeStatus::Expired}} once it can no longer be paid. Other unpaid bridges stay {{#enum BridgeStatus::Created}} after their {{#name expires_at}}, and a late deposit on another chain may still be accepted at a new price. Stop polling when you no longer expect a payment. |
| {{#enum BridgeStatus::Pending}} | The provider is processing the payment, from confirming the deposit to paying the recipient. {{#name provider_status}} gives the stage. |
| {{#enum BridgeStatus::Refunding}} | The recipient can't be paid, and the payment is being refunded. |
| {{#enum BridgeStatus::Completed}} | The recipient was paid. {{#name amount_out}} and {{#name delivery_tx_hash}} record the delivery. |
| {{#enum BridgeStatus::Refunded}} | The payment was refunded to the {{#name refund_address}}. |
| {{#enum BridgeStatus::Failed}} | The bridge failed. {{#name error_code}} gives the reason. |
| {{#enum BridgeStatus::Expired}} | The bridge can no longer be paid, and nobody paid it. |

## Limitations

- **Mainnet only.** The cross-chain provider operates against live networks.
- **Polling only.** Bridges send no events. Look the status up when you need it.
