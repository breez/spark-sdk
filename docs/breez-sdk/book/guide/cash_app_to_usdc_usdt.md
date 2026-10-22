# Cash App to USDC/USDT

`bridge_from_cash_app` lets a user send USDC or USDT to a recipient on an external chain. The user pays through Cash App over Lightning, and the cross-chain provider delivers the stablecoin to the recipient. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. Parse the recipient's external-chain address and [select a destination route](./cash_app.md#selecting-a-route) with `CrossChainRouteFilter::Send`.
2. Call `bridge_from_cash_app` with the route and the USD amount. The SDK gets a quote from the cross-chain provider and returns a Cash App deep link plus the quote details.
3. Open the returned `url`. The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

The SDK builds a `cash.app/launch/lightning/<bolt11>` deep link from the provider's invoice and returns it as the response `url`.

### Rust

```rust
// Parse the recipient's external-chain address (EVM/Solana/Tron).
let input = "<recipient address>";
let InputType::CrossChainAddress(address_details) = sdk.parse(input).await? else {
    anyhow::bail!("Not a cross-chain address");
};

// List the stablecoin destinations Cash App can fund over Lightning and
// pick one, e.g. USDC on Base.
let routes = sdk
    .get_cross_chain_routes(&CrossChainRouteFilter::Send {
        address_details: address_details.clone(),
        delivery_method: Some(DeliveryMethod::Lightning),
    })
    .await?;
let route = routes
    .into_iter()
    .find(|r| r.asset == "USDC" && r.chain == "base")
    .ok_or_else(|| anyhow::anyhow!("No USDC route on Base"))?;

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in the
// route asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC,
// about $10.
let response = sdk
    .bridge_from_cash_app(BridgeFromCashAppRequest {
        address: address_details.address,
        route,
        amount: 10_000_000,
        fee_policy: None,
        max_slippage_bps: None,
    })
    .await?;

// Open this Cash App URL to pay. The recipient then receives the stablecoin.
info!("Open this URL in Cash App: {}", response.url);
info!(
    "Recipient receives ~{} {}",
    response.estimated_out, response.asset
);
```

### Swift

```swift
// Parse the recipient's external-chain address (EVM/Solana/Tron).
let parsed = try await sdk.parse(input: "<recipient address>")
guard case let .crossChainAddress(v1: addressDetails) = parsed else {
    throw NSError(domain: "CashApp", code: 1)
}

// List the stablecoin destinations Cash App can fund over Lightning and
// pick one, e.g. USDC on Base.
let routes = try await sdk.getCrossChainRoutes(
    filter: .send(
        addressDetails: addressDetails,
        deliveryMethod: .lightning
    ))
guard let route = routes.first(where: { $0.asset == "USDC" && $0.chain == "base" }) else {
    throw NSError(domain: "CashApp", code: 2)
}

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in the
// route asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC,
// about $10.
let response = try await sdk.bridgeFromCashApp(
    request: BridgeFromCashAppRequest(
        address: addressDetails.address,
        route: route,
        amount: 10_000_000,
        feePolicy: nil,
        maxSlippageBps: nil
    ))

// Open this Cash App URL to pay. The recipient then receives the stablecoin.
print("Open this URL in Cash App: \(response.url)")
print("Recipient receives ~\(response.estimatedOut) \(response.asset)")
```

### Kotlin

```kotlin
// Parse the recipient's external-chain address (EVM/Solana/Tron).
val input = "<recipient address>"
val parsed = sdk.parse(input)
if (parsed !is InputType.CrossChainAddress) {
    throw IllegalArgumentException("Not a cross-chain address")
}
val addressDetails = parsed.v1

// List the stablecoin destinations Cash App can fund over Lightning and
// pick one, e.g. USDC on Base.
val routes = sdk.getCrossChainRoutes(
    CrossChainRouteFilter.Send(
        addressDetails = addressDetails,
        deliveryMethod = DeliveryMethod.LIGHTNING,
    )
)
val route = routes.find { it.asset == "USDC" && it.chain == "base" }
    ?: throw IllegalArgumentException("No USDC route on Base")

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in
// the route asset's base units (USDC, 6 decimals), so 10_000_000 =
// 10 USDC, about $10.
val response = sdk.bridgeFromCashApp(
    BridgeFromCashAppRequest(
        address = addressDetails.address,
        route = route,
        amount = BigInteger.fromLong(10_000_000L),
        feePolicy = null,
        maxSlippageBps = null,
    )
)

// Open this Cash App URL to pay. The recipient then receives the stablecoin.
// Log.v("Breez", "Open this URL in Cash App: ${response.url}")
// Log.v("Breez", "Recipient receives ~${response.estimatedOut} ${response.asset}")
```

### C#

