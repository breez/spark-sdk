# Cash App to USDC/USDT

`bridgeFromCashApp` lets a user send USDC or USDT to a recipient on an external chain. The user pays through Cash App over Lightning, and the cross-chain provider delivers the stablecoin to the recipient. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. [Select a destination route](./cash_app.md#selecting-a-route) for the recipient's external-chain address.
2. Call `bridgeFromCashApp` with the recipient, the route and the USD amount. The SDK gets a quote from the cross-chain provider and returns a Cash App deep link plus the quote details.
3. Open the returned `paymentRequest`. The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

The SDK builds a `cash.app/launch/lightning/<bolt11>` deep link from the provider's invoice and returns it as the response `paymentRequest`. The invoice itself is `info.depositAddress`.

```dart
// List the routes to the recipient's external-chain address
// (EVM/Solana/Tron) and pick a stablecoin Cash App can fund over
// Lightning, e.g. USDC on Base.
String recipient = "<recipient address>";
final routesResponse = await client.getRoutes(
  request: GetBridgeRoutesRequest(recipient: recipient),
);
BridgeRoutePair route = routesResponse.routes.firstWhere(
  (r) =>
      r.source.chain == "lightning" &&
      r.destination.asset == "USDC" &&
      r.destination.chain == "base",
);

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in
// USDC base units (6 decimals), so 10000000 = 10 USDC, about $10.
final response = await client.bridgeFromCashApp(
  request: CreateBridgeRequest(
    recipient: recipient,
    route: route,
    amount: BigInt.from(10000000),
    refundAddress: null,
    feePolicy: null,
    maxSlippageBps: null,
  ),
);

// Open this Cash App link to pay. The recipient then receives the stablecoin.
final info = response.info;
print("Open this URL in Cash App: ${response.paymentRequest}");
print(
  "Recipient receives ~${info.expectedReceivedAmount} ${info.destinationAsset}",
);
```



## Response fields

`bridgeFromCashApp` returns a `CreateBridgeResponse`, described in [Bridging USDC/USDT](./bridging.md#response-fields). Its `paymentRequest` is the Cash App deep link to open.

## Opening the payment link

On devices with Cash App installed the URL opens the app directly. Otherwise it falls back to the Cash App website. The same UX guidance as the [Buying Bitcoin](./buy_bitcoin.md#recommended-ux) Cash App flow applies (mobile redirect, desktop QR code, pre-opening a tab on web to avoid popup blockers).
