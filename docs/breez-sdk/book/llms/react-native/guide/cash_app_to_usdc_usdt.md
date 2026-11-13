# Cash App to USDC/USDT

`bridgeFromCashApp` lets a user send USDC or USDT to a recipient on an external chain. The user pays through Cash App over Lightning, and the cross-chain provider delivers the stablecoin to the recipient. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. Get a [bridging client](./bridging.md#getting-a-bridging-client): build one from your API key with `newBridgingClient`, or call `getBridgingClient` on a connected SDK.
2. [Select a destination route](./cash_app.md#selecting-a-route) for the recipient's external-chain address.
3. Call `bridgeFromCashApp` with the recipient, the route and the USD amount. The SDK creates an order with the cross-chain provider and returns a Cash App deep link plus the order details.
4. Open the returned `paymentRequest`, and store the `bridgeId` and `statusToken` to [track the bridge](#tracking-the-bridge). The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

The SDK builds a `cash.app/launch/lightning/<bolt11>` deep link from the provider's invoice and returns it as the response `paymentRequest`. The invoice itself is `info.depositAddress`.

```typescript
// List the routes to the recipient's external-chain address
// (EVM/Solana/Tron) and pick a stablecoin Cash App can fund over
// Lightning, e.g. USDC on Base.
const recipient = '<recipient address>'
const { routes } = await client.getRoutes({ recipient })
const route = routes.find(
  (r) =>
    r.source.chain === 'lightning' &&
    r.destination.asset === 'USDC' &&
    r.destination.chain === 'base'
)
if (route === undefined) {
  throw new Error('No USDC route on Base')
}

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in
// USDC base units (6 decimals), so 10_000_000 = 10 USDC, about $10.
const response = await client.bridgeFromCashApp({
  recipient,
  route,
  amount: BigInt(10_000_000),
  refundAddress: undefined,
  feePolicy: undefined,
  maxSlippageBps: undefined
})

// Open this Cash App link to pay. The recipient then receives the stablecoin.
const info = response.info
console.log(`Open this URL in Cash App: ${response.paymentRequest}`)
console.log(`Recipient receives ~${info.expectedReceivedAmount} ${info.destinationAsset}`)
```



## Slippage and refunds

The provider allows these payments up to 10% of price movement between the quote and the delivery, whatever `maxSlippageBps` says. With fees excluded, `maxSlippageBps` still bounds how far the quoted delivery may fall short of the `amount`: a bridge quoted below it fails before the user sees a link. With fees included, the quoted delivery isn't checked.

If the payment fails before the swap, the provider refunds it to the [refund address](./bridging.md#refund-address): a Lightning address, Cash App `$cashtag` or amountless BOLT11 invoice. An invoice must not expire before the bridge, and must still be valid when the refund is paid. Without a refund address, the payment can't be refunded automatically.

## Response fields

`bridgeFromCashApp` returns a `CreateBridgeResponse`, described in [Bridging USDC/USDT](./bridging.md#response-fields). Its `paymentRequest` is the Cash App deep link to open.

## Opening the payment link

On devices with Cash App installed the URL opens the app directly. Otherwise it falls back to the Cash App website. The same UX guidance as the [Buying Bitcoin](./buy_bitcoin.md#recommended-ux) Cash App flow applies (mobile redirect, desktop QR code, pre-opening a tab on web to avoid popup blockers).

## Tracking the bridge

Pass the `bridgeId` and `statusToken` from the response to `getBridgeStatus`. The bridge reads:

- `BridgeStatus.Created` until the user pays in Cash App.
- `BridgeStatus.Expired` if nobody pays before its `expiresAt`. The link can't be paid after that, so create a new bridge.
- `BridgeStatus.Pending` once the payment arrives, while the provider swaps and delivers.
- `BridgeStatus.Completed` once the recipient has the stablecoin. `deliveryTxHash` is the delivery transaction on the recipient's chain.
- `BridgeStatus.Refunding`, then `BridgeStatus.Refunded`, if the payment failed before the swap and is returned to the [refund address](#slippage-and-refunds).

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



See [Tracking a bridge](./bridging.md#tracking-a-bridge) for every status.
