# Cash App to USDC/USDT

`bridge_from_cash_app` lets a user send USDC or USDT to a recipient on an external chain. The user pays through Cash App over Lightning, and the cross-chain provider delivers the stablecoin to the recipient. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. [Select a destination route](./cash_app.md#selecting-a-route) for the recipient's external-chain address.
2. Call `bridge_from_cash_app` with the recipient, the route and the USD amount. The SDK gets a quote from the cross-chain provider and returns a Cash App deep link plus the quote details.
3. Open the returned `payment_request`. The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

The SDK builds a `cash.app/launch/lightning/<bolt11>` deep link from the provider's invoice and returns it as the response `payment_request`. The invoice itself is `info.deposit_address`.

### Rust

```rust
// List the routes to the recipient's external-chain address
// (EVM/Solana/Tron) and pick a stablecoin Cash App can fund over
// Lightning, e.g. USDC on Base.
let recipient = "<recipient address>".to_string();
let routes = client
    .get_routes(GetBridgeRoutesRequest {
        recipient: recipient.clone(),
    })
    .await?
    .routes;
let route = routes
    .into_iter()
    .find(|r| {
        r.source.chain == "lightning"
            && r.destination.asset == "USDC"
            && r.destination.chain == "base"
    })
    .ok_or_else(|| anyhow::anyhow!("No USDC route on Base"))?;

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in
// USDC base units (6 decimals), so 10_000_000 = 10 USDC, about $10.
let response = client
    .bridge_from_cash_app(CreateBridgeRequest {
        recipient,
        route,
        amount: 10_000_000,
        refund_address: None,
        fee_policy: None,
        max_slippage_bps: None,
    })
    .await?;

// Open this Cash App link to pay. The recipient then receives the stablecoin.
let info = response.info;
info!("Open this URL in Cash App: {}", response.payment_request);
info!(
    "Recipient receives ~{} {}",
    info.expected_received_amount, info.destination_asset
);
```

### Swift

```swift
// List the routes to the recipient's external-chain address
// (EVM/Solana/Tron) and pick a stablecoin Cash App can fund over
// Lightning, e.g. USDC on Base.
let recipient = "<recipient address>"
let routes = try await client.getRoutes(
    request: GetBridgeRoutesRequest(recipient: recipient)
).routes
guard
    let route = routes.first(where: {
        $0.source.chain == "lightning"
            && $0.destination.asset == "USDC"
            && $0.destination.chain == "base"
    })
else {
    throw NSError(domain: "CashApp", code: 1)
}

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in
// USDC base units (6 decimals), so 10_000_000 = 10 USDC, about $10.
let response = try await client.bridgeFromCashApp(
    request: CreateBridgeRequest(
        recipient: recipient,
        route: route,
        amount: BInt(10_000_000),
        refundAddress: nil,
        feePolicy: nil,
        maxSlippageBps: nil
    ))

// Open this Cash App link to pay. The recipient then receives the stablecoin.
let info = response.info
print("Open this URL in Cash App: \(response.paymentRequest)")
print("Recipient receives ~\(info.expectedReceivedAmount) \(info.destinationAsset)")
```

### Kotlin

```kotlin
// List the routes to the recipient's external-chain address
// (EVM/Solana/Tron) and pick a stablecoin Cash App can fund over
// Lightning, e.g. USDC on Base.
val recipient = "<recipient address>"
val routes = client.getRoutes(GetBridgeRoutesRequest(recipient = recipient)).routes
val route = routes.find {
    it.source.chain == "lightning" &&
        it.destination.asset == "USDC" &&
        it.destination.chain == "base"
} ?: throw IllegalArgumentException("No USDC route on Base")

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in
// USDC base units (6 decimals), so 10_000_000 = 10 USDC, about $10.
val response = client.bridgeFromCashApp(
    CreateBridgeRequest(
        recipient = recipient,
        route = route,
        amount = BigInteger.fromLong(10_000_000L),
        refundAddress = null,
        feePolicy = null,
        maxSlippageBps = null,
    )
)

// Open this Cash App link to pay. The recipient then receives the stablecoin.
val info = response.info
// Log.v("Breez", "Open this URL in Cash App: ${response.paymentRequest}")
// Log.v("Breez", "Recipient receives ~${info.expectedReceivedAmount} " +
//     "${info.destinationAsset}")
```

### C#

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

### Javascript (Wasm)

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

### React Native

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

### Flutter

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

### Python

```python
try:
    # List the routes to the recipient's external-chain address
    # (EVM/Solana/Tron) and pick a stablecoin Cash App can fund over
    # Lightning, e.g. USDC on Base.
    recipient = "<recipient address>"
    routes_response = await client.get_routes(
        request=GetBridgeRoutesRequest(recipient=recipient)
    )
    route = next(
        (
            r
            for r in routes_response.routes
            if r.source.chain == "lightning"
            and r.destination.asset == "USDC"
            and r.destination.chain == "base"
        ),
        None,
    )
    if route is None:
        raise ValueError("No USDC route on Base")

    # Send $10 of USDC, funded by Cash App over Lightning. The amount is in
    # USDC base units (6 decimals), so 10_000_000 = 10 USDC, about $10.
    request = CreateBridgeRequest(
        recipient=recipient,
        route=route,
        amount=10_000_000,
        refund_address=None,
        fee_policy=None,
        max_slippage_bps=None,
    )
    response = await client.bridge_from_cash_app(request=request)

    # Open this Cash App link to pay. The recipient then receives the
    # stablecoin.
    info = response.info
    logging.debug(f"Open this URL in Cash App: {response.payment_request}")
    logging.debug(
        f"Recipient receives ~{info.expected_received_amount} "
        f"{info.destination_asset}"
    )
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
// List the routes to the recipient's external-chain address
// (EVM/Solana/Tron) and pick a stablecoin Cash App can fund over
// Lightning, e.g. USDC on Base.
recipient := "<recipient address>"
routesResponse, err := client.GetRoutes(breez_sdk_spark.GetBridgeRoutesRequest{
	Recipient: recipient,
})
if err != nil {
	return nil, err
}
routes := routesResponse.Routes
var route *breez_sdk_spark.BridgeRoutePair
for i := range routes {
	if routes[i].Source.Chain == "lightning" &&
		routes[i].Destination.Asset == "USDC" &&
		routes[i].Destination.Chain == "base" {
		route = &routes[i]
		break
	}
}
if route == nil {
	return nil, errors.New("no USDC route on Base")
}

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in
// USDC base units (6 decimals), so 10_000_000 = 10 USDC, about $10.
response, err := client.BridgeFromCashApp(breez_sdk_spark.CreateBridgeRequest{
	Recipient:      recipient,
	Route:          *route,
	Amount:         new(big.Int).SetInt64(10_000_000),
	RefundAddress:  nil,
	FeePolicy:      nil,
	MaxSlippageBps: nil,
})
if err != nil {
	return nil, err
}

// Open this Cash App link to pay. The recipient then receives the stablecoin.
info := response.Info
log.Printf("Open this URL in Cash App: %s", response.PaymentRequest)
log.Printf("Recipient receives ~%v %s", info.ExpectedReceivedAmount, info.DestinationAsset)
```



## Response fields

`bridge_from_cash_app` returns a `CreateBridgeResponse`, described in [Bridging USDC/USDT](./bridging.md#response-fields). Its `payment_request` is the Cash App deep link to open.

## Opening the payment link

On devices with Cash App installed the URL opens the app directly. Otherwise it falls back to the Cash App website. The same UX guidance as the [Buying Bitcoin](./buy_bitcoin.md#recommended-ux) Cash App flow applies (mobile redirect, desktop QR code, pre-opening a tab on web to avoid popup blockers).

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
