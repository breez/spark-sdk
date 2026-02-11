# Buying Bitcoin

The Breez SDK allows users to purchase Bitcoin through external providers, such as Moonpay. The user is directed to a provider URL in their browser to complete the purchase, and the funds are deposited directly into their wallet.

To initiate a Bitcoin purchase:

{{#tabs buying_bitcoin:buy-bitcoin}}

The returned URL should be opened in a browser for the user to complete the purchase.

<div class="warning">
<h4>Developer note</h4>
MoonPay supports Apple Pay and Google Pay, but these payment methods will not work inside an iframe or standard web view. To ensure compatibility:

- **iOS**: Open the URL using <code>SFSafariViewController</code>.
- **Android**: Open the URL using <a href="https://developer.chrome.com/docs/android/custom-tabs" target="_blank">Chrome Custom Tabs</a>.
- **Desktop**: Apple Pay requires Safari; Google Pay requires Chrome.
</div>
