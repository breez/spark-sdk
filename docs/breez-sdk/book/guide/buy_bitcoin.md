# Buying Bitcoin

The Breez SDK allows users to purchase Bitcoin through external providers. Two providers are currently supported: **MoonPay** and **CashApp**. The user is directed to a provider URL in their browser (or the CashApp app) to complete the purchase, and the funds are deposited directly into their wallet. Buying Bitcoin is only available on mainnet.

## MoonPay

MoonPay supports fiat purchases via credit card, Apple Pay, Google Pay, and other payment methods. The request's `delivery` chooses how the purchase is delivered: as Bitcoin to an on-chain deposit address, or as USDC on Solana. Left unset, it is delivered as Bitcoin. Either way, the SDK returns a URL to open in a browser, where the user completes the purchase.

### Bitcoin delivery

MoonPay sends Bitcoin to an **on-chain deposit address**. The amount is optional. When set, it prefills the purchase.

#### Rust

```rust
// Optionally, prefill the purchase amount
let optional_amount_sat = Some(100_000);
// Optionally, set a redirect URL for after the purchase is completed
let optional_redirect_url = Some("https://example.com/purchase-complete".to_string());

let request = BuyBitcoinRequest::Moonpay {
    delivery: Some(MoonpayDelivery::Bitcoin {
        amount_sat: optional_amount_sat,
    }),
    redirect_url: optional_redirect_url,
};

let response = sdk.buy_bitcoin(request).await?;
info!("Open this URL in a browser to complete the purchase:");
info!("{}", response.url);
```

#### Swift

```swift
// Optionally, prefill the purchase amount
let optionalAmountSat: UInt64? = 100_000
// Optionally, set a redirect URL for after the purchase is completed
let optionalRedirectUrl: String? = "https://example.com/purchase-complete"

let request = BuyBitcoinRequest.moonpay(
    delivery: .bitcoin(amountSat: optionalAmountSat),
    redirectUrl: optionalRedirectUrl
)

let response = try await sdk.buyBitcoin(request: request)
print("Open this URL in a browser to complete the purchase:")
print("\(response.url)")
```

#### Kotlin

```kotlin
// Optionally, prefill the purchase amount
val optionalAmountSat: ULong? = 100_000u
// Optionally, set a redirect URL for after the purchase is completed
val optionalRedirectUrl: String? = "https://example.com/purchase-complete"

val request = BuyBitcoinRequest.Moonpay(
    delivery = MoonpayDelivery.Bitcoin(amountSat = optionalAmountSat),
    redirectUrl = optionalRedirectUrl
)

val response = sdk.buyBitcoin(request)
// Log.v("Breez", "Open this URL in a browser to complete the purchase:")
// Log.v("Breez", "${response.url}")
```

#### C#

```csharp
// Optionally, prefill the purchase amount
ulong? optionalAmountSat = 100_000;
// Optionally, set a redirect URL for after the purchase is completed
var optionalRedirectUrl = "https://example.com/purchase-complete";

var request = new BuyBitcoinRequest.Moonpay(
    delivery: new MoonpayDelivery.Bitcoin(amountSat: optionalAmountSat),
    redirectUrl: optionalRedirectUrl
);

var response = await sdk.BuyBitcoin(request: request);
Console.WriteLine("Open this URL in a browser to complete the purchase:");
Console.WriteLine($"{response.url}");
```

#### Javascript (Wasm)

```typescript
// Optionally, prefill the purchase amount
const optionalAmountSat = 100_000
// Optionally, set a redirect URL for after the purchase is completed
const optionalRedirectUrl = 'https://example.com/purchase-complete'

const response = await sdk.buyBitcoin({
  type: 'moonpay',
  delivery: {
    type: 'bitcoin',
    amountSat: optionalAmountSat
  },
  redirectUrl: optionalRedirectUrl
})
console.log('Open this URL in a browser to complete the purchase:')
console.log(response.url)
```

#### React Native

```typescript
// Optionally, prefill the purchase amount
const optionalAmountSat = BigInt(100_000)
// Optionally, set a redirect URL for after the purchase is completed
const optionalRedirectUrl = 'https://example.com/purchase-complete'

const request = new BuyBitcoinRequest.Moonpay({
  delivery: new MoonpayDelivery.Bitcoin({ amountSat: optionalAmountSat }),
  redirectUrl: optionalRedirectUrl
})

const response = await sdk.buyBitcoin(request)
console.log('Open this URL in a browser to complete the purchase:')
console.log(response.url)
```

