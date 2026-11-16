# Cash App to USDC/USDT

`BridgeFromCashApp` lets a user send USDC or USDT to a recipient on an external chain. The user pays through Cash App over Lightning, and the cross-chain provider delivers the stablecoin to the recipient. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. Get a [bridging client](./bridging.md#getting-a-bridging-client): build one from your API key with `NewBridgingClient`, or call `GetBridgingClient` on a connected SDK.
2. [Select a destination route](./cash_app.md#selecting-a-route) for the recipient's external-chain address.
3. Call `BridgeFromCashApp` with the recipient, the route and the USD amount. The SDK creates an order with the cross-chain provider and returns a Cash App deep link plus the order details.
4. Open the returned `PaymentRequest`, and store the `BridgeId` and `StatusToken` to [track the bridge](#tracking-the-bridge). The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

The SDK builds a `cash.app/launch/lightning/<bolt11>` deep link from the provider's invoice and returns it as the response `PaymentRequest`. The invoice itself is `Info.DepositAddress`.

```csharp
// List the routes to the recipient's external-chain address
// (EVM/Solana/Tron) and pick a stablecoin Cash App can fund over
// Lightning, e.g. USDC on Base.
var recipient = "<recipient address>";
var routes = (await client.GetRoutes(
    request: new GetBridgeRoutesRequest(recipient: recipient)
)).routes;
var route = routes.First(
    r => r.source.chain == "lightning"
        && r.destination.asset == "USDC"
        && r.destination.chain == "base"
);

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in
// USDC base units (6 decimals), so 10_000_000 = 10 USDC, about $10.
var request = new CreateBridgeRequest(
    recipient: recipient,
    route: route,
    amount: new BigInteger(10_000_000),
    refundAddress: null,
    feePolicy: null,
    maxSlippageBps: null
);
var response = await client.BridgeFromCashApp(request: request);

// Open this Cash App link to pay. The recipient then receives the
// stablecoin.
var info = response.info;
Console.WriteLine($"Open this URL in Cash App: {response.paymentRequest}");
Console.WriteLine(
    $"Recipient receives ~{info.expectedReceivedAmount} {info.destinationAsset}"
);
```



## Slippage and refunds

The provider allows these payments up to 10% of price movement between the quote and the delivery, whatever `MaxSlippageBps` says. With fees excluded, `MaxSlippageBps` still bounds how far the quoted delivery may fall short of the `Amount`: a bridge quoted below it fails before the user sees a link. With fees included, the quoted delivery isn't checked.

If the payment fails before the swap, the provider refunds it to the [refund address](./bridging.md#refund-address): a Lightning address, Cash App `$cashtag` or amountless BOLT11 invoice. An invoice must not expire before the bridge, and must still be valid when the refund is paid. Without a refund address, the payment can't be refunded automatically.

## Response fields

`BridgeFromCashApp` returns a `CreateBridgeResponse`, described in [Bridging USDC/USDT](./bridging.md#response-fields). Its `PaymentRequest` is the Cash App deep link to open.

## Opening the payment link

On devices with Cash App installed the URL opens the app directly. Otherwise it falls back to the Cash App website. The same UX guidance as the [Buying Bitcoin](./buy_bitcoin.md#recommended-ux) Cash App flow applies (mobile redirect, desktop QR code, pre-opening a tab on web to avoid popup blockers).

## Tracking the bridge

Pass the `BridgeId` and `StatusToken` from the response to `GetBridgeStatus`. The bridge reads:

- `BridgeStatus.Created` until the user pays in Cash App.
- `BridgeStatus.Expired` if nobody pays before its `ExpiresAt`. The link can't be paid after that, so create a new bridge.
- `BridgeStatus.Pending` once the payment arrives, while the provider swaps and delivers.
- `BridgeStatus.Completed` once the recipient has the stablecoin. `DeliveryTxHash` is the delivery transaction on the recipient's chain.
- `BridgeStatus.Refunding`, then `BridgeStatus.Refunded`, if the payment failed before the swap and is returned to the [refund address](#slippage-and-refunds).

```csharp
var response = await client.GetBridgeStatus(
    request: new GetBridgeStatusRequest(bridgeId: bridgeId, statusToken: statusToken)
);
switch (response.status)
{
    case BridgeStatus.Completed:
        Console.WriteLine($"Delivered {response.amountOut}");
        break;
    case BridgeStatus.Refunded:
    case BridgeStatus.Failed:
    case BridgeStatus.Expired:
        Console.WriteLine($"Not delivered: {response.status}");
        break;
    default:
        Console.WriteLine($"Not final yet: {response.status}");
        break;
}
```



See [Tracking a bridge](./bridging.md#tracking-a-bridge) for every status.
