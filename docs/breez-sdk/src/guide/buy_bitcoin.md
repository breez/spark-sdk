# Buying Bitcoin

The Breez SDK allows users to purchase Bitcoin through external providers. Two providers are currently supported: **MoonPay** and **CashApp**. The user is directed to a provider URL in their browser (or the CashApp app) to complete the purchase, and the funds are deposited directly into their wallet.

## MoonPay

MoonPay uses **on-chain Bitcoin deposit addresses** to receive purchased funds. It supports fiat-to-Bitcoin purchases via credit card, Apple Pay, Google Pay, and other payment methods.

To initiate a Bitcoin purchase via MoonPay:

{{#tabs buying_bitcoin:buy-bitcoin}}

The returned URL should be opened in a browser for the user to complete the purchase.

<div class="warning">
<h4>Developer note</h4>
MoonPay supports Apple Pay and Google Pay, but these payment methods will not work inside an iframe or standard web view. To ensure compatibility:

- **iOS**: Open the URL using <code>SFSafariViewController</code>.
- **Android**: Open the URL using <a href="https://developer.chrome.com/docs/android/custom-tabs" target="_blank">Chrome Custom Tabs</a>.
- **Desktop**: Apple Pay requires Safari; Google Pay requires Chrome.
</div>

## CashApp

CashApp uses **Lightning (bolt11 invoices)** to receive purchased funds. The SDK generates a Lightning invoice and returns a CashApp deep link (`cash.app/launch/lightning/...`) that opens the CashApp for the user to complete payment.

<div class="warning">
<h4>Developer note</h4>
CashApp is only available on <strong>mainnet</strong>. Attempting to use CashApp on testnet or regtest will return an error.
</div>

To initiate a Bitcoin purchase via CashApp:

{{#tabs buying_bitcoin:buy-bitcoin-cashapp}}

The returned URL is a CashApp universal link (`https://cash.app/launch/lightning/<bolt11>`). On devices with CashApp installed it opens the app directly; otherwise it falls back to the CashApp website.

<div class="warning">
<h4>Developer note</h4>

The URL is obtained <strong>after an async SDK call</strong>, which means <code>window.open()</code> will be blocked by popup blockers on most mobile browsers and PWAs.

**Recommended approach: pre-open a blank tab before the async call:**

```javascript
// 1. Open blank tab synchronously during the user gesture (click handler)
const newTab = window.open('', '_blank');

// 2. Async SDK call
const response = await sdk.buyBitcoin({ provider: 'cashApp' });

// 3. Navigate the pre-opened tab (or fall back to same-tab)
if (newTab) {
  newTab.location.href = response.url;
} else {
  // Mobile/PWA: popup was blocked, navigate in same tab
  window.location.href = response.url;
}
```

**Platform-specific guidance:**

| Platform | Behavior | Recommendation |
|----------|----------|----------------|
| **Desktop browsers** | Pre-opened tab works reliably | Use the pattern above |
| **Mobile browsers** | `window.open` may be blocked after async | Falls back to `location.href` automatically |
| **PWA (standalone)** | `window.open` is almost always blocked | Same-tab redirect via `location.href`; opens system browser |
| **iOS (native)** | Use `SFSafariViewController` or `UIApplication.open()` | Universal link triggers CashApp if installed |
| **Android (native)** | Use <a href="https://developer.chrome.com/docs/android/custom-tabs" target="_blank">Chrome Custom Tabs</a> or `Intent` | Universal link triggers CashApp if installed |

**CashApp availability:** US and UK only (excluding New York State for Bitcoin/Lightning features). CashApp handles region restrictions on their end, so no client-side gating is needed.
</div>
