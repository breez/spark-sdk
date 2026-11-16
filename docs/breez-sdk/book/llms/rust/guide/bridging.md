# Bridging USDC/USDT

A bridge is an order that pays a recipient from another chain or over Lightning, with no funds moving through the SDK. The payer pays the cross-chain provider, and the provider delivers to the recipient: a Spark address, a Lightning address, or a USDC/USDT address on another chain.

Bridging suits a server that facilitates payments: it needs only your API key, no seed and no storage.

## Getting a bridging client

Build a `BridgingClient` from your API key with `new_bridging_client`. Keep it for the life of your process: it caches the provider's routes and prices between calls.

```rust
let config = default_bridging_config("<your api key>".to_string());
let client = new_bridging_client(config).await?;
```



An app that already runs the SDK, including in [server mode](./server_mode.md), can get a client from it instead. The client shares the SDK's API key, and its [cross-chain settings](./config.md#usdc-usdt) when they are set. Bridging doesn't need cross-chain payments enabled.

```rust
let client = sdk.get_bridging_client()?;
```



## Creating a bridge

1. List the routes that can pay the `recipient` with `get_routes`. Each `BridgeRoutePair` has a `source`, what the payer pays, and a `destination`, what the recipient receives. Pick one by the `asset` and `chain` of its sides.
2. Call `create_bridge` with the same recipient, the route, the amount and, when needed, the payer's [refund address](#refund-address).
3. Show the payer the returned `payment_request`, and store the `bridge_id` and `status_token` to look up the status later. The [response](#response-fields) also carries the deposit and the expected delivery.

```rust
// Who to pay: a Spark address, a Lightning address or an EVM/Solana/Tron
// address.
let recipient = "<recipient>".to_string();

// List the routes that pay them and pick what the payer pays in, e.g.
// USDC on Base.
let routes = client
    .get_routes(GetBridgeRoutesRequest {
        recipient: recipient.clone(),
    })
    .await?
    .routes;
let route = routes
    .into_iter()
    .find(|r| r.source.asset == "USDC" && r.source.chain == "base")
    .ok_or_else(|| anyhow::anyhow!("No route from USDC on Base"))?;

// Ask for $10. The amount is in the recipient's stablecoin when they are
// paid in one, the payer's otherwise (6 decimals for USDC). The deposit is
// refunded to the payer's address if the recipient can't be paid.
let response = client
    .create_bridge(CreateBridgeRequest {
        recipient,
        route,
        amount: 10_000_000,
        // Required to pay a Lightning address.
        refund_address: Some("<payer address on Base>".to_string()),
        fee_policy: None,
        max_slippage_bps: None,
    })
    .await?;

// Show the payer what to pay, and store the bridge id and status token to
// look up the status later.
info!("Pay with: {}", response.payment_request);
info!("Bridge id: {}", response.bridge_id);
```



When a Cash App user pays, use [`bridge_from_cash_app`](#from-cash-app) instead.

### Recipients

| Recipient | Payer pays from | Recipient receives |
| --------- | --------------- | ------------------ |
| Spark address | USDC/USDT on another chain | Bitcoin, or a stablecoin token such as USDB |
| Lightning address, including a Cash App `$cashtag` | USDC/USDT on another chain | Bitcoin over Lightning |
| USDC/USDT address on another chain | Lightning, or USDC/USDT on any chain | USDC/USDT on that chain |

Pass the recipient as you have it: a Spark address, a Lightning address or `$cashtag`, or an address or EIP-681 URI on another chain. A recipient that can't be bridged fails with `SdkError::InvalidInput`.

Routes move only USD stablecoins and Bitcoin. For a recipient on another chain, the address decides which routes apply: an EVM address is paid on any EVM chain, unless it names a chain or token (as an EIP-681 URI does). That can be hundreds of routes, so filter them by the sources you accept.

### Amount and fees

The `amount` is in the base units of the stablecoin the recipient receives, per its side's `BridgeRoutePairSide.decimals`. When the recipient receives Bitcoin, it's in the stablecoin the payer pays instead. These routes carry USD-pegged stablecoins, so at parity it is the USD value: `1_000_000` is 1 USDC (6 decimals), about $1. USDC and USDT on BNB Chain have 18 decimals.

`fee_policy` decides who absorbs the provider fee. `FeePolicy::FeesExcluded` (the default) adds it on top of what the payer pays, and `FeePolicy::FeesIncluded` takes it out of the amount. The fee includes your [partner fee](./cross_chain_partner_fees.md) if you set one.

### Refund address

When the payer pays from another chain, `refund_address` is their address on that chain. If the recipient can't be paid, the provider refunds the payment there. It must belong to the `source` chain's family, e.g. an EVM `0x...` address for USDC on Base.

When the payer pays over Lightning, it's a Lightning address, Cash App `$cashtag` or amountless BOLT11 invoice, refunded if the payment fails before the swap. An invoice must not expire before the bridge's `expires_at`, and must still be valid when the refund is paid. Anything else is rejected with `SdkError::InvalidInput`.

It's required to pay a Lightning address, and recommended otherwise: without one, the provider has no address to refund to.

### Response fields

`CreateBridgeResponse` carries what the payer pays, the quote, and what [tracks the bridge](#tracking-a-bridge):

| Field | Meaning |
| ----- | ------- |
| `bridge_id` | Identifies the bridge. Store it to look its status up. |
| `status_token` | Pass it to `get_bridge_status` with the `bridge_id`. |
| `payment_request` | What the payer pays: an EIP-681 URI on EVM chains, a BOLT11 invoice over Lightning ([a Cash App link](#from-cash-app) from `bridge_from_cash_app`), otherwise the bare deposit address. |
| `info.deposit_address` | The deposit address on the source chain, or the BOLT11 invoice over Lightning. |
| `info.deposit_amount` | The amount the payer pays, in the source asset's base units (sats over Lightning). |
| `info.expected_received_amount` | The amount the recipient is expected to receive, in `info.destination_asset` base units (sats for Bitcoin). |
| `info.destination_asset` | The asset the recipient receives, e.g. `USDC`, `USDB` or `BTC`. |
| `info.token_identifier` | The Spark token identifier when the recipient receives a Spark token such as USDB. |
| `info.service_fee_amount` | Provider fee, including your [partner fee](./cross_chain_partner_fees.md) if you set one, in `info.service_fee_asset` units. |
| `info.service_fee_asset` | Denomination of the fee. Absent means the fee is in sats. |
| `info.service_fee_asset_decimals` | Decimals of `info.service_fee_asset`, for formatting the fee. Absent when the fee is in sats or the provider did not report them. |
| `info.expires_at` | Quote expiry, in seconds since the Unix epoch. Create the bridge again for a fresh quote if it lapses before the payer pays. |

A Solana transfer built in code must also create the deposit address's token account, which wallet apps do on their own. See [Paying from Solana](./cross_chain.md#paying-from-solana).

## Cash App

Two functions create a bridge with a Cash App user on one side. They take the same `CreateBridgeRequest` as `create_bridge`, return a `CreateBridgeResponse`, and are [tracked](#tracking-a-bridge) the same way.

### From Cash App

When a Cash App user pays a USDC/USDT recipient, call `bridge_from_cash_app` with a route whose `source` is on `lightning`. `create_bridge` doesn't do this for you: on the same route it returns a plain BOLT11 invoice. `bridge_from_cash_app` differs in three ways:

- Its `payment_request` is a `cash.app` link that opens Cash App on the invoice.
- The provider's order exists from the start. The bridge can be paid until its `expires_at`, then becomes `BridgeStatus::Expired`.
- The provider allows up to 10% of price movement, whatever `max_slippage_bps` says. See [Slippage and refunds](./cash_app_to_usdc_usdt.md#slippage-and-refunds).

See [Cash App to USDC/USDT](./cash_app_to_usdc_usdt.md) for a walkthrough.

### To Cash App

To pay a Cash App user from USDC/USDT on another chain, call `bridge_to_cash_app`. It creates the same bridge as `create_bridge` with the user's `$cashtag`, and fails with `SdkError::InvalidInput` for a recipient who isn't a Cash App user. As for any Lightning address, a [refund address](#refund-address) is required.

See [USDC/USDT to Cash App](./usdc_usdt_to_cash_app.md) for a walkthrough.

## Tracking a bridge

Read a bridge's status with `get_bridge_status`, passing the `bridge_id` and `status_token` that the bridge was created with.

```rust
let response = client
    .get_bridge_status(GetBridgeStatusRequest {
        bridge_id,
        status_token,
    })
    .await?;
match response.status {
    BridgeStatus::Completed => info!("Delivered {:?}", response.amount_out),
    BridgeStatus::Refunded | BridgeStatus::Failed | BridgeStatus::Expired => {
        info!("Not delivered: {:?}", response.status)
    }
    _ => info!("Not final yet: {:?}", response.status),
}
```



| Status | Meaning |
| ------ | ------- |
| `BridgeStatus::Created` | The bridge was created, and the payer hasn't paid yet. A bridge from Cash App becomes `BridgeStatus::Expired` once it can no longer be paid. Other unpaid bridges stay `BridgeStatus::Created` after their `expires_at`, and a late deposit on another chain may still be accepted at a new price. Stop polling when you no longer expect a payment. |
| `BridgeStatus::Pending` | The provider is processing the payment, from confirming the deposit to paying the recipient. `provider_status` gives the stage. |
| `BridgeStatus::Refunding` | The recipient can't be paid, and the payment is being refunded. |
| `BridgeStatus::Completed` | The recipient was paid. `amount_out` and `delivery_tx_hash` record the delivery. |
| `BridgeStatus::Refunded` | The payment was refunded to the `refund_address`. |
| `BridgeStatus::Failed` | The bridge failed. `error_code` gives the reason. |
| `BridgeStatus::Expired` | The bridge can no longer be paid, and nobody paid it. |

## Limitations

- **Mainnet only.** The cross-chain provider operates against live networks.
- **Polling only.** Bridges send no events. Look the status up when you need it.
