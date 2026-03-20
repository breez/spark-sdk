# Managing webhooks

Webhooks allow you to receive real-time notifications when events occur in your wallet, such as completed Lightning payments or on-chain deposits. The Spark service provider sends an HTTP POST request to your specified URL whenever a subscribed event occurs. Each webhook payload is signed using HMAC-SHA256 with the secret you provide during registration, allowing you to verify the authenticity of incoming notifications.

## Event types

The following event types are available for webhook subscriptions:

| Event type | Description |
|-----------|-------------|
| `LightningReceiveFinished` | A Lightning receive operation completed |
| `LightningSendFinished` | A Lightning send operation completed |
| `CoopExitFinished` | A cooperative exit completed |
| `StaticDepositFinished` | A static deposit completed |

## Webhook payload

When an event occurs, the Spark service provider sends an HTTP POST request to your webhook URL. The payload is a JSON object whose fields vary by event type. The request includes an `X-Spark-Signature` header containing an HMAC-SHA256 signature of the raw request body, computed using the secret you provided during registration.

All payloads share the following common fields:

| Field | Type | Description |
|-------|------|-------------|
| `id` | `string` | Unique identifier for the request |
| `created_at` | `string` | ISO 8601 timestamp of when the request was created |
| `updated_at` | `string` | ISO 8601 timestamp of the last update |
| `network` | `string` | The network (`MAINNET`, `TESTNET`, `REGTEST`) |
| `request_status` | `string` | Status of the request (e.g., `COMPLETED`) |
| `status` | `string` | Event-specific status |
| `type` | `string` | The event type (e.g., `SPARK_LIGHTNING_RECEIVE_FINISHED`) |
| `timestamp` | `string` | ISO 8601 timestamp of the event |

### Lightning receive finished

```json
{
  "id": "018677b5-e419-99d1-0000-a7030393c9af",
  "created_at": "2025-03-09T12:00:00Z",
  "updated_at": "2025-03-09T12:00:05Z",
  "network": "MAINNET",
  "request_status": "COMPLETED",
  "status": "TRANSFER_COMPLETED",
  "type": "SPARK_LIGHTNING_RECEIVE_FINISHED",
  "timestamp": "2025-03-09T12:00:06Z",
  "payment_preimage": "a1b2c3d4e5f6...",
  "receiver_identity_public_key": "02abc123...",
  "invoice_amount": {"value": 50000, "unit": "SATOSHI"},
  "htlc_amount": {"value": 50000, "unit": "SATOSHI"}
}
```

### Lightning send finished

```json
{
  "id": "018677b5-e419-99d1-0000-a7030393c9af",
  "created_at": "2025-03-09T12:00:00Z",
  "updated_at": "2025-03-09T12:00:05Z",
  "network": "MAINNET",
  "request_status": "COMPLETED",
  "status": "PREIMAGE_PROVIDED",
  "type": "SPARK_LIGHTNING_SEND_FINISHED",
  "timestamp": "2025-03-09T12:00:06Z",
  "encoded_invoice": "lnbc50u1p...",
  "fee": {"value": 100, "unit": "SATOSHI"},
  "idempotency_key": "user-defined-key-123",
  "invoice_amount": {"value": 50000, "unit": "SATOSHI"}
}
```

### Cooperative exit finished

```json
{
  "id": "018677b5-e419-99d1-0000-a7030393c9af",
  "created_at": "2025-03-09T12:00:00Z",
  "updated_at": "2025-03-09T12:00:05Z",
  "network": "MAINNET",
  "request_status": "COMPLETED",
  "status": "SUCCEEDED",
  "type": "SPARK_COOP_EXIT_FINISHED",
  "timestamp": "2025-03-09T12:00:06Z",
  "fee": {"value": 500, "unit": "SATOSHI"},
  "withdrawal_address": "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh",
  "l1_broadcast_fee": {"value": 200, "unit": "SATOSHI"},
  "exit_speed": "NORMAL",
  "coop_exit_txid": "a1b2c3d4...",
  "expires_at": "2025-03-10T12:00:00Z",
  "total_amount": {"value": 49300, "unit": "SATOSHI"}
}
```

### Static deposit finished

```json
{
  "id": "018677b5-e419-99d1-0000-a7030393c9af",
  "created_at": "2025-03-09T12:00:00Z",
  "updated_at": "2025-03-09T12:00:05Z",
  "network": "MAINNET",
  "request_status": "COMPLETED",
  "status": "TRANSFER_COMPLETED",
  "type": "SPARK_STATIC_DEPOSIT_FINISHED",
  "timestamp": "2025-03-09T12:00:06Z",
  "deposit_amount": {"value": 100000, "unit": "SATOSHI"},
  "credit_amount": {"value": 99500, "unit": "SATOSHI"},
  "max_fee": {"value": 1000, "unit": "SATOSHI"},
  "transaction_id": "d4e5f6a7b8c9...",
  "output_index": 0,
  "bitcoin_network": "MAINNET",
  "static_deposit_address": "bc1q..."
}
```

<h2 id="registering-a-webhook">
    <a class="header" href="#registering-a-webhook">Registering a webhook</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.register_webhook">API docs</a>
</h2>

To register a webhook, provide a URL, a secret for payload verification, and the event types you want to subscribe to.

{{#tabs webhooks:register-webhook}}

<h2 id="unregistering-a-webhook">
    <a class="header" href="#unregistering-a-webhook">Unregistering a webhook</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.unregister_webhook">API docs</a>
</h2>

To stop receiving notifications for a webhook, unregister it using its ID.

{{#tabs webhooks:unregister-webhook}}

<h2 id="listing-webhooks">
    <a class="header" href="#listing-webhooks">Listing webhooks</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_webhooks">API docs</a>
</h2>

To retrieve all currently registered webhooks, use the list method.

{{#tabs webhooks:list-webhooks}}
