# Cash App to USDC/USDT payments

`prepare_payment_link` lets a user send USDC or USDT to a recipient on an external chain (Ethereum-family, Solana, or Tron). The user pays through Cash App (over Lightning) and a cross-chain provider delivers the stablecoin to the recipient.

The SDK acts as orchestrator only. No wallet funds move, and nothing is written to the payment history: Cash App funds the purchase externally and the provider delivers autonomously. This is distinct from the wallet-funded, tracked flow on the [Send USDC/USDT](./cross_chain.md) page.

**Developer note**

Cash App is available in the US and UK only (excluding New York State for Bitcoin/Lightning features). Cash App handles region restrictions on their end, so no client-side gating is needed.

## How it works

1. Parse the recipient's external-chain address and select a destination route.
2. Call `prepare_payment_link` with the route and the USD amount. The SDK gets a quote from the cross-chain provider and returns a Cash App deep link plus the quote details.
3. Open the returned `url`. The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

Because the SDK never sees the funds, `prepare_payment_link` returns and forgets: there is no `Payment` row, no status tracking, and no event. Surface the quote fields in the response so the app can show the user what to expect before they pay.

## Selecting a route

Parse the recipient address, then list the stablecoin destinations you can send to with `get_cross_chain_routes` filtered by `CrossChainRouteFilter::PaymentLink`. These routes are the ones fundable by an external rail (Cash App over Lightning), a different set from the wallet-funded send routes. Pick the `CrossChainRoutePair` whose `CrossChainRoutePair.asset` and `CrossChainRoutePair.chain` match the destination you want, and pass it to `prepare_payment_link`.

Calling `prepare_payment_link` with a route that can't be funded over Lightning fails fast, before any funds move.

## Preparing the payment link

Cash App funds the purchase over **Lightning**. The SDK builds a `cash.app/launch/lightning/<bolt11>` deep link from the provider's invoice and returns it as the response `url`.

### Rust

```rust
// Parse the recipient's external-chain address (EVM/Solana/Tron).
let input = "<recipient address>";
let InputType::CrossChainAddress(address_details) = sdk.parse(input).await? else {
    anyhow::bail!("Not a cross-chain address");
};

// List the stablecoin destinations you can send to and pick one, e.g. USDC
// on Base. Payment-link routes are funded by Cash App over Lightning.
let routes = sdk
    .get_cross_chain_routes(&CrossChainRouteFilter::PaymentLink {
        address_details: address_details.clone(),
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
    .prepare_payment_link(PreparePaymentLinkRequest {
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
    throw NSError(domain: "PreparePaymentLink", code: 1)
}

// List the stablecoin destinations you can send to and pick one, e.g. USDC on Base.
// Restrict to routes fundable over Lightning.
let routes = try await sdk.getCrossChainRoutes(
    filter: .paymentLink(addressDetails: addressDetails))
guard let route = routes.first(where: { $0.asset == "USDC" && $0.chain == "base" }) else {
    throw NSError(domain: "PreparePaymentLink", code: 2)
}

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in the
// route asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC, about $10.
let response = try await sdk.preparePaymentLink(
    request: PreparePaymentLinkRequest(
        address: addressDetails.address,
        route: route,
        amount: 10_000_000,
        feePolicy: nil,
        maxSlippageBps: nil
    ))
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

// List the stablecoin destinations you can send to and pick one, e.g. USDC
// on Base. These routes are funded by an external rail, not the Spark wallet.
// Restrict to Lightning-fundable routes, since Cash App pays over Lightning.
val routes = sdk.getCrossChainRoutes(
    CrossChainRouteFilter.PaymentLink(
        addressDetails = addressDetails,
    )
)
val route = routes.find { it.asset == "USDC" && it.chain == "base" }
    ?: throw IllegalArgumentException("No USDC route on Base")

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in
// the route asset's base units (USDC, 6 decimals), so 10_000_000 =
// 10 USDC, about $10.
val request = PreparePaymentLinkRequest(
    address = addressDetails.address,
    route = route,
    amount = BigInteger.fromLong(10_000_000L),
    feePolicy = null,
    maxSlippageBps = null,
)

val response = sdk.preparePaymentLink(request)

// Open this Cash App URL to pay; the recipient then receives the stablecoin.
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

// List the stablecoin destinations you can send to and pick one,
// e.g. USDC on Base. Restrict to Lightning-fundable routes,
// since Cash App funds over Lightning.
var filter = new CrossChainRouteFilter.PaymentLink(
    addressDetails: addressDetails
);
var routes = await sdk.GetCrossChainRoutes(filter: filter);
var route = routes.First(r => r.asset == "USDC" && r.chain == "base");

// Send $10 of USDC, funded by Cash App over Lightning. The amount
// is in the route asset's base units (USDC, 6 decimals), so
// 10_000_000 = 10 USDC, about $10.
var request = new PreparePaymentLinkRequest(
    address: addressDetails.address,
    route: route,
    amount: new BigInteger(10_000_000),
    feePolicy: null,
    maxSlippageBps: null
);
var response = await sdk.PreparePaymentLink(request: request);
Console.WriteLine($"Open this URL in Cash App: {response.url}");
Console.WriteLine($"Recipient receives ~{response.estimatedOut} {response.asset}");
```