```csharp
// Parse the recipient's external-chain address (EVM/Solana/Tron).
var parsed = await sdk.Parse(input: "<recipient address>");
if (parsed is not InputType.CrossChainAddress crossChain)
{
    throw new InvalidOperationException("Not a cross-chain address");
}
var addressDetails = crossChain.v1;

// List the stablecoin destinations Cash App can fund over Lightning
// and pick one, e.g. USDC on Base.
var filter = new CrossChainRouteFilter.Send(
    addressDetails: addressDetails,
    deliveryMethod: DeliveryMethod.Lightning
);
var routes = await sdk.GetCrossChainRoutes(filter: filter);
var route = routes.First(r => r.asset == "USDC" && r.chain == "base");

// Send $10 of USDC, funded by Cash App over Lightning. The amount
// is in the route asset's base units (USDC, 6 decimals), so
// 10_000_000 = 10 USDC, about $10.
var request = new BridgeFromCashAppRequest(
    address: addressDetails.address,
    route: route,
    amount: new BigInteger(10_000_000),
    feePolicy: null,
    maxSlippageBps: null
);
var response = await sdk.BridgeFromCashApp(request: request);

// Open this Cash App URL to pay. The recipient then receives the
// stablecoin.
Console.WriteLine($"Open this URL in Cash App: {response.url}");
Console.WriteLine($"Recipient receives ~{response.estimatedOut} {response.asset}");
```

### Javascript (Wasm)

```typescript
// Parse the recipient's external-chain address (EVM/Solana/Tron).
const input = '<recipient address>'
const parsed = await sdk.parse(input)
if (parsed.type !== 'crossChainAddress') {
  throw new Error('Not a cross-chain address')
}

// List the stablecoin destinations Cash App can fund over Lightning and
// pick one, e.g. USDC on Base.
const routes = await sdk.getCrossChainRoutes({
  type: 'send',
  addressDetails: parsed,
  deliveryMethod: 'lightning'
})
const route = routes.find((r) => r.asset === 'USDC' && r.chain === 'base')
if (route === undefined) {
  throw new Error('No USDC route on Base')
}

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in the
// route asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC,
// about $10.
const response = await sdk.bridgeFromCashApp({
  address: parsed.address,
  route,
  amount: BigInt(10_000_000),
  feePolicy: undefined,
  maxSlippageBps: undefined
})

// Open this Cash App URL to pay. The recipient then receives the stablecoin.
console.log(`Open this URL in Cash App: ${response.url}`)
console.log(`Recipient receives ~${response.estimatedOut} ${response.asset}`)
```

### React Native

```typescript
// Parse the recipient's external-chain address (EVM/Solana/Tron).
const input = '<recipient address>'
const parsed = await sdk.parse(input)
if (parsed.tag !== InputType_Tags.CrossChainAddress) {
  throw new Error('Not a cross-chain address')
}
const addressDetails = parsed.inner[0]

// List the stablecoin destinations Cash App can fund over Lightning and
// pick one, e.g. USDC on Base.
const routes = await sdk.getCrossChainRoutes(
  new CrossChainRouteFilter.Send({
    addressDetails,
    deliveryMethod: DeliveryMethod.Lightning
  })
)
const route = routes.find((r) => r.asset === 'USDC' && r.chain === 'base')
if (route === undefined) {
  throw new Error('No USDC route on Base')
}

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in the
// route asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC,
// about $10.
const response = await sdk.bridgeFromCashApp({
  address: addressDetails.address,
  route,
  amount: BigInt(10_000_000),
  feePolicy: undefined,
  maxSlippageBps: undefined
})

// Open this Cash App URL to pay. The recipient then receives the stablecoin.
console.log(`Open this URL in Cash App: ${response.url}`)
console.log(`Recipient receives ~${response.estimatedOut} ${response.asset}`)
```

### Flutter

```dart
// Parse the recipient's external-chain address (EVM/Solana/Tron).
String input = "<recipient address>";
InputType parsed = await sdk.parse(input: input);
if (parsed is! InputType_CrossChainAddress) {
  throw Exception("Not a cross-chain address");
}
CrossChainAddressDetails addressDetails = parsed.field0;

// List the stablecoin destinations Cash App can fund over Lightning and
// pick one, e.g. USDC on Base.
List<CrossChainRoutePair> routes = await sdk.getCrossChainRoutes(
  filter: CrossChainRouteFilter.send(
    addressDetails: addressDetails,
    deliveryMethod: DeliveryMethod.lightning,
  ),
);
CrossChainRoutePair route =
    routes.firstWhere((r) => r.asset == "USDC" && r.chain == "base");

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in the
// route asset's base units (USDC, 6 decimals), so 10000000 = 10 USDC,
// about $10.
final response = await sdk.bridgeFromCashApp(
  request: BridgeFromCashAppRequest(
    address: addressDetails.address,
    route: route,
    amount: BigInt.from(10000000),
    feePolicy: null,
    maxSlippageBps: null,
  ),
);

// Open this Cash App URL to pay. The recipient then receives the stablecoin.
print("Open this URL in Cash App: ${response.url}");
print("Recipient receives ~${response.estimatedOut} ${response.asset}");
```

