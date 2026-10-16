# USDC/USDT to Cash App

`bridge_to_cash_app` lets someone holding USDC or USDT on an external chain pay a Cash App user. The payer deposits the stablecoin, and the cross-chain provider pays the recipient in Bitcoin over Lightning. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. [Select a source route](./cash_app.md#selecting-a-route) with `CrossChainRouteFilter::Receive`.
2. Call `bridge_to_cash_app` with the recipient, the route, the USD amount and the payer's refund address. The SDK checks the recipient and gets a quote from the cross-chain provider.
3. Show the payer the returned `payment_request`. Once their deposit lands, the provider pays the recipient.

### Rust

```rust
// List the stablecoin sources that can pay a Cash App user over Lightning
// and pick one, e.g. USDC on Base.
let routes = sdk
    .get_cross_chain_routes(&CrossChainRouteFilter::Receive {
        contract_address: None,
        delivery_method: Some(DeliveryMethod::Lightning),
    })
    .await?;
let route = routes
    .into_iter()
    .find(|r| r.asset == "USDC" && r.chain == "base")
    .ok_or_else(|| anyhow::anyhow!("No USDC route on Base"))?;

// Pay $10 of USDC to the Cash App user $alice. The amount is in the route
// asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC, about $10.
// The deposit is refunded to the payer's address if delivery fails.
let response = sdk
    .bridge_to_cash_app(BridgeToCashAppRequest {
        recipient: "$alice".to_string(),
        route,
        amount: 10_000_000,
        fee_policy: None,
        refund_address: "<payer address>".to_string(),
        max_slippage_bps: None,
    })
    .await?;

// Show the payer what to pay. The recipient then receives Bitcoin.
let info = response.info;
info!("Pay with: {}", response.payment_request);
info!(
    "Deposit {} to {}, recipient receives ~{} sats",
    info.deposit_amount, info.deposit_address, info.expected_received_amount
);
```

### Swift

```swift
// List the stablecoin sources that can pay a Cash App user over Lightning
// and pick one, e.g. USDC on Base.
let routes = try await sdk.getCrossChainRoutes(
    filter: .receive(
        contractAddress: nil,
        deliveryMethod: .lightning
    ))
guard let route = routes.first(where: { $0.asset == "USDC" && $0.chain == "base" }) else {
    throw NSError(domain: "CashApp", code: 3)
}

// Pay $10 of USDC to the Cash App user $alice. The amount is in the route
// asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC, about $10.
// The deposit is refunded to the payer's address if delivery fails.
let response = try await sdk.bridgeToCashApp(
    request: BridgeToCashAppRequest(
        recipient: "$alice",
        route: route,
        amount: 10_000_000,
        feePolicy: nil,
        refundAddress: "<payer address>",
        maxSlippageBps: nil
    ))

// Show the payer what to pay. The recipient then receives Bitcoin.
let info = response.info
print("Pay with: \(response.paymentRequest)")
print(
    "Deposit \(info.depositAmount) to \(info.depositAddress), "
        + "recipient receives ~\(info.expectedReceivedAmount) sats"
)
```

### Kotlin

```kotlin
// List the stablecoin sources that can pay a Cash App user over Lightning
// and pick one, e.g. USDC on Base.
val routes = sdk.getCrossChainRoutes(
    CrossChainRouteFilter.Receive(
        contractAddress = null,
        deliveryMethod = DeliveryMethod.LIGHTNING,
    )
)
val route = routes.find { it.asset == "USDC" && it.chain == "base" }
    ?: throw IllegalArgumentException("No USDC route on Base")

// Pay $10 of USDC to the Cash App user $alice. The amount is in the route
// asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC, about $10.
// The deposit is refunded to the payer's address if delivery fails.
val response = sdk.bridgeToCashApp(
    BridgeToCashAppRequest(
        recipient = "\$alice",
        route = route,
        amount = BigInteger.fromLong(10_000_000L),
        feePolicy = null,
        refundAddress = "<payer address>",
        maxSlippageBps = null,
    )
)

// Show the payer what to pay. The recipient then receives Bitcoin.
val info = response.info
// Log.v("Breez", "Pay with: ${response.paymentRequest}")
// Log.v("Breez", "Deposit ${info.depositAmount} to ${info.depositAddress}, " +
//     "recipient receives ~${info.expectedReceivedAmount} sats")
```

### C#

```csharp
// List the stablecoin sources that can pay a Cash App user over
// Lightning and pick one, e.g. USDC on Base.
var filter = new CrossChainRouteFilter.Receive(
    contractAddress: null,
    deliveryMethod: DeliveryMethod.Lightning
);
var routes = await sdk.GetCrossChainRoutes(filter: filter);
var route = routes.First(r => r.asset == "USDC" && r.chain == "base");

// Pay $10 of USDC to the Cash App user $alice. The amount is in
// the route asset's base units (USDC, 6 decimals), so
// 10_000_000 = 10 USDC, about $10. The deposit is refunded to the
// payer's address if delivery fails.
var request = new BridgeToCashAppRequest(
    recipient: "$alice",
    route: route,
    amount: new BigInteger(10_000_000),
    feePolicy: null,
    refundAddress: "<payer address>",
    maxSlippageBps: null
);
var response = await sdk.BridgeToCashApp(request: request);

// Show the payer what to pay. The recipient then receives Bitcoin.
var info = response.info;
Console.WriteLine($"Pay with: {response.paymentRequest}");
Console.WriteLine(
    $"Deposit {info.depositAmount} to {info.depositAddress}, "
        + $"recipient receives ~{info.expectedReceivedAmount} sats"
);
```

### Javascript (Wasm)

```typescript
// List the stablecoin sources that can pay a Cash App user over Lightning
// and pick one, e.g. USDC on Base.
const routes = await sdk.getCrossChainRoutes({
  type: 'receive',
  contractAddress: undefined,
  deliveryMethod: 'lightning'
})
const route = routes.find((r) => r.asset === 'USDC' && r.chain === 'base')
if (route === undefined) {
  throw new Error('No USDC route on Base')
}

// Pay $10 of USDC to the Cash App user $alice. The amount is in the route
// asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC, about $10.
// The deposit is refunded to the payer's address if delivery fails.
const response = await sdk.bridgeToCashApp({
  recipient: '$alice',
  route,
  amount: BigInt(10_000_000),
  feePolicy: undefined,
  refundAddress: '<payer address>',
  maxSlippageBps: undefined
})

// Show the payer what to pay. The recipient then receives Bitcoin.
const info = response.info
console.log(`Pay with: ${response.paymentRequest}`)
console.log(
  `Deposit ${info.depositAmount} to ${info.depositAddress}, ` +
    `recipient receives ~${info.expectedReceivedAmount} sats`
)
```

### React Native

```typescript
// List the stablecoin sources that can pay a Cash App user over Lightning
// and pick one, e.g. USDC on Base.
const routes = await sdk.getCrossChainRoutes(
  new CrossChainRouteFilter.Receive({
    contractAddress: undefined,
    deliveryMethod: DeliveryMethod.Lightning
  })
)
const route = routes.find((r) => r.asset === 'USDC' && r.chain === 'base')
if (route === undefined) {
  throw new Error('No USDC route on Base')
}

// Pay $10 of USDC to the Cash App user $alice. The amount is in the route
// asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC, about $10.
// The deposit is refunded to the payer's address if delivery fails.
const response = await sdk.bridgeToCashApp({
  recipient: '$alice',
  route,
  amount: BigInt(10_000_000),
  feePolicy: undefined,
  refundAddress: '<payer address>',
  maxSlippageBps: undefined
})

// Show the payer what to pay. The recipient then receives Bitcoin.
const info = response.info
console.log(`Pay with: ${response.paymentRequest}`)
console.log(
  `Deposit ${info.depositAmount} to ${info.depositAddress}, ` +
    `recipient receives ~${info.expectedReceivedAmount} sats`
)
```

### Flutter

```dart
// List the stablecoin sources that can pay a Cash App user over Lightning
// and pick one, e.g. USDC on Base.
List<CrossChainRoutePair> routes = await sdk.getCrossChainRoutes(
  filter: CrossChainRouteFilter.receive(
    contractAddress: null,
    deliveryMethod: DeliveryMethod.lightning,
  ),
);
CrossChainRoutePair route =
    routes.firstWhere((r) => r.asset == "USDC" && r.chain == "base");

// Pay $10 of USDC to the Cash App user $alice. The amount is in the route
// asset's base units (USDC, 6 decimals), so 10000000 = 10 USDC, about $10.
// The deposit is refunded to the payer's address if delivery fails.
final response = await sdk.bridgeToCashApp(
  request: BridgeToCashAppRequest(
    recipient: "\$alice",
    route: route,
    amount: BigInt.from(10000000),
    feePolicy: null,
    refundAddress: "<payer address>",
    maxSlippageBps: null,
  ),
);

// Show the payer what to pay. The recipient then receives Bitcoin.
final info = response.info;
print("Pay with: ${response.paymentRequest}");
print(
  "Deposit ${info.depositAmount} to ${info.depositAddress},"
  " recipient receives ~${info.expectedReceivedAmount} sats",
);
```

### Python

```python
try:
    # List the stablecoin sources that can pay a Cash App user over
    # Lightning and pick one, e.g. USDC on Base.
    routes = await sdk.get_cross_chain_routes(
        filter=CrossChainRouteFilter.RECEIVE(
            contract_address=None,
            delivery_method=DeliveryMethod.LIGHTNING,
        )
    )
    route = next(
        (r for r in routes if r.asset == "USDC" and r.chain == "base"),
        None,
    )
    if route is None:
        raise ValueError("No USDC route on Base")

    # Pay $10 of USDC to the Cash App user $alice. The amount is in the
    # route asset's base units (USDC, 6 decimals), so 10_000_000 =
    # 10 USDC, about $10. The deposit is refunded to the payer's address
    # if delivery fails.
    request = BridgeToCashAppRequest(
        recipient="$alice",
        route=route,
        amount=10_000_000,
        fee_policy=None,
        refund_address="<payer address>",
        max_slippage_bps=None,
    )
    response = await sdk.bridge_to_cash_app(request=request)

    # Show the payer what to pay. The recipient then receives Bitcoin.
    info = response.info
    logging.debug(f"Pay with: {response.payment_request}")
    logging.debug(
        f"Deposit {info.deposit_amount} to {info.deposit_address}, "
        f"recipient receives ~{info.expected_received_amount} sats"
    )
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
// List the stablecoin sources that can pay a Cash App user over Lightning
// and pick one, e.g. USDC on Base.
deliveryMethod := breez_sdk_spark.DeliveryMethodLightning
filter := breez_sdk_spark.CrossChainRouteFilterReceive{
	ContractAddress: nil,
	DeliveryMethod:  &deliveryMethod,
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

// Pay $10 of USDC to the Cash App user $alice. The amount is in the route
// asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC, about $10.
// The deposit is refunded to the payer's address if delivery fails.
request := breez_sdk_spark.BridgeToCashAppRequest{
	Recipient:      "$alice",
	Route:          *route,
	Amount:         new(big.Int).SetInt64(10_000_000),
	FeePolicy:      nil,
	RefundAddress:  "<payer address>",
	MaxSlippageBps: nil,
}
response, err := sdk.BridgeToCashApp(request)
if err != nil {
	return nil, err
}

// Show the payer what to pay. The recipient then receives Bitcoin.
info := response.Info
log.Printf("Pay with: %s", response.PaymentRequest)
log.Printf(
	"Deposit %v to %s, recipient receives ~%v sats",
	info.DepositAmount, info.DepositAddress, info.ExpectedReceivedAmount,
)
```



## Recipient

The `recipient` is a Cash App username, given as `alice` or `$alice`, or the user's Cash App Lightning address, `alice@cash.app`. All three reach the same user, who is paid at that Lightning address.

The SDK looks the recipient up before quoting, so an unknown username fails with `SdkError::InvalidInput` before the payer funds anything. So does an amount outside what the recipient accepts.

## Refund address

The `refund_address` is the payer's own address on the source chain. If the recipient can't be paid, the provider refunds the deposit there. It must belong to the route's chain family (e.g. an EVM `0x...` address for USDC on Base).

## Response fields

`BridgeToCashAppResponse` carries what the payer pays and the quote:

| Field | Meaning |
| ----- | ------- |
| `payment_request` | What the payer pays: an EIP-681 URI on EVM chains, otherwise the bare deposit address. |
| `info.deposit_address` | The deposit address on the source chain. |
| `info.deposit_amount` | The amount the payer deposits, in the route asset's base units. |
| `info.expected_received_amount` | The sats the recipient is expected to receive. |
| `info.service_fee_amount` | Provider fee, including your [partner fee](./cross_chain_partner_fees.md) if you set one, in `info.service_fee_asset` units. |
| `info.expires_at` | Quote expiry, in seconds since the Unix epoch. Call `bridge_to_cash_app` again for a fresh quote if it lapses before the payer deposits. |

## Limitations

- **Delivery can still fail after funding.** The provider looks the recipient up again when it pays. If the account was closed, its limit lowered, or slippage moved the amount across a limit since the quote, the payment fails and the deposit is refunded to `refund_address`.
- **Lightning delivery only.** A completed bridge means the Lightning payment was delivered. Whether Cash App credits it as Bitcoin or converts it to dollars depends on the recipient's Cash App settings.

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
