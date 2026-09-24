# Managing webhooks

Webhooks allow you to receive real-time notifications when events occur in your wallet, such as completed Lightning payments or on-chain deposits. The Spark service provider sends an HTTP POST request to your specified URL whenever a subscribed event occurs. Each webhook payload is signed using HMAC-SHA256 with the secret you provide during registration, allowing you to verify the authenticity of incoming notifications.

## Event types

The following event types are available for webhook subscriptions:

| Event type | Payload `type` | Description |
|-----------|----------------|-------------|
| `WebhookEventType.LightningReceiveFinished` | `SPARK_LIGHTNING_RECEIVE_FINISHED` | A Lightning receive finished |
| `WebhookEventType.LightningSendFinished` | `SPARK_LIGHTNING_SEND_FINISHED` | A Lightning send finished |
| `WebhookEventType.CoopExitFinished` | `SPARK_COOP_EXIT_FINISHED` | A cooperative exit finished |
| `WebhookEventType.StaticDepositFinished` | `SPARK_STATIC_DEPOSIT_FINISHED` | A static deposit claim finished |

## Webhook payload

When an event occurs, the Spark service provider sends an HTTP POST request to your webhook URL. The payload is a JSON object whose fields vary by event type. The request includes an `X-Spark-Signature` header containing the hex-encoded HMAC-SHA256 signature of the raw request body, computed using the secret you provided during registration.

All payloads share the following common fields:

