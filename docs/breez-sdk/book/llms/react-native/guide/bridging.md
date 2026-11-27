# Bridging USDC/USDT

A bridge is an order that pays a recipient from another chain or over Lightning, with no funds moving through the SDK. The payer pays the cross-chain provider, and the provider delivers to the recipient: a Spark address, a Lightning address, or a USDC/USDT address on another chain. A bridge paid over Lightning can also deliver a chain's [native coin, or Bitcoin on-chain](#native-coins-and-bitcoin).

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



When a Cash App user pays, use [`bridgeFromCashApp`](#from-cash-app) instead.

### Recipients

| Recipient | Payer pays from | Recipient receives |
| --------- | --------------- | ------------------ |
| Spark address | USDC/USDT on another chain | Bitcoin, or a stablecoin token such as USDB |
| Lightning address, including a Cash App `$cashtag` | USDC/USDT on another chain | Bitcoin over Lightning |
| USDC/USDT address on another chain | Lightning, or USDC/USDT on any chain | USDC/USDT on that chain |
| Address on another chain | Lightning | That chain's [native coin](#native-coins-and-bitcoin) |
| Bitcoin address | Lightning | Bitcoin on-chain |

Pass the recipient as you have it: a Spark address, a Lightning address or `$cashtag`, a Bitcoin address or BIP21 URI, or an address or EIP-681 URI on another chain. A recipient that can't be bridged fails with `SdkError.InvalidInput`.

Routes move USD stablecoins and Bitcoin. A payer on Lightning can also pay a [native coin](#native-coins-and-bitcoin). For a recipient on another chain, the address decides which routes apply: an EVM address is paid on any EVM chain, unless it names a chain or token (as an EIP-681 URI does). That can be hundreds of routes, so filter them by the sources you accept and by the `destination` `asset`.

### Amount and fees

The `amount` is in the base units of the side the route's `amountSide` names, per that side's `BridgeRoutePairSide.decimals`:

- `BridgeAmountSide.Destination`: the stablecoin the recipient receives.
- `BridgeAmountSide.Source`: what the payer pays. That's the stablecoin they deposit when the recipient receives Bitcoin on Spark or Lightning, and sats when they pay over Lightning for a native coin or Bitcoin on-chain.

A stablecoin amount at parity is the USD value: `1_000_000` is 1 USDC (6 decimals), about $1. USDC and USDT on BNB Chain have 18 decimals.

`feePolicy` decides who absorbs the provider fee. `FeePolicy.FeesExcluded` (the default) adds it on top of what the payer pays, and `FeePolicy.FeesIncluded` takes it out of the amount. The fee includes your [partner fee](./cross_chain_partner_fees.md) if you set one.

With `FeePolicy.FeesExcluded`, the payer also pays a small [overpay](./config.md#usdc-usdt), which the recipient receives.

### Refund address

When the payer pays from another chain, `refundAddress` is their address on that chain. If the recipient can't be paid, the provider refunds the payment there. It must belong to the `source` chain's family, e.g. an EVM `0x...` address for USDC on Base.

When the payer pays over Lightning, it's a Lightning address, Cash App `$cashtag` or amountless BOLT11 invoice, refunded if the payment fails before the swap. An invoice must not expire before the bridge's `expiresAt`, and must still be valid when the refund is paid. Anything else is rejected with `SdkError.InvalidInput`.

It's required to pay a Lightning address, and recommended otherwise. Without one, the provider refunds a deposit from another chain to the address it came from, but only when that is safe, and a payment over Lightning can't be refunded automatically.

**Developer note**

A refund address is required when the payer sends from an exchange. Exchanges send from shared addresses, which the provider never refunds to, so without one a deposit it can't deliver waits for manual recovery. It is also advised for deposits from BSC and Tron.

### Response fields

`CreateBridgeResponse` carries what the payer pays, the quote, and what [tracks the bridge](#tracking-a-bridge):

| Field | Meaning |
| ----- | ------- |
| `bridgeId` | Identifies the bridge. Store it to look its status up. |
| `statusToken` | Pass it to `getBridgeStatus` with the `bridgeId`. |
| `paymentRequest` | What the payer pays: an EIP-681 URI on EVM chains, a BOLT11 invoice over Lightning ([a Cash App link](#from-cash-app) from `bridgeFromCashApp`), otherwise the bare deposit address. |
| `info.depositAddress` | The deposit address on the source chain, or the BOLT11 invoice over Lightning. |
| `info.depositAmount` | The amount the payer pays, in the source asset's base units (sats over Lightning). |
| `info.expectedReceivedAmount` | The amount the recipient is expected to receive, in `info.destinationAsset` base units (sats for Bitcoin, wei for ETH). |
| `info.destinationAsset` | The asset the recipient receives, e.g. `USDC`, `USDB`, `BTC` or `ETH`. |
| `info.tokenIdentifier` | The Spark token identifier when the recipient receives a Spark token such as USDB. |
| `info.serviceFeeAmount` | Provider fee, including your [partner fee](./cross_chain_partner_fees.md) if you set one, in `info.serviceFeeAsset` units. |
| `info.serviceFeeAsset` | Denomination of the fee. Absent means the fee is in sats. |
| `info.serviceFeeAssetDecimals` | Decimals of `info.serviceFeeAsset`, for formatting the fee. Absent when the fee is in sats or the provider did not report them. |
| `info.expiresAt` | Quote expiry, in seconds since the Unix epoch. Create the bridge again for a fresh quote if it lapses before the payer pays. |

A Solana transfer built in code must also create the deposit address's token account, which wallet apps do on their own. See [Paying from Solana](./cross_chain.md#paying-from-solana).

## Native coins and Bitcoin

A bridge paid over Lightning can also deliver a chain's native coin, or Bitcoin on-chain. The provider decides which coins and chains are available, and `getRoutes` lists only those.

Their routes have `lightning` as the `source` `chain`, and a `destination` with no `contractAddress`. They are created like any other bridge, with these differences:

- The `amount` is in sats: the route's `amountSide` is `BridgeAmountSide.Source`. With `FeePolicy.FeesExcluded` the fee is added on top, and with `FeePolicy.FeesIncluded` the payer pays exactly the amount.
- `info.expectedReceivedAmount` is in the coin's base units: wei for ETH, lamports for SOL, sats for Bitcoin.
- An EVM recipient gets routes to the native coin of every EVM chain, listed after the stablecoin routes from the same source. Pick by the `destination` `asset` and `chain`.

| Coin | Recipient |
| ---- | --------- |
| ETH, BNB, POL, MON, HYPE | EVM address, or an EIP-681 URI naming the chain |
| SOL | Solana address |
| TRX | Tron address |
| ZEC | Zcash transparent address (`t1...` or `t3...`) |
| Bitcoin on-chain | Bitcoin address or BIP21 URI |

**Developer note**

- **Zcash:** only transparent addresses can be paid. Shielded, unified and TEX addresses are rejected.
- **Bitcoin on-chain:** the minimum moves with the on-chain exit fee. The route's `minAmount` is the minimum that must reach the recipient when the routes were listed, so with `FeePolicy.FeesIncluded` the amount must also cover the fees. Below the current minimum, `createBridge` fails with `SdkError.CrossChainAmountOutOfRange`, whose message names it. If the exit fee rises while a bridge with `FeePolicy.FeesExcluded` is created, the recipient can receive up to `maxSlippageBps` less, and a larger rise fails with `SdkError.InvalidInput`: create it again. `info.serviceFeeAmount` includes the exit fee. No overpay is added, as the cost is known exactly. The provider takes no [partner fee](./cross_chain_partner_fees.md) on these bridges.

## Cash App

Two functions create a bridge with a Cash App user on one side. They take the same `CreateBridgeRequest` as `createBridge`, return a `CreateBridgeResponse`, and are [tracked](#tracking-a-bridge) the same way.

### From Cash App

When a Cash App user pays a USDC/USDT recipient, or a [native coin or Bitcoin on-chain](#native-coins-and-bitcoin), call `bridgeFromCashApp` with a route whose `source` is on `lightning`. `createBridge` doesn't do this for you: on the same route it returns a plain BOLT11 invoice. `bridgeFromCashApp` differs in three ways:

- Its `paymentRequest` is a `cash.app` link that opens Cash App on the invoice.
- The provider's order exists from the start. The bridge can be paid until its `expiresAt`, then becomes `BridgeStatus.Expired`.
- The provider allows up to 10% of price movement, whatever `maxSlippageBps` says. See [Slippage and refunds](./cash_app_to_usdc_usdt.md#slippage-and-refunds).

See [Cash App to USDC/USDT](./cash_app_to_usdc_usdt.md) for a walkthrough.

### To Cash App

To pay a Cash App user from USDC/USDT on another chain, call `bridgeToCashApp`. It creates the same bridge as `createBridge` with the user's `$cashtag`, and fails with `SdkError.InvalidInput` for a recipient who isn't a Cash App user. As for any Lightning address, a [refund address](#refund-address) is required.

See [USDC/USDT to Cash App](./usdc_usdt_to_cash_app.md) for a walkthrough.

## Tracking a bridge

Read a bridge's status with `getBridgeStatus`, passing the `bridgeId` and `statusToken` that the bridge was created with.

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
| `BridgeStatus.Created` | The bridge was created, and the provider hasn't registered a payment yet. A payment over Lightning can take several minutes to register. A bridge from Cash App becomes `BridgeStatus.Expired` once it can no longer be paid. Other unpaid bridges stay `BridgeStatus.Created` after their `expiresAt`, and a late deposit on another chain may still be accepted at a new price. Stop polling when you no longer expect a payment. |
| `BridgeStatus.Pending` | The provider is processing the payment, from confirming the deposit to paying the recipient. `providerStatus` gives the stage. |
| `BridgeStatus.Refunding` | The recipient can't be paid, and the payment is being refunded. |
| `BridgeStatus.Completed` | The recipient was paid. `amountOut` and `deliveryTxHash` record the delivery. |
| `BridgeStatus.Refunded` | The payment was refunded to the `refundAddress`, or without one to the address it came from. |
| `BridgeStatus.Failed` | The bridge failed. `errorCode` gives the reason. |
| `BridgeStatus.Expired` | The bridge can no longer be paid, and nobody paid it. |

Once the payment arrives, the response also carries `orderId`, the provider's id for the order. If a paid bridge stays `BridgeStatus.Pending` or fails without a refund, see [USDC/USDT orders](./reporting_an_issue.md#usdc-usdt-orders).

## Limitations

- **Mainnet only.** The cross-chain provider operates against live networks.
- **Polling only.** Bridges send no events. Look the status up when you need it.