### Javascript (Wasm)

```typescript
// Parse the recipient's external-chain address (EVM/Solana/Tron).
const parsed = await sdk.parse('<recipient address>')
if (parsed.type !== 'crossChainAddress') {
  throw new Error('Not a cross-chain address')
}

// List the stablecoin destinations you can send to and pick one, e.g. USDC on Base.
// Restrict to Lightning-fundable routes, matching Cash App's Lightning funding.
const routes = await sdk.getCrossChainRoutes({
  type: 'paymentLink',
  addressDetails: parsed
})
const route = routes.find((r) => r.asset === 'USDC' && r.chain === 'base')
if (route === undefined) {
  throw new Error('No USDC on Base route available')
}

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in the
// route asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC,
// about $10.
const response = await sdk.preparePaymentLink({
  address: parsed.address,
  route,
  amount: BigInt(10_000_000),
  feePolicy: undefined,
  maxSlippageBps: undefined
})
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

// List the stablecoin destinations you can send to and pick one, e.g. USDC
// on Base. These routes are funded by an external rail, not the Spark wallet.
// Restrict to Lightning-fundable routes.
const routes = await sdk.getCrossChainRoutes(
  new CrossChainRouteFilter.PaymentLink({
    addressDetails
  })
)
const route = routes.find((r) => r.asset === 'USDC' && r.chain === 'base')
if (route === undefined) {
  throw new Error('No USDC route on Base')
}

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in the
// route asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC,
// about $10.
const response = await sdk.preparePaymentLink({
  address: addressDetails.address,
  route,
  amount: BigInt(10_000_000),
  feePolicy: undefined,
  maxSlippageBps: undefined
})

// Open this Cash App URL to pay; the recipient then receives the stablecoin.
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

// List the stablecoin destinations you can send to and pick one, e.g. USDC
// on Base. These routes are funded by an external rail, not the Spark wallet.
// Restrict to Lightning-fundable routes to match the Cash App flow below.
List<CrossChainRoutePair> routes = await sdk.getCrossChainRoutes(
  filter: CrossChainRouteFilter_PaymentLink(
    addressDetails: addressDetails,
  ),
);
CrossChainRoutePair route =
    routes.firstWhere((r) => r.asset == "USDC" && r.chain == "base");

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in the
// route asset's base units (USDC, 6 decimals), so 10000000 = 10 USDC,
// about $10.
final response = await sdk.preparePaymentLink(
  request: PreparePaymentLinkRequest(
    address: addressDetails.address,
    route: route,
    amount: BigInt.from(10000000),
    feePolicy: null,
    maxSlippageBps: null,
  ),
);

// Open this Cash App URL to pay; the recipient then receives the stablecoin.
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

    # List the stablecoin destinations you can send to and pick one,
    # e.g. USDC on Base. Restrict to Lightning-fundable routes.
    routes = await sdk.get_cross_chain_routes(
        filter=CrossChainRouteFilter.PAYMENT_LINK(
            address_details=address_details,
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
    request = PreparePaymentLinkRequest(
        address=address_details.address,
        route=route,
        amount=10_000_000,
        fee_policy=None,
        max_slippage_bps=None,
    )
    response = await sdk.prepare_payment_link(request=request)
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

// List the stablecoin destinations you can send to and pick one, e.g. USDC
// on Base. These routes are funded by an external rail, not the Spark wallet.
// Restrict to Lightning-fundable routes, since Cash App funds over Lightning.
filter := breez_sdk_spark.CrossChainRouteFilterPaymentLink{
	AddressDetails: addressDetails,
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
request := breez_sdk_spark.PreparePaymentLinkRequest{
	Address:        addressDetails.Address,
	Route:          *route,
	Amount:         new(big.Int).SetInt64(10_000_000),
	FeePolicy:      nil,
	MaxSlippageBps: nil,
}
response, err := sdk.PreparePaymentLink(request)
if err != nil {
	return nil, err
}

// Open this Cash App URL to pay; the recipient then receives the stablecoin.
log.Printf("Open this URL in Cash App: %v", response.Url)
log.Printf("Recipient receives ~%v %s", response.EstimatedOut, response.Asset)
```



