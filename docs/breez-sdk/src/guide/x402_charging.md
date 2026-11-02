# Charging with x402

You can charge for HTTP resources and MCP tools with [x402](x402.md) payments over Lightning. When a request arrives without a payment, create a challenge and answer with it. When the request comes back with a payment, settle the payment and serve the request only if settlement succeeds.

## Creating a challenge

Call {{#name create_challenge}} with the unpaid request and its price in {{#name amounts}}, one amount for each way it can be paid. Lightning is the only one today, priced in sats with {{#enum X402PaymentAmount::Lightning}}. Describe the request as it arrived: the method, the headers, the raw body, and the URL. Build the URL from your server's configured public origin, never from the request's `Host` or forwarding headers, which the client controls.

The payment commits to the method, URL and body. Use {{#name bound_names}} to commit it to the headers that change what is served or to whom, such as `authorization` or `accept`, so a payment for one variant can't be used for another. Bind every header your price depends on too, so a client can't pay one price and ask for something that costs more. For an MCP tool call, the names are `params._meta` members, which are case-sensitive, and the server URI must be exactly the one your clients connect to. A request that can't be bound to a payment, such as one whose URL has characters outside URI syntax, is refused with an {{#enum SdkError::X402Error}} of kind {{#enum X402ErrorKind::Refused}}: answer it with status `400`.

Answer with status `402` and a `PAYMENT-REQUIRED` header set to {{#name payment_required}}. For an MCP tool call, {{#name payment_required}} is JSON: return a tool result with `isError` set, with it as both its `structuredContent` and the text of `content[0]`.

{{#tabs x402:create-challenge}}

## Settling a payment

When the request comes back with a `PAYMENT-SIGNATURE` header, pass it to {{#name settle}} with the request as it arrived, the same bound names, and the same price and {{#name max_timeout_secs}} you create its challenges with. For an MCP tool call, pass the JSON in `params._meta["x402/payment"]`.

Settlement checks that the payment is for an invoice the SDK issued for this same request at your current price, that it carries the invoice's preimage, and that the invoice was paid to you. It then marks the payment as used, so it can't pay for another request. A payment made at another price is refused, so keep a price stable while its challenges can still be paid. To charge a fiat price, for example, convert it at a rate you hold for at least {{#name max_timeout_secs}}.

The SDK settles payments itself, in place of an x402 facilitator. Don't settle these payments with a standard facilitator: it wouldn't check the quote each challenge carries (see [Running more than one server](#running-more-than-one-server)), so it would accept invoices other Spark users issued.

Send a `PAYMENT-RESPONSE` header set to {{#name payment_response}} whatever the outcome, and answer according to {{#name status}}. For an MCP tool call, {{#name payment_response}} is JSON for the result's `_meta["x402/payment-response"]`.

- {{#enum X402SettleStatus::Settled}}: serve the request.
- {{#enum X402SettleStatus::Pending}}: the payment is valid, but its receipt isn't confirmed yet. Answer with status `402` and no new challenge. Its reason, `settlement_pending`, is one the x402 specification marks as not final, so clients that follow it, the SDK's included, send the same payment again rather than paying a new invoice.
- {{#enum X402SettleStatus::Malformed}}: the payment can't be read. Over HTTP, answer with status `400`.
- {{#enum X402SettleStatus::Refused}}: answer with a new challenge. {{#name error_reason}} says why.
- {{#enum X402SettleStatus::Error}}: settling failed on your side, for example because the replay store couldn't be reached. The payment stays unused, so the client can send it again. Over HTTP, answer with status `500`.

For an MCP tool call, answer every status other than {{#enum X402SettleStatus::Settled}} with a tool result that has `isError` set. For {{#enum X402SettleStatus::Malformed}} and {{#enum X402SettleStatus::Refused}}, that is a new challenge, as for an unpaid call. For {{#enum X402SettleStatus::Pending}} and {{#enum X402SettleStatus::Error}}, return no new challenge, so the client can't pay twice: put {{#name payment_response}} in `_meta["x402/payment-response"]` and a message in `content[0]`.

{{#tabs x402:settle-payment}}

## Using middleware in JavaScript

The npm package includes middleware that creates challenges and settles payments for you. `x402Hono` works with Hono, and `x402Express` with Express. `requirePayment` charges for a single request on any server built on the Fetch API's `Request` and `Response`. The middleware answers CORS preflight requests without charging and charges every other request that reaches it, whatever its method, so mount your CORS handling before it (see [Serving browsers](#serving-browsers)). It charges HTTP requests, so don't mount it in front of an MCP endpoint: a tool call's payment commits to the call itself, even over HTTP. Shared caches don't keep its responses: challenges are marked `no-store` and paid responses `private`. With `requirePayment`, mark paid responses `private` yourself.

The payment commits to the body bytes as sent, so the middleware must see them, up to `maxBodyBytes` (1 MiB by default): a larger body is answered with status `413`. Mount it before anything that reads the body. With Express, `x402Express` reads the body itself when no parser has run before it. To keep a parser such as `express.json()` in front of it, have the parser keep the raw bytes: `express.json({ verify: (req, res, buf) => { req.rawBody = buf }, inflate: false })`.

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

Browser code on another origin can read the x402 headers and send `PAYMENT-SIGNATURE` only if your server allows it with CORS. Your CORS setup decides which origins may call you, and its answer to preflight requests must allow the `PAYMENT-SIGNATURE` request header, plus `Content-Type` when requests carry a JSON body, and any other header your clients send. Some releases of the TypeScript x402 client, for one, also send `Access-Control-Expose-Headers` with the paid request. Mount your CORS handling before `x402Hono` or `x402Express`, or answer preflight requests before calling `requirePayment`.

The middleware adds `PAYMENT-REQUIRED` and `PAYMENT-RESPONSE` to `Access-Control-Expose-Headers`. `requirePayment` does so on the responses it returns, and on a paid request, set `PAYMENT-RESPONSE` yourself and pass `Access-Control-Expose-Headers` through `exposeX402Headers`. If you call {{#name create_challenge}} and {{#name settle}} yourself, expose those two headers on your responses.

## Running more than one server

Each challenge carries a quote the SDK signs, and settlement accepts only its own quotes. Every Spark invoice is signed by the same service provider key, so the quote is what tells your invoices apart from ones issued elsewhere. The quote key is derived from your signer, so every server instance built from the same seed accepts the others' quotes. If your signer can't compute HMACs, set an [x402 quote secret](customizing.md#with-x402-quote-secret).

Settlement also records each payment as used, so it can't pay for a second request. Every instance that settles payments for the same SDK must share these records. The storage backend keeps them, so with more than one instance, use PostgreSQL or MySQL, or provide your own [x402 replay store](customizing.md#with-x402-replay-store).

<div class="warning">
<h4>Developer note</h4>
Each challenge creates a Lightning invoice, including for requests that are never paid. Rate-limit unpaid requests before creating challenges, as you would any other endpoint that does work per request.
</div>
