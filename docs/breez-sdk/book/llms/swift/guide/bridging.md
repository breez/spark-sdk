# Bridging USDC/USDT

A bridge is an order that pays a recipient from another chain or over Lightning, with no funds moving through the SDK. The payer pays the cross-chain provider, and the provider delivers to the recipient: a Spark address, a Lightning address, or a USDC/USDT address on another chain.

Bridging suits a server that facilitates payments: it needs only your API key, no seed and no storage.

## Getting a bridging client

Build a `BridgingClient` from your API key with `newBridgingClient`. Keep it for the life of your process: it caches the provider's routes and prices between calls.

```swift
let config = defaultBridgingConfig(apiKey: "<your api key>")
let client = try await newBridgingClient(config: config)
```



An app that already runs the SDK, including in [server mode](./server_mode.md), can get a client from it instead. The client shares the SDK's API key, and its [cross-chain settings](./config.md#usdc-usdt) when they are set. Bridging doesn't need cross-chain payments enabled.

```swift
let client = try sdk.getBridgingClient()
```



## Creating a bridge

1. List the routes that can pay the `recipient` with `getRoutes`. Each `BridgeRoutePair` has a `source`, what the payer pays, and a `destination`, what the recipient receives. Pick one by the `asset` and `chain` of its sides.
2. Call `createBridge` with the same recipient, the route, the amount and, when needed, the payer's [refund address](#refund-address).
3. Show the payer the returned `paymentRequest`, and store the `bridgeId` and `statusToken` to look up the status later. The [response](#response-fields) also carries the deposit and the expected delivery.

```swift
// Who to pay: a Spark address, a Lightning address or an EVM/Solana/Tron
// address.
let recipient = "<recipient>"

// List the routes that pay them and pick what the payer pays in, e.g.
// USDC on Base.
let routes = try await client.getRoutes(
    request: GetBridgeRoutesRequest(recipient: recipient)
).routes
guard
    let route = routes.first(where: {
        $0.source.asset == "USDC" && $0.source.chain == "base"
    })
else {
    throw NSError(domain: "Bridging", code: 1)
}

// Ask for $10. The amount is in the recipient's stablecoin when they are
// paid in one, the payer's otherwise (6 decimals for USDC). The deposit is
// refunded to the payer's address if the recipient can't be paid.
let response = try await client.createBridge(
    request: CreateBridgeRequest(
        recipient: recipient,
        route: route,
        amount: BInt(10_000_000),
        // Required to pay a Lightning address.
        refundAddress: "<payer address on Base>",
        feePolicy: nil,
        maxSlippageBps: nil
    ))

// Show the payer what to pay, and store the bridge id and status token to
// look up the status later.
print("Pay with: \(response.paymentRequest)")
print("Bridge id: \(response.bridgeId)")
```



When a Cash App user pays, use [`bridgeFromCashApp`](#from-cash-app) instead.

### Recipients

| Recipient | Payer pays from | Recipient receives |
| --------- | --------------- | ------------------ |
| Spark address | USDC/USDT on another chain | Bitcoin, or a stablecoin token such as USDB |
| Lightning address, including a Cash App `$cashtag` | USDC/USDT on another chain | Bitcoin over Lightning |
| USDC/USDT address on another chain | Lightning, or USDC/USDT on any chain | USDC/USDT on that chain |

Pass the recipient as you have it: a Spark address, a Lightning address or `$cashtag`, or an address or EIP-681 URI on another chain. A recipient that can't be bridged fails with `SdkError.invalidInput`.

Routes move only USD stablecoins and Bitcoin. For a recipient on another chain, the address decides which routes apply: an EVM address is paid on any EVM chain, unless it names a chain or token (as an EIP-681 URI does). That can be hundreds of routes, so filter them by the sources you accept.

### Amount and fees

The `amount` is in the base units of the stablecoin the recipient receives, per its side's `BridgeRoutePairSide.decimals`. When the recipient receives Bitcoin, it's in the stablecoin the payer pays instead. These routes carry USD-pegged stablecoins, so at parity it is the USD value: `1_000_000` is 1 USDC (6 decimals), about $1. USDC and USDT on BNB Chain have 18 decimals.

`feePolicy` decides who absorbs the provider fee. `FeePolicy.feesExcluded` (the default) adds it on top of what the payer pays, and `FeePolicy.feesIncluded` takes it out of the amount. The fee includes your [partner fee](./cross_chain_partner_fees.md) if you set one.

### Refund address

When the payer pays from another chain, `refundAddress` is their address on that chain. If the recipient can't be paid, the provider refunds the payment there. It must belong to the `source` chain's family, e.g. an EVM `0x...` address for USDC on Base.

When the payer pays over Lightning, it's a Lightning address, Cash App `$cashtag` or amountless BOLT11 invoice, refunded if the payment fails before the swap. An invoice must not expire before the bridge's `expiresAt`, and must still be valid when the refund is paid. Anything else is rejected with `SdkError.invalidInput`.

It's required to pay a Lightning address, and recommended otherwise: without one, the provider has no address to refund to.

### Response fields

`CreateBridgeResponse` carries what the payer pays, the quote, and what [tracks the bridge](#tracking-a-bridge):

| Field | Meaning |
| ----- | ------- |
| `bridgeId` | Identifies the bridge. Store it to look its status up. |
| `statusToken` | Pass it to `getBridgeStatus` with the `bridgeId`. |
| `paymentRequest` | What the payer pays: an EIP-681 URI on EVM chains, a BOLT11 invoice over Lightning ([a Cash App link](#from-cash-app) from `bridgeFromCashApp`), otherwise the bare deposit address. |
| `info.depositAddress` | The deposit address on the source chain, or the BOLT11 invoice over Lightning. |
| `info.depositAmount` | The amount the payer pays, in the source asset's base units (sats over Lightning). |
| `info.expectedReceivedAmount` | The amount the recipient is expected to receive, in `info.destinationAsset` base units (sats for Bitcoin). |
| `info.destinationAsset` | The asset the recipient receives, e.g. `USDC`, `USDB` or `BTC`. |
| `info.tokenIdentifier` | The Spark token identifier when the recipient receives a Spark token such as USDB. |
| `info.serviceFeeAmount` | Provider fee, including your [partner fee](./cross_chain_partner_fees.md) if you set one, in `info.serviceFeeAsset` units. |
| `info.serviceFeeAsset` | Denomination of the fee. Absent means the fee is in sats. |
| `info.serviceFeeAssetDecimals` | Decimals of `info.serviceFeeAsset`, for formatting the fee. Absent when the fee is in sats or the provider did not report them. |
| `info.expiresAt` | Quote expiry, in seconds since the Unix epoch. Create the bridge again for a fresh quote if it lapses before the payer pays. |

A Solana transfer built in code must also create the deposit address's token account, which wallet apps do on their own. See [Paying from Solana](./cross_chain.md#paying-from-solana).

## Cash App

Two functions create a bridge with a Cash App user on one side. They take the same `CreateBridgeRequest` as `createBridge`, return a `CreateBridgeResponse`, and are [tracked](#tracking-a-bridge) the same way.

### From Cash App

When a Cash App user pays a USDC/USDT recipient, call `bridgeFromCashApp` with a route whose `source` is on `lightning`. `createBridge` doesn't do this for you: on the same route it returns a plain BOLT11 invoice. `bridgeFromCashApp` differs in three ways:

- Its `paymentRequest` is a `cash.app` link that opens Cash App on the invoice.
- The provider's order exists from the start. The bridge can be paid until its `expiresAt`, then becomes `BridgeStatus.expired`.
- The provider allows up to 10% of price movement, whatever `maxSlippageBps` says. See [Slippage and refunds](./cash_app_to_usdc_usdt.md#slippage-and-refunds).

See [Cash App to USDC/USDT](./cash_app_to_usdc_usdt.md) for a walkthrough.

### To Cash App

To pay a Cash App user from USDC/USDT on another chain, call `bridgeToCashApp`. It creates the same bridge as `createBridge` with the user's `$cashtag`, and fails with `SdkError.invalidInput` for a recipient who isn't a Cash App user. As for any Lightning address, a [refund address](#refund-address) is required.

See [USDC/USDT to Cash App](./usdc_usdt_to_cash_app.md) for a walkthrough.

## Tracking a bridge

Read a bridge's status with `getBridgeStatus`, passing the `bridgeId` and `statusToken` that the bridge was created with.

```swift
let response = try await client.getBridgeStatus(
    request: GetBridgeStatusRequest(bridgeId: bridgeId, statusToken: statusToken))
switch response.status {
case .completed:
    print("Delivered \(response.amountOut.map { "\($0)" } ?? "unknown")")
case .refunded, .failed, .expired:
    print("Not delivered: \(response.status)")
default:
    print("Not final yet: \(response.status)")
}
```



| Status | Meaning |
| ------ | ------- |
| `BridgeStatus.created` | The bridge was created, and the payer hasn't paid yet. A bridge from Cash App becomes `BridgeStatus.expired` once it can no longer be paid. Other unpaid bridges stay `BridgeStatus.created` after their `expiresAt`, and a late deposit on another chain may still be accepted at a new price. Stop polling when you no longer expect a payment. |
| `BridgeStatus.pending` | The provider is processing the payment, from confirming the deposit to paying the recipient. `providerStatus` gives the stage. |
| `BridgeStatus.refunding` | The recipient can't be paid, and the payment is being refunded. |
| `BridgeStatus.completed` | The recipient was paid. `amountOut` and `deliveryTxHash` record the delivery. |
| `BridgeStatus.refunded` | The payment was refunded to the `refundAddress`. |
| `BridgeStatus.failed` | The bridge failed. `errorCode` gives the reason. |
| `BridgeStatus.expired` | The bridge can no longer be paid, and nobody paid it. |

## Limitations

- **Mainnet only.** The cross-chain provider operates against live networks.
- **Polling only.** Bridges send no events. Look the status up when you need it.
