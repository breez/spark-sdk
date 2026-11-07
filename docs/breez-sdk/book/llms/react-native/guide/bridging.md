# Bridging USDC/USDT

A bridge is an order that pays a recipient from another chain or over Lightning, with no funds moving through the SDK. The payer pays the cross-chain provider, and the provider delivers to the recipient: a Spark address, a Lightning address, or a USDC/USDT address on another chain.

Bridging suits a server that facilitates payments: it needs only your API key, no seed and no storage.

## Getting a bridging client

Build a `BridgingClient` from your API key with `newBridgingClient`. Keep it for the life of your process: it caches the provider's routes and prices between calls.

```typescript
const config = defaultBridgingConfig('<your api key>')
const client = await newBridgingClient(config)
```



An app that already runs the SDK, including in [server mode](./server_mode.md), can get a client from it instead. The client shares the SDK's API key, and its [cross-chain settings](./config.md#usdc-usdt) when they are set. Bridging doesn't need cross-chain payments enabled.

```typescript
const client = sdk.getBridgingClient()
```



## Creating a bridge

1. List the routes that can pay the `recipient` with `getRoutes`. Each `BridgeRoutePair` has a `source`, what the payer pays, and a `destination`, what the recipient receives. Pick one by the `asset` and `chain` of its sides.
2. Call `createBridge` with the same recipient, the route, the amount and, when needed, the payer's [refund address](#refund-address).
3. Show the payer the returned `paymentRequest`, and store the `bridgeId` and `statusToken` to look up the status later. The [response](#response-fields) also carries the deposit and the expected delivery.

```typescript
// Who to pay: a Spark address, a Lightning address or an EVM/Solana/Tron
// address.
const recipient = '<recipient>'

// List the routes that pay them and pick what the payer pays in, e.g.
// USDC on Base.
const { routes } = await client.getRoutes({ recipient })
const route = routes.find((r) => r.source.asset === 'USDC' && r.source.chain === 'base')
if (route === undefined) {
  throw new Error('No route from USDC on Base')
}

// Ask for $10. The amount is in the recipient's stablecoin when they are
// paid in one, the payer's otherwise (6 decimals for USDC). The deposit is
// refunded to the payer's address if the recipient can't be paid.
const response = await client.createBridge({
  recipient,
  route,
  amount: BigInt(10_000_000),
  // Required to pay a Lightning address.
  refundAddress: '<payer address on Base>',
  feePolicy: undefined,
  maxSlippageBps: undefined
})

// Show the payer what to pay, and store the bridge id and status token to
// look up the status later.
console.log(`Pay with: ${response.paymentRequest}`)
console.log(`Bridge id: ${response.bridgeId}`)
```



### Recipients

| Recipient | Payer pays from | Recipient receives |
| --------- | --------------- | ------------------ |
| Spark address | USDC/USDT on another chain | Bitcoin, or a stablecoin token such as USDB |
| Lightning address, including a Cash App `$cashtag` | USDC/USDT on another chain | Bitcoin over Lightning |
| USDC/USDT address on another chain | Lightning, or USDC/USDT on any chain | USDC/USDT on that chain |

Pass the recipient as you have it: a Spark address, a Lightning address or `$cashtag`, or an address or EIP-681 URI on another chain. A recipient that can't be bridged fails with `SdkError.InvalidInput`.

Routes move only USD stablecoins and Bitcoin. For a recipient on another chain, the address decides which routes apply: an EVM address is paid on any EVM chain, unless it names a chain or token (as an EIP-681 URI does). That can be hundreds of routes, so filter them by the sources you accept.

### Amount and fees

The `amount` is in the base units of the stablecoin the recipient receives, per its side's `BridgeRoutePairSide.decimals`. When the recipient receives Bitcoin, it's in the stablecoin the payer pays instead. These routes carry USD-pegged stablecoins, so at parity it is the USD value: `1_000_000` is 1 USDC (6 decimals), about $1. USDC and USDT on BNB Chain have 18 decimals.

`feePolicy` decides who absorbs the provider fee. `FeePolicy.FeesExcluded` (the default) adds it on top of what the payer pays, and `FeePolicy.FeesIncluded` takes it out of the amount. The fee includes your [partner fee](./cross_chain_partner_fees.md) if you set one.

### Refund address

When the payer pays from another chain, `refundAddress` is their address on that chain. If the recipient can't be paid, the provider refunds the payment there. It must belong to the `source` chain's family, e.g. an EVM `0x...` address for USDC on Base.

It's required to pay a Lightning address, and recommended whenever the payer pays from another chain: without one, the provider has no address to refund to. A payer paying over Lightning needs none, and any address given is ignored.

### Response fields

`CreateBridgeResponse` carries what the payer pays, the quote, and what [tracks the bridge](#tracking-a-bridge):

| Field | Meaning |
| ----- | ------- |
| `bridgeId` | Identifies the bridge. Store it to look its status up. |
| `statusToken` | Pass it to `getBridgeStatus` with the `bridgeId`. |
| `paymentRequest` | What the payer pays: an EIP-681 URI on EVM chains, a BOLT11 invoice over Lightning, otherwise the bare deposit address. |
| `info.depositAddress` | The deposit address on the source chain, or the BOLT11 invoice over Lightning. |
| `info.depositAmount` | The amount the payer pays, in the source asset's base units (sats over Lightning). |
| `info.expectedReceivedAmount` | The amount the recipient is expected to receive, in `info.destinationAsset` base units (sats for Bitcoin). |
| `info.destinationAsset` | The asset the recipient receives, e.g. `USDC`, `USDB` or `BTC`. |
| `info.tokenIdentifier` | The Spark token identifier when the recipient receives a Spark token such as USDB. |
| `info.serviceFeeAmount` | Provider fee, including your [partner fee](./cross_chain_partner_fees.md) if you set one, in `info.serviceFeeAsset` units. |
| `info.serviceFeeAsset` | Denomination of the fee. Absent means the fee is in sats. |
| `info.serviceFeeAssetDecimals` | Decimals of `info.serviceFeeAsset`, for formatting the fee. Absent when the fee is in sats or the provider did not report them. |
| `info.expiresAt` | Quote expiry, in seconds since the Unix epoch. Create the bridge again for a fresh quote if it lapses before the payer pays. |

## Tracking a bridge

Read a bridge's status with `getBridgeStatus`, passing the `bridgeId` and `statusToken` that `createBridge` returned.

```typescript
const response = await client.getBridgeStatus({ bridgeId, statusToken })
switch (response.status) {
  case BridgeStatus.Completed:
    console.log(`Delivered ${response.amountOut ?? 'unknown'}`)
    break
  case BridgeStatus.Refunded:
  case BridgeStatus.Failed:
  case BridgeStatus.Expired:
    console.log(`Not delivered: ${BridgeStatus[response.status]}`)
    break
  default:
    console.log(`Not final yet: ${BridgeStatus[response.status]}`)
}
```



| Status | Meaning |
| ------ | ------- |
| `BridgeStatus.Created` | The bridge was created, and the payer hasn't paid yet. An unpaid bridge stays `BridgeStatus.Created` after its `expiresAt`. A late deposit on another chain may still be accepted at a new price. Stop polling when you no longer expect a payment. |
| `BridgeStatus.Pending` | The provider is processing the payment, from confirming the deposit to paying the recipient. `providerStatus` gives the stage. |
| `BridgeStatus.Refunding` | The recipient can't be paid, and the payment is being refunded. |
| `BridgeStatus.Completed` | The recipient was paid. `amountOut` and `deliveryTxHash` record the delivery. |
| `BridgeStatus.Refunded` | The payment was refunded to the `refundAddress`. |
| `BridgeStatus.Failed` | The bridge failed. `errorCode` gives the reason. |
| `BridgeStatus.Expired` | The provider's order expired without a payment. |

## Limitations

- **Mainnet only.** The cross-chain provider operates against live networks.
- **Polling only.** Bridges send no events. Look the status up when you need it.