| Field | Type | Description |
|-------|------|-------------|
| `id` | `string` | Unique identifier for the request |
| `created_at` | `string` | ISO 8601 timestamp of when the request was created |
| `updated_at` | `string` | ISO 8601 timestamp of the last update |
| `network` | `string` | The network: `MAINNET`, `REGTEST`, `SIGNET` or `TESTNET` |
| `request_status` | `string \| null` | Outcome of the request: `SUCCEEDED`, `FAILED` or `CANCELED`. Its type also allows `CREATED`, `IN_PROGRESS` and `UNKNOWN`. |
| `status` | `string` | Status of the request at the time of the event. Its values depend on the event type and are listed with each event below. |
| `type` | `string` | The event type, as listed under [Event types](#event-types) |
| `timestamp` | `string` | ISO 8601 timestamp of when this delivery attempt was sent |

An `amount` is an object with an integer `value` and the `unit` of that value, such as `SATOSHI` or `MILLISATOSHI`. Each payload contains every field listed for its event type.

### Lightning receive finished

The event is sent to the webhooks of the wallet that created the invoice. Lightning Address invoices are created by the LNURL server, so a wallet's own webhooks do not receive this event for its Lightning Address payments. See [Lightning Address payment notifications](lnurl_webhooks.md) for those payments.

| Field | Type | Description |
|-------|------|-------------|
| `status` | `string` | A `LightningReceiveRequestStatus` value: `INVOICE_CREATED`, `HTLC_RECEIVED`, `TRANSFER_CREATED`, `TRANSFER_CREATION_FAILED`, `PAYMENT_PREIMAGE_PENDING`, `PAYMENT_PREIMAGE_RECOVERED`, `PAYMENT_PREIMAGE_QUERYING_FAILED`, `PAYMENT_PREIMAGE_RECOVERING_FAILED`, `TRANSFER_CANCELED`, `HTLC_FAILED`, `LIGHTNING_PAYMENT_RECEIVED`, `TRANSFER_FAILED`, `TRANSFER_COMPLETED`, `REFUND_SIGNING_COMMITMENTS_QUERYING_FAILED`, `REFUND_SIGNING_FAILED` |
| `payment_preimage` | `string \| null` | Hex-encoded payment preimage |
| `receiver_identity_public_key` | `string \| null` | Hex-encoded identity public key of the receiving wallet, when another wallet created the invoice |
| `invoice_amount` | `amount` | Amount of the invoice |
| `htlc_amount` | `amount \| null` | Amount of the received HTLC |

```json
{
  "type": "SPARK_LIGHTNING_RECEIVE_FINISHED",
  "id": "0194c7a2-5e1b-7c3d-9f00-3a1b2c4d5e6f",
  "created_at": "2026-01-15T14:32:00Z",
  "updated_at": "2026-01-15T14:32:00Z",
  "network": "MAINNET",
  "request_status": "SUCCEEDED",
  "status": "TRANSFER_COMPLETED",
  "payment_preimage": "0000000000000000000000000000000000000000000000000000000000000000",
  "receiver_identity_public_key": "000000000000000000000000000000000000000000000000000000000000000000",
  "invoice_amount": {
    "value": 100000,
    "unit": "SATOSHI"
  },
  "htlc_amount": {
    "value": 100000,
    "unit": "SATOSHI"
  },
  "timestamp": "2026-01-15T14:32:00Z"
}
```

### Lightning send finished

| Field | Type | Description |
|-------|------|-------------|
| `status` | `string` | A `LightningSendRequestStatus` value: `CREATED`, `USER_TRANSFER_VALIDATION_FAILED`, `LIGHTNING_PAYMENT_INITIATED`, `LIGHTNING_PAYMENT_FAILED`, `LIGHTNING_PAYMENT_SUCCEEDED`, `PREIMAGE_PROVIDED`, `PREIMAGE_PROVIDING_FAILED`, `TRANSFER_COMPLETED`, `TRANSFER_FAILED`, `PENDING_USER_SWAP_RETURN`, `USER_SWAP_RETURNED`, `USER_SWAP_RETURN_FAILED`, `REQUEST_VALIDATED` |
| `encoded_invoice` | `string` | The BOLT11 invoice |
| `fee` | `amount` | Fee for paying the invoice |
| `idempotency_key` | `string \| null` | Idempotency key of the send request |
| `invoice_amount` | `amount` | Amount of the invoice |
| `payment_preimage` | `string \| null` | Hex-encoded payment preimage |

```json
{
  "type": "SPARK_LIGHTNING_SEND_FINISHED",
  "id": "0194c7a2-5e1b-7c3d-9f00-3a1b2c4d5e6f",
  "created_at": "2026-01-15T14:32:00Z",
  "updated_at": "2026-01-15T14:32:00Z",
  "network": "MAINNET",
  "request_status": "SUCCEEDED",
  "status": "PREIMAGE_PROVIDED",
  "encoded_invoice": "lnbc500u1test...",
  "fee": {
    "value": 100,
    "unit": "SATOSHI"
  },
  "idempotency_key": "user-defined-key-123",
  "invoice_amount": {
    "value": 50000,
    "unit": "SATOSHI"
  },
  "payment_preimage": "0000000000000000000000000000000000000000000000000000000000000000",
  "timestamp": "2026-01-15T14:32:00Z"
}
```

### Cooperative exit finished

| Field | Type | Description |
|-------|------|-------------|
| `status` | `string` | A `SparkCoopExitRequestStatus` value: `INITIATED`, `COMPLETE_REQUEST_RECEIVED`, `INBOUND_TRANSFER_CHECKED`, `TX_BROADCASTING_SCHEDULED`, `TX_BROADCASTING_FAILED`, `TX_BROADCASTED`, `ON_CHAIN_TX_CONFIRMED`, `INBOUND_TRANSFER_CLAIMING_FAILED`, `SUCCEEDED`, `EXPIRING_SCHEDULED`, `EXPIRING_FAILED`, `EXPIRED`, `FAILING_SCHEDULED`, `FAILING_FAILED`, `FAILED`, `TX_SIGNED`, `WAITING_ON_TX_CONFIRMATIONS`, `INBOUND_TRANSFER_CLAIMING_SCHEDULED` |
| `fee` | `amount` | Fee for the cooperative exit, excluding `l1_broadcast_fee` |
| `withdrawal_address` | `string \| null` | Bitcoin address to withdraw to |
| `l1_broadcast_fee` | `amount` | On-chain fee of the cooperative exit |
| `exit_speed` | `string \| null` | Requested exit speed: `FAST`, `MEDIUM` or `SLOW` |
| `coop_exit_txid` | `string` | Id of the cooperative exit transaction |
| `expires_at` | `string` | ISO 8601 timestamp of when the request expires |
| `total_amount` | `amount` | Total amount of the cooperative exit |

```json
{
  "type": "SPARK_COOP_EXIT_FINISHED",
  "id": "0194c7a2-5e1b-7c3d-9f00-3a1b2c4d5e6f",
  "created_at": "2026-01-15T14:32:00Z",
  "updated_at": "2026-01-15T14:32:00Z",
  "network": "MAINNET",
  "request_status": "SUCCEEDED",
  "status": "SUCCEEDED",
  "fee": {
    "value": 500,
    "unit": "SATOSHI"
  },
  "withdrawal_address": "bc1qtest...",
  "l1_broadcast_fee": {
    "value": 250,
    "unit": "SATOSHI"
  },
  "exit_speed": "MEDIUM",
  "coop_exit_txid": "0000000000000000000000000000000000000000000000000000000000000000",
  "expires_at": "2026-01-16T14:32:00Z",
  "total_amount": {
    "value": 200000,
    "unit": "SATOSHI"
  },
  "timestamp": "2026-01-15T14:32:00Z"
}
```

### Static deposit finished

| Field | Type | Description |
|-------|------|-------------|
| `status` | `string` | A `ClaimStaticDepositStatus` value: `CREATED`, `TRANSFER_CREATED`, `TRANSFER_CREATION_FAILED`, `TRANSFER_COMPLETED`, `UTXO_SWAPPING_FAILED`, `SPEND_TX_CREATED`, `SPEND_TX_BROADCAST`, `SPEND_TX_CONFIRMED` |
| `deposit_amount` | `amount` | Amount of the deposit |
| `credit_amount` | `amount` | Amount to be credited to the wallet |
| `max_fee` | `amount` | Maximum fee the wallet agreed to pay |
| `transaction_id` | `string` | Id of the deposit transaction |
| `output_index` | `integer` | Index of the deposit output in that transaction |
| `bitcoin_network` | `string` | Network of the deposit: `MAINNET`, `REGTEST`, `SIGNET` or `TESTNET` |
| `static_deposit_address` | `string \| null` | Static deposit address that received the deposit |

```json
{
  "type": "SPARK_STATIC_DEPOSIT_FINISHED",
  "id": "0194c7a2-5e1b-7c3d-9f00-3a1b2c4d5e6f",
  "created_at": "2026-01-15T14:32:00Z",
  "updated_at": "2026-01-15T14:32:00Z",
  "network": "MAINNET",
  "request_status": "SUCCEEDED",
  "status": "TRANSFER_COMPLETED",
  "deposit_amount": {
    "value": 300000,
    "unit": "SATOSHI"
  },
  "credit_amount": {
    "value": 299500,
    "unit": "SATOSHI"
  },
  "max_fee": {
    "value": 500,
    "unit": "SATOSHI"
  },
  "transaction_id": "0000000000000000000000000000000000000000000000000000000000000000",
  "output_index": 0,
  "bitcoin_network": "MAINNET",
  "static_deposit_address": "bc1qtest...",
  "timestamp": "2026-01-15T14:32:00Z"
}
```

## Registering a webhook

To register a webhook, provide a URL, a secret for payload verification, and the event types you want to subscribe to.

```dart
RegisterWebhookRequest request = RegisterWebhookRequest(
  url: "https://example.com/webhook",
  secret: "your-webhook-secret",
  eventTypes: [
    WebhookEventType.lightningReceiveFinished(),
    WebhookEventType.lightningSendFinished(),
  ],
);
RegisterWebhookResponse response = await sdk.registerWebhook(request: request);
print("Webhook registered with ID: ${response.webhookId}");
```



## Unregistering a webhook

To stop receiving notifications for a webhook, unregister it using its ID.

```dart
String webhookId = "webhook-id";
await sdk.unregisterWebhook(
  request: UnregisterWebhookRequest(webhookId: webhookId),
);
print("Webhook unregistered");
```



## Listing webhooks

To retrieve all currently registered webhooks, use the list method.

```dart
List<Webhook> webhooks = await sdk.listWebhooks();
for (Webhook webhook in webhooks) {
  print("Webhook: id=${webhook.id}, url=${webhook.url}, events=${webhook.eventTypes}");
}
```
