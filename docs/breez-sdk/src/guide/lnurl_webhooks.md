# Lightning Address payment notifications

When one of your users receives a payment to their Lightning Address, Breez can send a webhook to your server. Payments are received automatically without any user interaction — the webhook simply lets you know it happened. For example, you could send the user a push notification, update a balance in your backend, trigger a fulfillment flow, or log the event for analytics.

## How it works

Your users' Lightning Addresses are served by the Breez LNURL server. When a payment comes in, the LNURL server sends a webhook to your server.

![Webhook flow](images/lnurl_webhook_flow.svg)

As an example, if you want to send push notifications to your users, you could run a Notification Delivery Service (NDS) that receives the webhook and forwards a push notification to the user's device:

![NDS push notification flow](images/lnurl_webhook_nds.svg)

## Getting started

To start receiving webhooks, provide your webhook endpoint URL to Breez. Breez will configure it for your domain so that all Lightning Address payments on that domain trigger a POST request to your endpoint.

Your endpoint should accept `POST` requests with a JSON body and respond with a `2xx` status code to acknowledge receipt.

## Payload

All payloads use a `{ "template": "...", "data": { ... } }` envelope. Currently the only template is `payment_received`:

```json
{
  "template": "payment_received",
  "data": {
    "paymentHash": "abc123...",
    "invoice": "lnbc50u1p...",
    "preimage": "def456...",
    "amountSat": 50000,
    "userPubkey": "02abc123...",
    "lightningAddress": "alice@yourdomain.com",
    "senderComment": "Thanks!",
    "timestamp": 1711929600000
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `paymentHash` | `string` | Hex-encoded payment hash |
| `invoice` | `string` | BOLT11 invoice that was paid |
| `preimage` | `string` | Hex-encoded payment preimage |
| `amountSat` | `number \| null` | Amount received in satoshis. May be `null` in rare cases where the amount is not available. |
| `userPubkey` | `string` | The Spark identity public key of the user who received the payment |
| `lightningAddress` | `string \| null` | The Lightning Address that received the payment (e.g. `alice@yourdomain.com`) |
| `senderComment` | `string \| null` | Comment attached by the sender, if any |
| `timestamp` | `number` | Milliseconds since Unix epoch when the webhook was enqueued |

## Retries

If your endpoint is unreachable or responds with a non-2xx status code, Breez will automatically retry delivery with exponential backoff. Because of this, your endpoint may receive the same webhook more than once for the same payment — use the `paymentHash` field to deduplicate.

## Best practices

- **Return 2xx quickly.** Do your processing asynchronously after acknowledging the webhook. Slow responses will be treated as failures and retried.
- **Deduplicate on `paymentHash`.** The same payment may be delivered more than once due to retries.
- **Use `lightningAddress` or `userPubkey` to identify the user.** These fields tell you which user received the payment.
