# Managing webhooks

Webhooks allow you to receive real-time notifications when events occur in your wallet, such as completed Lightning payments or on-chain deposits. The Spark service provider sends an HTTP POST request to your specified URL whenever a subscribed event occurs. Each webhook payload is signed using HMAC-SHA256 with the secret you provide during registration, allowing you to verify the authenticity of incoming notifications.

## Event types

The following event types are available for webhook subscriptions:

| Event type | Description |
|-----------|-------------|
| `WebhookEventType::LightningReceiveFinished` | A Lightning receive operation completed |
| `WebhookEventType::LightningSendFinished` | A Lightning send operation completed |
| `WebhookEventType::CoopExitFinished` | A cooperative exit completed |
| `WebhookEventType::StaticDepositFinished` | A static deposit completed |

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

## Registering a webhook

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.register_webhook

To register a webhook, provide a URL, a secret for payload verification, and the event types you want to subscribe to.

### Rust

```rust
let response = sdk
    .register_webhook(RegisterWebhookRequest {
        url: "https://example.com/webhook".to_string(),
        secret: "your-webhook-secret".to_string(),
        event_types: vec![
            WebhookEventType::LightningReceiveFinished,
            WebhookEventType::LightningSendFinished,
        ],
    })
    .await?;
info!("Webhook registered with ID: {}", response.webhook_id);
```

### Swift

```swift
let response = try await sdk.registerWebhook(
    request: RegisterWebhookRequest(
        url: "https://example.com/webhook",
        secret: "your-webhook-secret",
        eventTypes: [.lightningReceiveFinished, .lightningSendFinished]
    ))
print("Webhook registered with ID: \(response.webhookId)")
```

### Kotlin

```kotlin
val response = sdk.registerWebhook(RegisterWebhookRequest(
    url = "https://example.com/webhook",
    secret = "your-webhook-secret",
    eventTypes = listOf(
        WebhookEventType.LightningReceiveFinished,
        WebhookEventType.LightningSendFinished
    )
))
// Log.v("Breez", "Webhook registered with ID: ${response.webhookId}")
```

### C#

```csharp
var response = await sdk.RegisterWebhook(request: new RegisterWebhookRequest(
    url: "https://example.com/webhook",
    secret: "your-webhook-secret",
    eventTypes: new WebhookEventType[]
    {
        new WebhookEventType.LightningReceiveFinished(),
        new WebhookEventType.LightningSendFinished()
    }
));
Console.WriteLine($"Webhook registered with ID: {response.webhookId}");
```

### Javascript (Wasm)

```typescript
const response = await sdk.registerWebhook({
  url: 'https://example.com/webhook',
  secret: 'your-webhook-secret',
  eventTypes: [{ type: 'lightningReceiveFinished' }, { type: 'lightningSendFinished' }]
})
console.log(`Webhook registered with ID: ${response.webhookId}`)
```

### React Native

```typescript
const response = await sdk.registerWebhook({
  url: 'https://example.com/webhook',
  secret: 'your-webhook-secret',
  eventTypes: [
    new WebhookEventType.LightningReceiveFinished(),
    new WebhookEventType.LightningSendFinished()
  ]
})
console.log(`Webhook registered with ID: ${response.webhookId}`)
```

### Flutter

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

### Python

```python
event_types = [
    WebhookEventType.LIGHTNING_RECEIVE_FINISHED(),
    WebhookEventType.LIGHTNING_SEND_FINISHED(),
]
response = await sdk.register_webhook(
    request=RegisterWebhookRequest(
        url="https://example.com/webhook",
        secret="your-webhook-secret",
        event_types=event_types,
    )
)
logging.debug(f"Webhook registered with ID: {response.webhook_id}")
```

### Go

```go
response, err := sdk.RegisterWebhook(breez_sdk_spark.RegisterWebhookRequest{
	Url:    "https://example.com/webhook",
	Secret: "your-webhook-secret",
	EventTypes: []breez_sdk_spark.WebhookEventType{
		breez_sdk_spark.WebhookEventTypeLightningReceiveFinished{},
		breez_sdk_spark.WebhookEventTypeLightningSendFinished{},
	},
})
if err != nil {
	return nil, err
}

log.Printf("Webhook registered with ID: %v", response.WebhookId)
```