### Amounts and fees

The `amount` on `PreparePaymentLinkRequest` is in the destination asset's base units, per the route's `CrossChainRoutePair.decimals`. These routes deliver USD-pegged stablecoins, so at parity it is the USD value: `1_000_000` is 1 USDC (6 decimals), about $1.

`fee_policy` controls who absorbs the provider fee, reusing the same `FeePolicy` as the send flow:

- `FeePolicy::FeesExcluded` (default) delivers the target amount to the recipient and adds the fee on top of what the user pays.
- `FeePolicy::FeesIncluded` takes the fee out of the amount, so the user pays exactly the amount and the recipient receives less.

`max_slippage_bps` bounds the price movement tolerated between quote and delivery, in basis points (1 bps = 0.01%). Left unset, the provider default applies.

### Response fields

`PreparePaymentLinkResponse` carries the payment URL and the quote:

| Field | Meaning |
| ----- | ------- |
| `url` | The Cash App deep link to open. |
| `amount_sats` | Bitcoin amount the user deposits through Cash App. |
| `estimated_out` | Expected amount delivered to the recipient, in `asset` units. |
| `asset` | The delivered stablecoin (e.g. `USDC`). |
| `service_fee_amount` | Provider fee for the conversion. |
| `service_fee_asset` | Denomination of the fee. Absent means the fee is in sats. |
| `expires_at` | Quote expiry. Re-call `prepare_payment_link` for a fresh quote if it lapses before the user starts paying. |

## Opening the payment link

On devices with Cash App installed the URL opens the app directly; otherwise it falls back to the Cash App website. The same UX guidance as the [Buying Bitcoin](./buy_bitcoin.md#recommended-ux) Cash App flow applies (mobile redirect, desktop QR code, pre-opening a tab on web to avoid popup blockers).

## Limitations

- **Mainnet only.** Cash App and the cross-chain providers operate against live networks. There is no testnet equivalent.
- **Not tracked.** The purchase is funded outside the wallet, so it produces no `Payment` row and no event. If you need delivery confirmation, observe the recipient chain directly.
- **Orchestra routes only.** Payment links quote against Orchestra, whose orders deliver without the SDK online. A Boltz route would need the wallet to stay online to claim the swap before the payer's payment settles, so `get_cross_chain_routes` with `CrossChainRouteFilter::PaymentLink` returns only Orchestra routes and `prepare_payment_link` rejects a Boltz route.

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
