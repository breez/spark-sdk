# WebLN

The Breez SDK - Nodeless *(Spark Implementation)* includes WebLN support for mobile WebViews, allowing WebLN-aware websites to interact with the SDK through native app integrations.

## Overview

WebLN is a specification for Lightning Network wallet interactions in web browsers. The SDK provides platform integrations for:

- **Flutter** - `flutter_inappwebview` controller [*]
- **React Native / Expo** - `react-native-webview` component [*]
- **Android** - Native `WebView` controller
- **iOS** - `WKWebView` controller

[*] Required dependency

## Supported Methods

| Method | Description |
|--------|-------------|
| **enable()** | Request permission to interact with the SDK |
| **getInfo()** | Get the pubkey and supported methods |
| **sendPayment(invoice)** | Pay a BOLT11 invoice |
| **makeInvoice(args)** | Create a BOLT11 invoice |
| **signMessage(message)** | Sign a message |
| **verifyMessage(signature, message)** | Verify a signed message |
| **lnurl(lnurlString)** | Handle LNURL-pay, LNURL-withdraw, and LNURL-auth |

## Integration

Each platform requires implementing callback functions for user interactions. The SDK does **not** provide default UI - your app must implement permission dialogs, payment confirmations, and LNURL flows.

{{#tabs web_ln:webln-integration}}

## LNURL Support

The WebLN `lnurl()` method supports LNURL Auth, Pay and Withdraw. When a website calls `window.webln.lnurl(lnurlString)`, the controller parses the LNURL and extracts the parameters:

### LNURL-Auth

1. Your `onLnurlRequest` callback receives a request with:
   - `type`: {{#enum LnurlType::Auth}}
   - `domain`: Service domain
2. Show confirmation UI (no amount needed)
3. Return `{ approved: true }`
4. The controller completes the authentication

### LNURL-Pay

1. Your `onLnurlRequest` callback receives a request with:
   - `type`: {{#enum LnurlType::Pay}}
   - `domain`: Service domain
   - `minAmountSats` / `maxAmountSats`: Amount bounds
   - `metadata`: Service metadata
2. Show UI for user to select amount within bounds
3. Return `{ approved: true, amountSats: selectedAmount, comment: optionalComment }`
4. The controller completes the payment and returns the preimage

### LNURL-Withdraw

1. Your `onLnurlRequest` callback receives a request with:
   - `type`: {{#enum LnurlType::Withdraw}}
   - `domain`: Service domain
   - `minAmountSats` / `maxAmountSats`: Amount bounds
   - `defaultDescription`: Suggested description
2. Show UI for user to select amount
3. Return `{ approved: true, amountSats: selectedAmount }`
4. The controller creates an invoice and completes the withdrawal

## Security Considerations

- **Origin Validation**: The controller tracks enabled domains per session. Only domains that have been approved via `onEnableRequest` can call WebLN methods.
- **Payment Confirmation**: Always require explicit user confirmation for payments via the `onPaymentRequest` callback. Never auto-approve payments.
- **Amount Limits**: Consider implementing maximum auto-approve amounts in your callback logic for smaller payments, with additional confirmation for larger amounts.
- **Domain Trust**: Display the domain clearly to users when requesting WebLN access. Users should only approve domains they trust.

## WebLN Specification

For more details on the WebLN specification, see:
- [WebLN Documentation](https://www.webln.dev/docs/introduction)
- [WebLN GitHub](https://github.com/joule-labs/webln)