## Unregistering a webhook

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.unregister_webhook

To stop receiving notifications for a webhook, unregister it using its ID.

### Rust

```rust
let webhook_id = "webhook-id".to_string();
sdk.unregister_webhook(UnregisterWebhookRequest { webhook_id })
    .await?;
info!("Webhook unregistered");
```

### Swift

```swift
let webhookId = "webhook-id"
try await sdk.unregisterWebhook(
    request: UnregisterWebhookRequest(webhookId: webhookId))
print("Webhook unregistered")
```

### Kotlin

```kotlin
val webhookId = "webhook-id"
sdk.unregisterWebhook(UnregisterWebhookRequest(webhookId = webhookId))
// Log.v("Breez", "Webhook unregistered")
```

### C#

```csharp
var webhookId = "webhook-id";
await sdk.UnregisterWebhook(request: new UnregisterWebhookRequest(
    webhookId: webhookId
));
Console.WriteLine("Webhook unregistered");
```

### Javascript (Wasm)

```typescript
const webhookId = 'webhook-id'
await sdk.unregisterWebhook({ webhookId })
console.log('Webhook unregistered')
```

### React Native

```typescript
const webhookId = 'webhook-id'
await sdk.unregisterWebhook({ webhookId })
console.log('Webhook unregistered')
```

### Flutter

```dart
String webhookId = "webhook-id";
await sdk.unregisterWebhook(
  request: UnregisterWebhookRequest(webhookId: webhookId),
);
print("Webhook unregistered");
```

### Python

```python
webhook_id = "webhook-id"
await sdk.unregister_webhook(
    request=UnregisterWebhookRequest(webhook_id=webhook_id)
)
logging.debug("Webhook unregistered")
```

### Go

```go
webhookId := "webhook-id"
err := sdk.UnregisterWebhook(breez_sdk_spark.UnregisterWebhookRequest{
	WebhookId: webhookId,
})
if err != nil {
	return err
}

log.Printf("Webhook unregistered")
```



## Listing webhooks

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_webhooks

To retrieve all currently registered webhooks, use the list method.

### Rust

```rust
let webhooks = sdk.list_webhooks().await?;
for webhook in webhooks {
    info!(
        "Webhook: id={}, url={}, events={:?}",
        webhook.id, webhook.url, webhook.event_types
    );
}
```

### Swift

```swift
let webhooks = try await sdk.listWebhooks()
for webhook in webhooks {
    print("Webhook: id=\(webhook.id), url=\(webhook.url), events=\(webhook.eventTypes)")
}
```

### Kotlin

```kotlin
val webhooks = sdk.listWebhooks()
for (webhook in webhooks) {
    // Log.v("Breez", "Webhook: id=${webhook.id}, url=${webhook.url}, events=${webhook.eventTypes}")
}
```

### C#

```csharp
var webhooks = await sdk.ListWebhooks();
foreach (var webhook in webhooks)
{
    Console.WriteLine($"Webhook: id={webhook.id}, url={webhook.url}, events={webhook.eventTypes}");
}
```

### Javascript (Wasm)

```typescript
const webhooks = await sdk.listWebhooks()
for (const webhook of webhooks) {
  console.log(`Webhook: id=${webhook.id}, url=${webhook.url}, events=${String(webhook.eventTypes)}`)
}
```

### React Native

```typescript
const webhooks = await sdk.listWebhooks()
for (const webhook of webhooks) {
  console.log(`Webhook: id=${webhook.id}, url=${webhook.url}, events=${String(webhook.eventTypes)}`)
}
```

### Flutter

```dart
List<Webhook> webhooks = await sdk.listWebhooks();
for (Webhook webhook in webhooks) {
  print("Webhook: id=${webhook.id}, url=${webhook.url}, events=${webhook.eventTypes}");
}
```

### Python

```python
webhooks = await sdk.list_webhooks()
for webhook in webhooks:
    logging.debug(
        f"Webhook: id={webhook.id}, url={webhook.url}, "
        f"events={webhook.event_types}"
    )
```

### Go

```go
webhooks, err := sdk.ListWebhooks()
if err != nil {
	return nil, err
}

for _, webhook := range webhooks {
	log.Printf("Webhook: id=%v, url=%v, events=%v", webhook.Id, webhook.Url, webhook.EventTypes)
}
```



---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
