# Paying with x402

The SDK pays [x402](x402.md) challenges over Lightning, for both HTTP requests and MCP tool calls. To charge for your own resources instead, see [Charging with x402](x402_charging.md).

## Paying for an HTTP request

A server that requires payment answers with status `402` and a `PAYMENT-REQUIRED` header. Pass that header to {{#name pay}} together with the request it answered. The challenge commits to the exact request, so describe it as it will be sent again: the method, the absolute URL, the headers and the raw body. Include headers your HTTP client adds on its own, such as `Accept`. A server can require them, and a challenge that doesn't match the request is refused. If your HTTP client followed a redirect to the challenge, pay it only when the redirect stayed on the same origin and the request is a `GET` or `HEAD`, and describe the request to the URL that answered: otherwise your client may have changed the method or dropped headers on the way. Send the paid request to that URL without following redirects, so the payment goes only to the server that charged.

Before anything is paid, the challenge is checked: it must be for this request, from the server's advertised payee, unexpired, and cost no more than {{#name max_amount}}. Give the most you'll spend over Lightning, routing fee included, as {{#enum X402PaymentAmount::Lightning}}. Once paid, send the request again with a `PAYMENT-SIGNATURE` header set to {{#name payment_signature}}.

{{#tabs x402:pay-http-request}}

## Paying for an MCP tool call

An MCP server returns the challenge in the `structuredContent` of a tool result, and as text in `content[0]`. Pass that JSON to {{#name pay}} together with the tool call's `params`, and with {{#name server}} set to the URI of the MCP server you connected to, from your own configuration. Never take it from the challenge: the check that the payment goes to the server you meant to pay relies on it. For a tool call, {{#name payment_signature}} is JSON: call the tool again with it under `params._meta["x402/payment"]`.

{{#tabs x402:pay-mcp-tool-call}}

## Pending payments

If the payment hasn't completed within {{#name completion_timeout_secs}} (30 seconds by default), {{#name pay}} returns {{#enum X402PaymentStatus::Pending}}. Paying the same challenge again waits on that payment instead of paying twice. That works after the challenge expires too, but the server accepts the payment only until shortly past its expiry (60 seconds by default), so resume promptly. Don't send the request again to retry: the server answers with a new challenge, and paying it would pay a second time.

## Paying from a token balance

To pay from a token balance such as USDB, set {{#name conversion_options}} and the tokens are converted to sats before paying. With [stable balance](stable_balance.md) active, this happens automatically when the sats balance is too low. {{#name max_amount}} limits the Lightning payment, while the conversion's cost is limited by its slippage.

## Refused challenges

A challenge that fails a check, or costs more than {{#name max_amount}}, is refused with an {{#enum SdkError::X402Error}} of kind {{#enum X402ErrorKind::Refused}}, and nothing is paid. Its {{#name reason}} is the x402 reason, such as `invalid_exact_lnbtc_invoice_expired` or `invalid_exact_lnbtc_request_mismatch`. A payment that was sent and failed has kind {{#enum X402ErrorKind::PaymentFailed}}, and nothing was paid either. Other errors can come after the invoice was paid, so check your payments before paying a new challenge for the same request. See [X402Error](errors.md#x402-error).

## Using fetch in JavaScript

The npm package includes `withX402`, which wraps `fetch` so that an x402 challenge is paid and the request sent again with the payment. It limits what each request and, optionally, all requests together may spend, and which hosts it pays.

```typescript
import { withX402 } from '@breeztech/breez-sdk-spark/x402-fetch'

const x402Fetch = withX402(fetch, sdk.getX402(), {
  maxAmountSats: 100,
  budgetSats: 10_000,
  allowedHosts: ['api.example.com']
})
const response = await x402Fetch('https://api.example.com/articles/1')
```

A payment still pending after a few attempts throws `X402PaymentPendingError`, which carries what's needed to resume it without paying twice. So does a failure after the payment may have been sent, or an error such as a `500` in answer to the paid request. A paid request the server refuses throws `X402PaymentRefusedError` with the server's reason. Either way the payment may have been made, so don't simply fetch again. A request that already carries a `PAYMENT-SIGNATURE` header is sent as is, and never paid again.

`withX402` pays only over `https`, apart from loopback addresses, unless you set `allowHttp`. A challenge reached through a redirect is paid only for a `GET` or `HEAD` that stayed on the same origin.
