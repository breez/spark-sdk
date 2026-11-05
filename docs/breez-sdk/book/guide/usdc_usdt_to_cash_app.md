# USDC/USDT to Cash App

`bridge_to_cash_app` lets someone holding USDC or USDT on an external chain pay a Cash App user. The payer deposits the stablecoin, and the cross-chain provider pays the recipient in Bitcoin over Lightning. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. [Select a source route](./cash_app.md#selecting-a-route) for the Cash App user.
2. Call `bridge_to_cash_app` with the recipient, the route, the USD amount and the payer's refund address. The SDK checks the recipient and gets a quote from the cross-chain provider.
3. Show the payer the returned `payment_request`, and store the `bridge_id` and `status_token` to [track the bridge](./bridging.md#tracking-a-bridge). Once their deposit lands, the provider pays the recipient.

### Rust

```rust
// List the routes to the Cash App user and pick what the payer pays in,
// e.g. USDC on Base.
let recipient = "$alice".to_string();
let routes = client
    .get_routes(GetBridgeRoutesRequest {
        recipient: recipient.clone(),
    })
    .await?
    .routes;
let route = routes
    .into_iter()
    .find(|r| r.source.asset == "USDC" && r.source.chain == "base")
    .ok_or_else(|| anyhow::anyhow!("No route from USDC on Base"))?;

// Pay $10 of USDC to the Cash App user $alice. The amount is in USDC base
// units (6 decimals), so 10_000_000 = 10 USDC, about $10. The deposit is
// refunded to the payer's address if delivery fails.
let response = client
    .bridge_to_cash_app(CreateBridgeRequest {
        recipient,
        route,
        amount: 10_000_000,
        refund_address: Some("<payer address>".to_string()),
        fee_policy: None,
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
// List the routes to the Cash App user and pick what the payer pays in,
// e.g. USDC on Base.
let recipient = "$alice"
let routes = try await client.getRoutes(
    request: GetBridgeRoutesRequest(recipient: recipient)
).routes
guard
    let route = routes.first(where: {
        $0.source.asset == "USDC" && $0.source.chain == "base"
    })
else {
    throw NSError(domain: "CashApp", code: 2)
}

// Pay $10 of USDC to the Cash App user $alice. The amount is in USDC base
// units (6 decimals), so 10_000_000 = 10 USDC, about $10. The deposit is
// refunded to the payer's address if delivery fails.
let response = try await client.bridgeToCashApp(
    request: CreateBridgeRequest(
        recipient: recipient,
        route: route,
        amount: BInt(10_000_000),
        refundAddress: "<payer address>",
        feePolicy: nil,
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
// List the routes to the Cash App user and pick what the payer pays in,
// e.g. USDC on Base.
val recipient = "\$alice"
val routes = client.getRoutes(GetBridgeRoutesRequest(recipient = recipient)).routes
val route = routes.find { it.source.asset == "USDC" && it.source.chain == "base" }
    ?: throw IllegalArgumentException("No route from USDC on Base")

// Pay $10 of USDC to the Cash App user $alice. The amount is in USDC base
// units (6 decimals), so 10_000_000 = 10 USDC, about $10. The deposit is
// refunded to the payer's address if delivery fails.
val response = client.bridgeToCashApp(
    CreateBridgeRequest(
        recipient = recipient,
        route = route,
        amount = BigInteger.fromLong(10_000_000L),
        refundAddress = "<payer address>",
        feePolicy = null,
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
// List the routes to the Cash App user and pick what the payer pays in,
// e.g. USDC on Base.
var recipient = "$alice";
var routes = (await client.GetRoutes(
    request: new GetBridgeRoutesRequest(recipient: recipient)
)).routes;
var route = routes.First(
    r => r.source.asset == "USDC" && r.source.chain == "base"
);

// Pay $10 of USDC to the Cash App user $alice. The amount is in USDC base
// units (6 decimals), so 10_000_000 = 10 USDC, about $10. The deposit is
// refunded to the payer's address if delivery fails.
var request = new CreateBridgeRequest(
    recipient: recipient,
    route: route,
    amount: new BigInteger(10_000_000),
    refundAddress: "<payer address>",
    feePolicy: null,
    maxSlippageBps: null
);
var response = await client.BridgeToCashApp(request: request);

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
// List the routes to the Cash App user and pick what the payer pays in,
// e.g. USDC on Base.
const recipient = '$alice'
const { routes } = await client.getRoutes({ recipient })
const route = routes.find((r) => r.source.asset === 'USDC' && r.source.chain === 'base')
if (route === undefined) {
  throw new Error('No route from USDC on Base')
}

// Pay $10 of USDC to the Cash App user $alice. The amount is in USDC base
// units (6 decimals), so 10_000_000 = 10 USDC, about $10. The deposit is
// refunded to the payer's address if delivery fails.
const response = await client.bridgeToCashApp({
  recipient,
  route,
  amount: BigInt(10_000_000),
  refundAddress: '<payer address>',
  feePolicy: undefined,
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
// List the routes to the Cash App user and pick what the payer pays in,
// e.g. USDC on Base.
const recipient = '$alice'
const { routes } = await client.getRoutes({ recipient })
const route = routes.find((r) => r.source.asset === 'USDC' && r.source.chain === 'base')
if (route === undefined) {
  throw new Error('No route from USDC on Base')
}

// Pay $10 of USDC to the Cash App user $alice. The amount is in USDC base
// units (6 decimals), so 10_000_000 = 10 USDC, about $10. The deposit is
// refunded to the payer's address if delivery fails.
const response = await client.bridgeToCashApp({
  recipient,
  route,
  amount: BigInt(10_000_000),
  refundAddress: '<payer address>',
  feePolicy: undefined,
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
// List the routes to the Cash App user and pick what the payer pays in,
// e.g. USDC on Base.
String recipient = "\$alice";
final routesResponse = await client.getRoutes(
  request: GetBridgeRoutesRequest(recipient: recipient),
);
BridgeRoutePair route = routesResponse.routes.firstWhere(
  (r) => r.source.asset == "USDC" && r.source.chain == "base",
);

// Pay $10 of USDC to the Cash App user $alice. The amount is in USDC base
// units (6 decimals), so 10000000 = 10 USDC, about $10. The deposit is
// refunded to the payer's address if delivery fails.
final response = await client.bridgeToCashApp(
  request: CreateBridgeRequest(
    recipient: recipient,
    route: route,
    amount: BigInt.from(10000000),
    refundAddress: "<payer address>",
    feePolicy: null,
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
    # List the routes to the Cash App user and pick what the payer pays in,
    # e.g. USDC on Base.
    recipient = "$alice"
    routes_response = await client.get_routes(
        request=GetBridgeRoutesRequest(recipient=recipient)
    )
    route = next(
        (
            r
            for r in routes_response.routes
            if r.source.asset == "USDC" and r.source.chain == "base"
        ),
        None,
    )
    if route is None:
        raise ValueError("No route from USDC on Base")

    # Pay $10 of USDC to the Cash App user $alice. The amount is in USDC base
    # units (6 decimals), so 10_000_000 = 10 USDC, about $10. The deposit is
    # refunded to the payer's address if delivery fails.
    request = CreateBridgeRequest(
        recipient=recipient,
        route=route,
        amount=10_000_000,
        refund_address="<payer address>",
        fee_policy=None,
        max_slippage_bps=None,
    )
    response = await client.bridge_to_cash_app(request=request)

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
// List the routes to the Cash App user and pick what the payer pays in,
// e.g. USDC on Base.
recipient := "$alice"
routesResponse, err := client.GetRoutes(breez_sdk_spark.GetBridgeRoutesRequest{
	Recipient: recipient,
})
if err != nil {
	return nil, err
}
routes := routesResponse.Routes
var route *breez_sdk_spark.BridgeRoutePair
for i := range routes {
	if routes[i].Source.Asset == "USDC" && routes[i].Source.Chain == "base" {
		route = &routes[i]
		break
	}
}
if route == nil {
	return nil, errors.New("no route from USDC on Base")
}

// Pay $10 of USDC to the Cash App user $alice. The amount is in USDC base
// units (6 decimals), so 10_000_000 = 10 USDC, about $10. The deposit is
// refunded to the payer's address if delivery fails.
refundAddress := "<payer address>"
response, err := client.BridgeToCashApp(breez_sdk_spark.CreateBridgeRequest{
	Recipient:      recipient,
	Route:          *route,
	Amount:         new(big.Int).SetInt64(10_000_000),
	RefundAddress:  &refundAddress,
	FeePolicy:      nil,
	MaxSlippageBps: nil,
})
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

The `recipient` is a Cash App username, given as `$alice`, or the user's Cash App Lightning address, `alice@cash.app`. Both reach the same user, who is paid at that Lightning address. `bridge_to_cash_app` also takes the bare `alice`, but `get_routes` needs the `$`.

`get_routes` looks the user up, so an unknown username fails before anything is quoted. `bridge_to_cash_app` fails with `SdkError::InvalidInput` for a recipient who isn't a Cash App user, or an amount outside what the recipient accepts.

## Refund address

The `refund_address` is the payer's own address on the source chain. If the recipient can't be paid, the provider refunds the deposit there. It must belong to the `source` chain's family (e.g. an EVM `0x...` address for USDC on Base).

## Response fields

`bridge_to_cash_app` returns a `CreateBridgeResponse`, described in [Bridging USDC/USDT](./bridging.md#response-fields). Its `payment_request` is what the payer pays: an EIP-681 URI on EVM chains, otherwise the bare deposit address.

## Limitations

- **Delivery can still fail after funding.** The provider looks the recipient up again when it pays. If the account was closed, its limit lowered, or slippage moved the amount across a limit since the quote, the payment fails and the deposit is refunded to `refund_address`.
- **Lightning delivery only.** A completed bridge means the Lightning payment was delivered. Whether Cash App credits it as Bitcoin or converts it to dollars depends on the recipient's Cash App settings.

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