#### Flutter

```dart
// Optionally, prefill the purchase amount
BigInt? optionalAmountSat = BigInt.from(100000);
// Optionally, set a redirect URL for after the purchase is completed
String? optionalRedirectUrl = "https://example.com/purchase-complete";

final request = BuyBitcoinRequest_Moonpay(
  delivery: MoonpayDelivery_Bitcoin(amountSat: optionalAmountSat),
  redirectUrl: optionalRedirectUrl,
);

final response = await sdk.buyBitcoin(request: request);
print("Open this URL in a browser to complete the purchase:");
print(response.url);
```

#### Python

```python
# Optionally, prefill the purchase amount
optional_amount_sat = 100_000
# Optionally, set a redirect URL for after the purchase is completed
optional_redirect_url = "https://example.com/purchase-complete"

try:
    request = BuyBitcoinRequest.MOONPAY(
        delivery=MoonpayDelivery.BITCOIN(
            amount_sat=optional_amount_sat,
        ),
        redirect_url=optional_redirect_url,
    )

    response = await sdk.buy_bitcoin(request=request)
    logging.debug("Open this URL in a browser to complete the purchase:")
    logging.debug(response.url)
except Exception as error:
    logging.error(error)
    raise
```

#### Go

```go
// Optionally, prefill the purchase amount
optionalAmountSat := uint64(100_000)
// Optionally, set a redirect URL for after the purchase is completed
optionalRedirectUrl := "https://example.com/purchase-complete"

var delivery breez_sdk_spark.MoonpayDelivery = breez_sdk_spark.MoonpayDeliveryBitcoin{
	AmountSat: &optionalAmountSat,
}
request := breez_sdk_spark.BuyBitcoinRequestMoonpay{
	Delivery:    &delivery,
	RedirectUrl: &optionalRedirectUrl,
}

response, err := sdk.BuyBitcoin(request)
if err != nil {
	return err
}

log.Printf("Open this URL in a browser to complete the purchase:")
log.Printf("%v", response.Url)
```



### USDC on Solana delivery

