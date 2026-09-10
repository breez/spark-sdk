# Buying Bitcoin

The Breez SDK allows users to purchase Bitcoin through external providers. Two providers are currently supported: **MoonPay** and **CashApp**. The user is directed to a provider URL in their browser (or the CashApp app) to complete the purchase, and the funds are deposited directly into their wallet.

## MoonPay

MoonPay uses **on-chain Bitcoin deposit addresses** to receive purchased funds. It supports fiat-to-Bitcoin purchases via credit card, Apple Pay, Google Pay, and other payment methods.

To initiate a Bitcoin purchase via MoonPay:

### Rust

```rust
// Optionally, lock the purchase to a specific amount
let optional_locked_amount_sat = Some(100_000);
// Optionally, set a redirect URL for after the purchase is completed
let optional_redirect_url = Some("https://example.com/purchase-complete".to_string());

let request = BuyBitcoinRequest::Moonpay {
    locked_amount_sat: optional_locked_amount_sat,
    redirect_url: optional_redirect_url,
};

let response = sdk.buy_bitcoin(request).await?;
info!("Open this URL in a browser to complete the purchase:");
info!("{}", response.url);
```

### Swift

```swift
// Optionally, lock the purchase to a specific amount
let optionalLockedAmountSat: UInt64 = 100_000
// Optionally, set a redirect URL for after the purchase is completed
let optionalRedirectUrl = "https://example.com/purchase-complete"

let request = BuyBitcoinRequest.moonpay(
    lockedAmountSat: optionalLockedAmountSat,
    redirectUrl: optionalRedirectUrl
)

let response = try await sdk.buyBitcoin(request: request)
print("Open this URL in a browser to complete the purchase:")
print("\(response.url)")
```

### Kotlin

```kotlin
// Optionally, lock the purchase to a specific amount
val optionalLockedAmountSat: ULong = 100_000u
// Optionally, set a redirect URL for after the purchase is completed
val optionalRedirectUrl = "https://example.com/purchase-complete"

val request = BuyBitcoinRequest.Moonpay(
    lockedAmountSat = optionalLockedAmountSat,
    redirectUrl = optionalRedirectUrl
)

val response = sdk.buyBitcoin(request)
// Log.v("Breez", "Open this URL in a browser to complete the purchase:")
// Log.v("Breez", "${response.url}")
```

### C#

```csharp
// Optionally, lock the purchase to a specific amount
var optionalLockedAmountSat = (ulong)100000;
// Optionally, set a redirect URL for after the purchase is completed
var optionalRedirectUrl = "https://example.com/purchase-complete";

var request = new BuyBitcoinRequest.Moonpay(
    lockedAmountSat: optionalLockedAmountSat,
    redirectUrl: optionalRedirectUrl
);

var response = await sdk.BuyBitcoin(request: request);
Console.WriteLine("Open this URL in a browser to complete the purchase:");
Console.WriteLine($"{response.url}");
```

### Javascript (Wasm)

```typescript
// Optionally, lock the purchase to a specific amount
const optionalLockedAmountSat = 100_000
// Optionally, set a redirect URL for after the purchase is completed
const optionalRedirectUrl = 'https://example.com/purchase-complete'

const response = await sdk.buyBitcoin({
  type: 'moonpay',
  lockedAmountSat: optionalLockedAmountSat,
  redirectUrl: optionalRedirectUrl
})
console.log('Open this URL in a browser to complete the purchase:')
console.log(response.url)
```

### React Native

```typescript
// Optionally, lock the purchase to a specific amount
const optionalLockedAmountSat = BigInt(100_000)
// Optionally, set a redirect URL for after the purchase is completed
const optionalRedirectUrl = 'https://example.com/purchase-complete'

const request = new BuyBitcoinRequest.Moonpay({
  lockedAmountSat: optionalLockedAmountSat,
  redirectUrl: optionalRedirectUrl
})

const response = await sdk.buyBitcoin(request)
console.log('Open this URL in a browser to complete the purchase:')
console.log(response.url)
```

### Flutter

```dart
// Optionally, lock the purchase to a specific amount
final optionalLockedAmountSat = BigInt.from(100000);
// Optionally, set a redirect URL for after the purchase is completed
final optionalRedirectUrl = "https://example.com/purchase-complete";

final request = BuyBitcoinRequest_Moonpay(
    lockedAmountSat: optionalLockedAmountSat,
    redirectUrl: optionalRedirectUrl);

final response = await sdk.buyBitcoin(request: request);
print("Open this URL in a browser to complete the purchase:");
print(response.url);
```

### Python

```python
# Optionally, lock the purchase to a specific amount
optional_locked_amount_sat = 100_000
# Optionally, set a redirect URL for after the purchase is completed
optional_redirect_url = "https://example.com/purchase-complete"

try:
    request = BuyBitcoinRequest.MOONPAY(
        locked_amount_sat=optional_locked_amount_sat,
        redirect_url=optional_redirect_url,
    )

    response = await sdk.buy_bitcoin(request=request)
    logging.debug("Open this URL in a browser to complete the purchase:")
    logging.debug(response.url)
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
optionalLockedAmountSat := uint64(100_000)
optionalRedirectUrl := "https://example.com/purchase-complete"

request := breez_sdk_spark.BuyBitcoinRequestMoonpay{
	LockedAmountSat: &optionalLockedAmountSat,
	RedirectUrl:     &optionalRedirectUrl,
}

response, err := sdk.BuyBitcoin(request)
if err != nil {
	return err
}

log.Printf("Open this URL in a browser to complete the purchase:")
log.Printf("%v", response.Url)
```



The returned URL should be opened in a browser for the user to complete the purchase.

**Developer note**

MoonPay supports Apple Pay and Google Pay, but these payment methods will not work inside an iframe or standard web view. To ensure compatibility:

- **iOS**: Open the URL using <code>SFSafariViewController</code>.
- **Android**: Open the URL using <a href="https://developer.chrome.com/docs/android/custom-tabs" target="_blank">Chrome Custom Tabs</a>.
- **Desktop**: Apple Pay requires Safari; Google Pay requires Chrome.

## CashApp

CashApp uses **Lightning (bolt11 invoices)** to receive purchased funds. The caller specifies the amount in satoshis; the SDK generates a bolt11 invoice for that amount and returns a CashApp deep link (`cash.app/launch/lightning/...`) that opens CashApp so the user can complete payment.

**Developer notes**

<ul>
<li>CashApp is only available on <strong>mainnet</strong>. Using CashApp on testnet or regtest returns an error.</li>
<li>The amount is <strong>required</strong>. With an amountless invoice, Cash App only lets the payer fund from their existing Cash App BTC balance. When the invoice carries an amount, Cash App opens up funding via fiat balance and debit card.</li>
</ul>

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
