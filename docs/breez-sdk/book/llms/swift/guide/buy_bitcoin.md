# Buying Bitcoin

The Breez SDK allows users to purchase Bitcoin through external providers. Two providers are currently supported: **MoonPay** and **CashApp**. The user is directed to a provider URL in their browser (or the CashApp app) to complete the purchase, and the funds are deposited directly into their wallet. Buying Bitcoin is only available on mainnet.

## MoonPay

MoonPay supports fiat purchases via credit card, Apple Pay, Google Pay, and other payment methods. The request's `delivery` chooses how the purchase is delivered: as Bitcoin to an on-chain deposit address, or as USDC on Solana. Left unset, it is delivered as Bitcoin. Either way, the SDK returns a URL to open in a browser, where the user completes the purchase.

### Bitcoin delivery

MoonPay sends Bitcoin to an **on-chain deposit address**. The amount is optional. When set, it prefills the purchase.

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



### USDC on Solana delivery

MoonPay sends **USDC on Solana**, which settles much faster than on-chain Bitcoin. The USDC is converted on arrival and received as a Spark payment: as the token when a [stable balance](./stable_balance.md) is active, otherwise as Bitcoin. This requires [USDC/USDT receives](./config.md#usdc-usdt) to be enabled with `crossChainConfig`.

The `amount` is in USD at 6 decimals, so `1_000_000` is $1. By default this is the amount received, and the conversion fee is added on top of what MoonPay buys. With `feeMode` set to `CrossChainFeeMode.feesIncluded` the amount is what MoonPay buys and the fee is deducted from it.

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



The response's `crossChainInfo` describes the quote. Use it to show the user how much USDC MoonPay buys (`depositAmount`), what they can expect to receive (`expectedReceivedAmount` in `destinationAsset`), and the conversion fee (`serviceFeeAmount`). MoonPay's own fees are shown in its checkout.

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

```swift
// Cash App requires the amount to be specified up front.
let amountSats: UInt64 = 50_000

let request = BuyBitcoinRequest.cashApp(amountSats: amountSats)

let response = try await sdk.buyBitcoin(request: request)
print("Open this URL in Cash App to complete the purchase:")
print("\(response.url)")
```



The returned URL is a CashApp universal link (`https://cash.app/launch/lightning/<bolt11>`). On devices with CashApp installed it opens the app directly; otherwise it falls back to the CashApp website.

### Recommended UX

1. Collect a non-zero amount before calling `buyBitcoin`.
2. On mobile, redirect to the returned URL. On desktop, render it as a QR code and dismiss when `SdkEvent.paymentSucceeded` fires for the invoice.

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