MoonPay sends **USDC on Solana**, which settles much faster than on-chain Bitcoin. The USDC is converted on arrival and received as a Spark payment: as the token when a [stable balance](./stable_balance.md) is active, otherwise as Bitcoin. This requires [USDC/USDT receives](./config.md#usdc-usdt) to be enabled with `cross_chain_config`.

The `amount` is in USD at 6 decimals, so `1_000_000` is $1. By default this is the amount received, and the conversion fee is added on top of what MoonPay buys. With `fee_mode` set to `CrossChainFeeMode::FeesIncluded` the amount is what MoonPay buys and the fee is deducted from it.

#### Rust

```rust
// USD amount to receive, in 6-decimal base units ($50)
let amount = 50_000_000;

let request = BuyBitcoinRequest::Moonpay {
    delivery: Some(MoonpayDelivery::CrossChain {
        amount,
        fee_mode: None,
    }),
    redirect_url: None,
};

let response = sdk.buy_bitcoin(request).await?;
info!("Open this URL in a browser to complete the purchase:");
info!("{}", response.url);

if let Some(info) = response.cross_chain_info {
    info!("USDC to buy: {}", info.deposit_amount);
    info!(
        "Expected to receive: {} {}",
        info.expected_received_amount, info.destination_asset
    );
    info!("Conversion fee: {}", info.service_fee_amount);
}
```

#### Swift

```swift
// USD amount to receive, in 6-decimal base units ($50)
let amount = BInt(50_000_000)

let request = BuyBitcoinRequest.moonpay(
    delivery: .crossChain(amount: amount, feeMode: nil),
    redirectUrl: nil
)

let response = try await sdk.buyBitcoin(request: request)
print("Open this URL in a browser to complete the purchase:")
print("\(response.url)")

if let info = response.crossChainInfo {
    print("USDC to buy: \(info.depositAmount)")
    print(
        "Expected to receive: \(info.expectedReceivedAmount) "
            + "\(info.destinationAsset)"
    )
    print("Conversion fee: \(info.serviceFeeAmount)")
}
```

#### Kotlin

```kotlin
// USD amount to receive, in 6-decimal base units ($50)
val amount = BigInteger.fromLong(50_000_000L)

val request = BuyBitcoinRequest.Moonpay(
    delivery = MoonpayDelivery.CrossChain(
        amount = amount,
        feeMode = null
    ),
    redirectUrl = null
)

val response = sdk.buyBitcoin(request)
// Log.v("Breez", "Open this URL in a browser to complete the purchase:")
// Log.v("Breez", "${response.url}")

response.crossChainInfo?.let { info ->
    // Log.v("Breez", "USDC to buy: ${info.depositAmount}")
    // Log.v("Breez", "Expected to receive: ${info.expectedReceivedAmount} " +
    //     "${info.destinationAsset}")
    // Log.v("Breez", "Conversion fee: ${info.serviceFeeAmount}")
}
```

#### C#

```csharp
// USD amount to receive, in 6-decimal base units ($50)
var amount = new BigInteger(50_000_000);

var request = new BuyBitcoinRequest.Moonpay(
    delivery: new MoonpayDelivery.CrossChain(
        amount: amount,
        feeMode: null
    ),
    redirectUrl: null
);

var response = await sdk.BuyBitcoin(request: request);
Console.WriteLine("Open this URL in a browser to complete the purchase:");
Console.WriteLine($"{response.url}");

if (response.crossChainInfo is { } info)
{
    Console.WriteLine($"USDC to buy: {info.depositAmount}");
    Console.WriteLine(
        "Expected to receive: "
            + $"{info.expectedReceivedAmount} {info.destinationAsset}"
    );
    Console.WriteLine($"Conversion fee: {info.serviceFeeAmount}");
}
```

#### Javascript (Wasm)

```typescript
// USD amount to receive, in 6-decimal base units ($50)
const amount = BigInt(50_000_000)

const response = await sdk.buyBitcoin({
  type: 'moonpay',
  delivery: {
    type: 'crossChain',
    amount,
    feeMode: undefined
  },
  redirectUrl: undefined
})
console.log('Open this URL in a browser to complete the purchase:')
console.log(response.url)

if (response.crossChainInfo !== undefined) {
  const info = response.crossChainInfo
  console.log(`USDC to buy: ${info.depositAmount}`)
  console.log(
    `Expected to receive: ${info.expectedReceivedAmount} ${info.destinationAsset}`
  )
  console.log(`Conversion fee: ${info.serviceFeeAmount}`)
}
```

#### React Native

```typescript
// USD amount to receive, in 6-decimal base units ($50)
const amount = BigInt(50_000_000)

const request = new BuyBitcoinRequest.Moonpay({
  delivery: new MoonpayDelivery.CrossChain({ amount, feeMode: undefined }),
  redirectUrl: undefined
})

const response = await sdk.buyBitcoin(request)
console.log('Open this URL in a browser to complete the purchase:')
console.log(response.url)

if (response.crossChainInfo !== undefined) {
  const info = response.crossChainInfo
  console.log(`USDC to buy: ${info.depositAmount}`)
  console.log(
    `Expected to receive: ${info.expectedReceivedAmount} ${info.destinationAsset}`
  )
  console.log(`Conversion fee: ${info.serviceFeeAmount}`)
}
```

#### Flutter

```dart
// USD amount to receive, in 6-decimal base units ($50)
final amount = BigInt.from(50000000);

final request = BuyBitcoinRequest_Moonpay(
  delivery: MoonpayDelivery_CrossChain(amount: amount, feeMode: null),
  redirectUrl: null,
);

final response = await sdk.buyBitcoin(request: request);
print("Open this URL in a browser to complete the purchase:");
print(response.url);

final info = response.crossChainInfo;
if (info != null) {
  print("USDC to buy: ${info.depositAmount}");
  print(
    "Expected to receive: "
    "${info.expectedReceivedAmount} ${info.destinationAsset}",
  );
  print("Conversion fee: ${info.serviceFeeAmount}");
}
```

#### Python

```python
# USD amount to receive, in 6-decimal base units ($50)
amount = 50_000_000

try:
    request = BuyBitcoinRequest.MOONPAY(
        delivery=MoonpayDelivery.CROSS_CHAIN(
            amount=amount,
            fee_mode=None,
        ),
        redirect_url=None,
    )

    response = await sdk.buy_bitcoin(request=request)
    logging.debug("Open this URL in a browser to complete the purchase:")
    logging.debug(response.url)

    info = response.cross_chain_info
    if info is not None:
        logging.debug(f"USDC to buy: {info.deposit_amount}")
        logging.debug(
            f"Expected to receive: {info.expected_received_amount} "
            f"{info.destination_asset}"
        )
        logging.debug(f"Conversion fee: {info.service_fee_amount}")
except Exception as error:
    logging.error(error)
    raise
```

#### Go

```go
// USD amount to receive, in 6-decimal base units ($50)
amount := new(big.Int).SetInt64(50_000_000)

var delivery breez_sdk_spark.MoonpayDelivery = breez_sdk_spark.MoonpayDeliveryCrossChain{
	Amount:  amount,
	FeeMode: nil,
}
request := breez_sdk_spark.BuyBitcoinRequestMoonpay{
	Delivery:    &delivery,
	RedirectUrl: nil,
}

response, err := sdk.BuyBitcoin(request)
if err != nil {
	return err
}

log.Printf("Open this URL in a browser to complete the purchase:")
log.Printf("%v", response.Url)

if info := response.CrossChainInfo; info != nil {
	log.Printf("USDC to buy: %v", info.DepositAmount)
	log.Printf(
		"Expected to receive: %v %s",
		info.ExpectedReceivedAmount, info.DestinationAsset,
	)
	log.Printf("Conversion fee: %v", info.ServiceFeeAmount)
}
```



The response's `cross_chain_info` describes the quote. Use it to show the user how much USDC MoonPay buys (`deposit_amount`), what they can expect to receive (`expected_received_amount` in `destination_asset`), and the conversion fee (`service_fee_amount`). MoonPay's own fees are shown in its checkout.

These figures are estimates. The amount is a prefill the user can change in MoonPay, and a purchase completed after the quote expires is converted at the live rate. See [Quote expiry](./cross_chain.md#quote-expiry-1). MoonPay also enforces its own minimum order, which the user sees in the checkout.

**Developer note**

MoonPay supports Apple Pay and Google Pay, but these payment methods will not work inside an iframe or standard web view. To ensure compatibility:

- **iOS**: Open the URL using <code>SFSafariViewController</code>.
- **Android**: Open the URL using <a href="https://developer.chrome.com/docs/android/custom-tabs" target="_blank">Chrome Custom Tabs</a>.
- **Desktop**: Apple Pay requires Safari; Google Pay requires Chrome.

## CashApp

CashApp uses **Lightning (bolt11 invoices)** to receive purchased funds. The caller specifies the amount in satoshis; the SDK generates a bolt11 invoice for that amount and returns a CashApp deep link (`cash.app/launch/lightning/...`) that opens CashApp so the user can complete payment.

**Developer note**

The amount is <strong>required</strong>. With an amountless invoice, Cash App only lets the payer fund from their existing Cash App BTC balance. When the invoice carries an amount, Cash App opens up funding via fiat balance and debit card.

To initiate a Bitcoin purchase via CashApp:

### Rust

```rust
// Cash App requires the amount to be specified up front.
let amount_sats = 50_000;

let request = BuyBitcoinRequest::CashApp { amount_sats };

let response = sdk.buy_bitcoin(request).await?;
info!("Open this URL in Cash App to complete the purchase:");
info!("{}", response.url);
```

### Swift

```swift
// Cash App requires the amount to be specified up front.
let amountSats: UInt64 = 50_000

let request = BuyBitcoinRequest.cashApp(amountSats: amountSats)

let response = try await sdk.buyBitcoin(request: request)
print("Open this URL in Cash App to complete the purchase:")
print("\(response.url)")
```

### Kotlin

```kotlin
// Cash App requires the amount to be specified up front.
val amountSats: ULong = 50_000u

val request = BuyBitcoinRequest.CashApp(amountSats = amountSats)

val response = sdk.buyBitcoin(request)
// Log.v("Breez", "Open this URL in Cash App to complete the purchase:")
// Log.v("Breez", "${response.url}")
```

### C#

```csharp
// Cash App requires the amount to be specified up front.
var amountSats = (ulong)50_000;

var request = new BuyBitcoinRequest.CashApp(
    amountSats: amountSats
);

var response = await sdk.BuyBitcoin(request: request);
Console.WriteLine("Open this URL in Cash App to complete the purchase:");
Console.WriteLine($"{response.url}");
```

### Javascript (Wasm)

```typescript
// Cash App requires the amount to be specified up front.
const amountSats = 50_000

const response = await sdk.buyBitcoin({
  type: 'cashApp',
  amountSats
})
console.log('Open this URL in Cash App to complete the purchase:')
console.log(response.url)
```

### React Native

```typescript
// Cash App requires the amount to be specified up front.
const amountSats = BigInt(50_000)

const request = new BuyBitcoinRequest.CashApp({
  amountSats
})

const response = await sdk.buyBitcoin(request)
console.log('Open this URL in Cash App to complete the purchase:')
console.log(response.url)
```

### Flutter

```dart
// Cash App requires the amount to be specified up front.
final amountSats = BigInt.from(50000);

final request = BuyBitcoinRequest_CashApp(amountSats: amountSats);

final response = await sdk.buyBitcoin(request: request);
print("Open this URL in Cash App to complete the purchase:");
print(response.url);
```

### Python

```python
# Cash App requires the amount to be specified up front.
amount_sats = 50_000

try:
    request = BuyBitcoinRequest.CASH_APP(
        amount_sats=amount_sats,
    )

    response = await sdk.buy_bitcoin(request=request)
    logging.debug("Open this URL in Cash App to complete the purchase:")
    logging.debug(response.url)
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
// Cash App requires the amount to be specified up front.
amountSats := uint64(50_000)

request := breez_sdk_spark.BuyBitcoinRequestCashApp{
	AmountSats: amountSats,
}

response, err := sdk.BuyBitcoin(request)
if err != nil {
	return err
}

log.Printf("Open this URL in Cash App to complete the purchase:")
log.Printf("%v", response.Url)
```



The returned URL is a CashApp universal link (`https://cash.app/launch/lightning/<bolt11>`). On devices with CashApp installed it opens the app directly; otherwise it falls back to the CashApp website.

### Recommended UX

1. Collect a non-zero amount before calling `buy_bitcoin`.
2. On mobile, redirect to the returned URL. On desktop, render it as a QR code and dismiss when `SdkEvent::PaymentSucceeded` fires for the invoice.

### Popup blockers on the web

On web, `window.open()` called after `await sdk.buyBitcoin(...)` is typically blocked by mobile browsers and PWAs because it falls outside the original user gesture. Pre-open a blank tab synchronously inside the click handler, then navigate it once the URL is ready:

```javascript
// Open a placeholder tab during the user gesture so the browser
// allows it; we navigate it once the SDK returns.
const newTab = window.open('', '_blank');

// Generate the Cash App invoice for the chosen amount.
const response = await sdk.buyBitcoin({ type: 'cashApp', amountSats: 50_000 });

// Send the user to Cash App. If the placeholder was blocked, redirect
// the current tab. The OS opens Cash App via the universal link.
if (newTab) {
  newTab.location.href = response.url;
} else {
  window.location.href = response.url;
}
```

### Platform-specific guidance

| Platform | Behavior | Recommendation |
|----------|----------|----------------|
| **Desktop browsers** | Pre-opened tab works reliably; most desktops won't have CashApp installed | Render the CashApp URL as a QR for the user to scan on their phone |
| **Mobile browsers** | `window.open` may be blocked after async | Pre-open a tab (see above); falls back to `location.href` automatically |
| **PWA (standalone)** | `window.open` is almost always blocked | Same-tab redirect via `location.href`; opens system browser, which hands off to CashApp |
| **iOS (native)** | Universal link triggers CashApp if installed | Open via `UIApplication.open()` or `SFSafariViewController` |
| **Android (native)** | Universal link triggers CashApp if installed | Open via `Intent` or <a href="https://developer.chrome.com/docs/android/custom-tabs" target="_blank">Chrome Custom Tabs</a> |

**CashApp availability:** US and UK only (excluding New York State for Bitcoin/Lightning features). CashApp handles region restrictions on their end, so no client-side gating is needed.

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
