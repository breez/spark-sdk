# Charging with x402

You can charge for HTTP resources and MCP tools with [x402](x402.md) payments over Lightning. When a request arrives without a payment, create a challenge and answer with it. When the request comes back with a payment, settle the payment and serve the request only if settlement succeeds.

## Creating a challenge

Call {{#name create_challenge}} with the unpaid request and its price in {{#name amounts}}, one amount for each way it can be paid. Lightning is the only one today, priced in sats with {{#enum X402PaymentAmount::Lightning}}. To charge a fiat price, convert it with the rates from {{#name list_fiat_rates}}: the payment settles at the challenge's price even if the rate moves before it comes back. Describe the request as it arrived: the method, the headers, the raw body, and the URL. Build the URL from your server's configured public origin, never from the request's `Host` or forwarding headers, which the client controls.

The payment commits to the method, URL and body. Use {{#name bound_names}} to commit it to the headers that change what is served or to whom, such as `authorization` or `accept`, so a payment for one variant can't be used for another. Give them in lowercase, sorted.

Answer with status `402` and a `PAYMENT-REQUIRED` header set to {{#name payment_required}}. For an MCP tool call, {{#name payment_required}} is JSON: return a tool result with `isError` set, with it as both its `structuredContent` and the text of `content[0]`.

{{#tabs x402:create-challenge}}

## Settling a payment

When the request comes back with a `PAYMENT-SIGNATURE` header, pass it to {{#name settle}} with the request as it arrived and the same bound names. For an MCP tool call, pass the JSON in `params._meta["x402/payment"]`.

Settlement checks that the payment is for an invoice the SDK issued for this same request, that it carries the invoice's preimage, and that the invoice was paid to you. It then marks the payment as used, so it can't pay for another request. Serve the request only when {{#name success}} is true, and send a `PAYMENT-RESPONSE` header set to {{#name payment_response}} either way. For an MCP tool call, {{#name payment_response}} is JSON for the result's `_meta["x402/payment-response"]`. When settlement fails, {{#name error_reason}} says why. Answer with a new challenge, unless the reason is `exact_lnbtc_invoice_not_settled` or `exact_lnbtc_receiver_unavailable`: the payment may still settle, so the client should send the same payment again rather than pay a new invoice. Over HTTP, answer a payment that can't be read (`invalid_payload`, `invalid_x402_version`, or a missing or malformed preimage) with status `400` instead.

{{#tabs x402:settle-payment}}

## Using middleware in JavaScript

The npm package includes middleware that creates challenges and settles payments for you. `x402Hono` works with Hono, and `x402Express` with Express, where routes with a body need `express.raw()` mounted first, since the payment commits to the raw bytes. `requirePayment` does the same for any server built on the Fetch API's `Request` and `Response`.

```typescript
import { x402Hono } from '@breeztech/breez-sdk-spark/x402-server'

const x402 = sdk.getX402()
app.use('/articles/*', x402Hono(x402, {
  publicOrigin: 'https://api.example.com',
  priceSats: 21,
  boundHeaders: ['accept']
}))
```

## Serving browsers

Browser code on another origin can read the x402 headers and send `PAYMENT-SIGNATURE` only if your server allows it with CORS. Your CORS setup decides which origins may call you, and its answer to preflight requests must allow the `PAYMENT-SIGNATURE` request header, plus `Content-Type` when requests carry a JSON body. The JavaScript middleware adds `PAYMENT-REQUIRED` and `PAYMENT-RESPONSE` to `Access-Control-Expose-Headers` and passes preflight requests on to your CORS handling. If you call {{#name create_challenge}} and {{#name settle}} yourself, expose those two headers on your responses.

## Running more than one server

Each challenge carries a quote the SDK signs, and settlement accepts only its own quotes. Every Spark invoice is signed by the same service provider key, so the quote is what tells your invoices apart from ones issued elsewhere. The quote key is derived from your signer, so every server instance built from the same seed accepts the others' quotes. If your signer can't compute HMACs, set an [x402 quote secret](customizing.md#with-x402-quote-secret).

Settlement also records each payment as used, so it can't pay for a second request. Every instance that settles payments for the same SDK must share these records. The storage backend keeps them, so with more than one instance, use PostgreSQL or MySQL, or provide your own [x402 replay store](customizing.md#with-x402-replay-store).

<div class="warning">
<h4>Developer note</h4>
Each challenge creates a Lightning invoice, including for requests that are never paid. Rate-limit unpaid requests before creating challenges, as you would any other endpoint that does work per request.
</div>