### Python

```python
# Parse the recipient's external-chain address (EVM/Solana/Tron).
input_str = "<recipient address>"
try:
    parsed = await sdk.parse(input=input_str)
    if not isinstance(parsed, InputType.CROSS_CHAIN_ADDRESS):
        raise ValueError("Not a cross-chain address")
    address_details = parsed[0]

    # List the stablecoin destinations Cash App can fund over Lightning and
    # pick one, e.g. USDC on Base.
    routes = await sdk.get_cross_chain_routes(
        filter=CrossChainRouteFilter.SEND(
            address_details=address_details,
            delivery_method=DeliveryMethod.LIGHTNING,
        )
    )
    route = next(
        (r for r in routes if r.asset == "USDC" and r.chain == "base"),
        None,
    )
    if route is None:
        raise ValueError("No USDC route on Base")

    # Send $10 of USDC, funded by Cash App over Lightning. The amount is in
    # the route asset's base units (USDC, 6 decimals), so 10_000_000 =
    # 10 USDC, about $10.
    request = BridgeFromCashAppRequest(
        address=address_details.address,
        route=route,
        amount=10_000_000,
        fee_policy=None,
        max_slippage_bps=None,
    )
    response = await sdk.bridge_from_cash_app(request=request)

    # Open this Cash App URL to pay. The recipient then receives the
    # stablecoin.
    logging.debug(f"Open this URL in Cash App: {response.url}")
    logging.debug(
        f"Recipient receives ~{response.estimated_out} {response.asset}"
    )
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
// Parse the recipient's external-chain address (EVM/Solana/Tron).
inputStr := "<recipient address>"
input, err := sdk.Parse(inputStr)
if err != nil {
	return nil, err
}
addressInput, ok := input.(breez_sdk_spark.InputTypeCrossChainAddress)
if !ok {
	return nil, errors.New("not a cross-chain address")
}
addressDetails := addressInput.Field0

// List the stablecoin destinations Cash App can fund over Lightning and
// pick one, e.g. USDC on Base.
deliveryMethod := breez_sdk_spark.DeliveryMethodLightning
filter := breez_sdk_spark.CrossChainRouteFilterSend{
	AddressDetails: addressDetails,
	DeliveryMethod: &deliveryMethod,
}
routes, err := sdk.GetCrossChainRoutes(filter)
if err != nil {
	return nil, err
}
var route *breez_sdk_spark.CrossChainRoutePair
for i := range routes {
	if routes[i].Asset == "USDC" && routes[i].Chain == "base" {
		route = &routes[i]
		break
	}
}
if route == nil {
	return nil, errors.New("no USDC route on Base")
}

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in the
// route asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC,
// about $10.
request := breez_sdk_spark.BridgeFromCashAppRequest{
	Address:        addressDetails.Address,
	Route:          *route,
	Amount:         new(big.Int).SetInt64(10_000_000),
	FeePolicy:      nil,
	MaxSlippageBps: nil,
}
response, err := sdk.BridgeFromCashApp(request)
if err != nil {
	return nil, err
}

// Open this Cash App URL to pay. The recipient then receives the stablecoin.
log.Printf("Open this URL in Cash App: %v", response.Url)
log.Printf("Recipient receives ~%v %s", response.EstimatedOut, response.Asset)
```



## Response fields

`BridgeFromCashAppResponse` carries the payment URL and the quote:

| Field | Meaning |
| ----- | ------- |
| `url` | The Cash App deep link to open. |
| `amount_sats` | Bitcoin amount the user deposits through Cash App. |
| `estimated_out` | Expected amount delivered to the recipient, in `asset` units. |
| `asset` | The delivered stablecoin (e.g. `USDC`). |
| `service_fee_amount` | Provider fee for the conversion, including your [partner fee](./cross_chain_partner_fees.md) if you set one. |
| `service_fee_asset` | Denomination of the fee. Absent means the fee is in sats. |
| `service_fee_asset_decimals` | Decimals of `service_fee_asset`, for formatting the fee. Absent when the fee is in sats or the provider did not report them. |
| `expires_at` | Quote expiry, in seconds since the Unix epoch. Call `bridge_from_cash_app` again for a fresh quote if it lapses before the user starts paying. |

## Opening the payment link

On devices with Cash App installed the URL opens the app directly. Otherwise it falls back to the Cash App website. The same UX guidance as the [Buying Bitcoin](./buy_bitcoin.md#recommended-ux) Cash App flow applies (mobile redirect, desktop QR code, pre-opening a tab on web to avoid popup blockers).

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
