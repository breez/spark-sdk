# Cash App to USDC/USDT

`bridge_from_cash_app` lets a user send USDC or USDT to a recipient on an external chain. The user pays through Cash App over Lightning, and the cross-chain provider delivers the stablecoin to the recipient. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. Get a [bridging client](./bridging.md#getting-a-bridging-client): build one from your API key with `new_bridging_client`, or call `get_bridging_client` on a connected SDK.
2. [Select a destination route](./cash_app.md#selecting-a-route) for the recipient's external-chain address.
3. Call `bridge_from_cash_app` with the recipient, the route and the USD amount. The SDK creates an order with the cross-chain provider and returns a Cash App deep link plus the order details.
4. Open the returned `payment_request`, and store the `bridge_id` and `status_token` to [track the bridge](#tracking-the-bridge). The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

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



## Slippage and refunds

The provider allows these payments up to 10% of price movement between the quote and the delivery, whatever `max_slippage_bps` says. With fees excluded, `max_slippage_bps` still bounds how far the quoted delivery may fall short of the `amount`: a bridge quoted below it fails before the user sees a link. With fees included, the quoted delivery isn't checked.

If the payment fails before the swap, the provider refunds it to the [refund address](./bridging.md#refund-address): a Lightning address, Cash App `$cashtag` or amountless BOLT11 invoice. An invoice must not expire before the bridge, and must still be valid when the refund is paid. Without a refund address, the payment can't be refunded automatically.

## Response fields

`bridge_from_cash_app` returns a `CreateBridgeResponse`, described in [Bridging USDC/USDT](./bridging.md#response-fields). Its `payment_request` is the Cash App deep link to open.

## Opening the payment link

On devices with Cash App installed the URL opens the app directly. Otherwise it falls back to the Cash App website. The same UX guidance as the [Buying Bitcoin](./buy_bitcoin.md#recommended-ux) Cash App flow applies (mobile redirect, desktop QR code, pre-opening a tab on web to avoid popup blockers).

## Tracking the bridge

Pass the `bridge_id` and `status_token` from the response to `get_bridge_status`. The bridge reads:

- `BridgeStatus::Created` until the user pays in Cash App.
- `BridgeStatus::Expired` if nobody pays before its `expires_at`. The link can't be paid after that, so create a new bridge.
- `BridgeStatus::Pending` once the payment arrives, while the provider swaps and delivers.
- `BridgeStatus::Completed` once the recipient has the stablecoin. `delivery_tx_hash` is the delivery transaction on the recipient's chain.
- `BridgeStatus::Refunding`, then `BridgeStatus::Refunded`, if the payment failed before the swap and is returned to the [refund address](#slippage-and-refunds).

### Rust

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

### Swift

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

### Kotlin

```kotlin
val response = client.getBridgeStatus(
    GetBridgeStatusRequest(bridgeId = bridgeId, statusToken = statusToken)
)
when (response.status) {
    BridgeStatus.COMPLETED -> {
        // Log.v("Breez", "Delivered ${response.amountOut}")
    }
    BridgeStatus.REFUNDED, BridgeStatus.FAILED, BridgeStatus.EXPIRED -> {
        // Log.v("Breez", "Not delivered: ${response.status}")
    }
    else -> {
        // Log.v("Breez", "Not final yet: ${response.status}")
    }
}
```

### C#

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

### Javascript (Wasm)

```typescript
const response = await client.getBridgeStatus({ bridgeId, statusToken })
switch (response.status) {
  case 'completed': {
    console.log(`Delivered ${response.amountOut ?? 'unknown'}`)
    break
  }
  case 'refunded':
  case 'failed':
  case 'expired': {
    console.log(`Not delivered: ${response.status}`)
    break
  }
  default: {
    console.log(`Not final yet: ${response.status}`)
  }
}
```

### React Native

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

### Flutter

```dart
final response = await client.getBridgeStatus(
  request: GetBridgeStatusRequest(
    bridgeId: bridgeId,
    statusToken: statusToken,
  ),
);
switch (response.status) {
  case BridgeStatus.completed:
    print("Delivered ${response.amountOut}");
    break;
  case BridgeStatus.refunded:
  case BridgeStatus.failed:
  case BridgeStatus.expired:
    print("Not delivered: ${response.status}");
    break;
  default:
    print("Not final yet: ${response.status}");
}
```

### Python

```python
try:
    response = await client.get_bridge_status(
        request=GetBridgeStatusRequest(
            bridge_id=bridge_id, status_token=status_token
        )
    )
except Exception as error:
    logging.error(error)
    raise
status = response.status
if status == BridgeStatus.COMPLETED:
    logging.debug(f"Delivered {response.amount_out}")
elif status in (BridgeStatus.REFUNDED, BridgeStatus.FAILED, BridgeStatus.EXPIRED):
    logging.debug(f"Not delivered: {status}")
else:
    logging.debug(f"Not final yet: {status}")
```

### Go

```go
response, err := client.GetBridgeStatus(breez_sdk_spark.GetBridgeStatusRequest{
	BridgeId:    bridgeId,
	StatusToken: statusToken,
})
if err != nil {
	return err
}
switch response.Status {
case breez_sdk_spark.BridgeStatusCompleted:
	if response.AmountOut != nil {
		log.Printf("Delivered %v", *response.AmountOut)
	}
case breez_sdk_spark.BridgeStatusRefunded,
	breez_sdk_spark.BridgeStatusFailed,
	breez_sdk_spark.BridgeStatusExpired:
	log.Printf("Not delivered: %v", response.Status)
default:
	log.Printf("Not final yet: %v", response.Status)
}
```



See [Tracking a bridge](./bridging.md#tracking-a-bridge) for every status.

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
