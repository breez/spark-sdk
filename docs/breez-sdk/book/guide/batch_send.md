# Sending to multiple recipients

A single transaction can pay multiple recipients at once. The batch API is separate from the regular send flow because a batch has no single amount to report: it may span several tokens, so prepare reports a total per asset instead.

A batch currently pays tokens only. Sending sats to several recipients at once is not supported yet, so every recipient must resolve to a token.

## Preparing the batch

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_batch

Each recipient is identified by a `payment_request`, which is either a Spark address or a Spark invoice, and the two may be mixed freely in one batch:

- **Spark address**: the token identifier and the amount must be set, exactly as for a single-recipient send. Leaving the token identifier unset would name sats, which a batch cannot send yet.
- **Spark invoice**: the token and amount are taken from the invoice. The amount is only required if the invoice doesn't specify one. If the invoice specifies an amount, providing a different amount is not supported.

The same invoice may only appear once in a batch. Repeating a plain Spark address is allowed: that is simply two outputs to the same payee.

A batch that pays a Spark invoice is limited to a single token, and that includes its address recipients: the Spark operators reject a transaction that carries an invoice and pays more than one token. Prepare rejects such a batch, so split it into one batch per token. A batch of address recipients only may span as many tokens as you like.

The response resolves every recipient into the concrete `destination`, asset and amount it will be paid, and reports `totals`: what the batch debits, one entry per distinct asset. You may show these to the user before sending.

### Rust

```rust
// Each recipient is a Spark address or a Spark invoice. An invoice that
// names its own token and amount needs neither here.
let recipients = vec![
    BatchRecipient {
        payment_request: "<spark address>".to_string(),
        amount: Some(1_000),
        token_identifier: Some("<token identifier>".to_string()),
    },
    BatchRecipient {
        payment_request: "<spark invoice>".to_string(),
        amount: None,
        token_identifier: None,
    },
];

let prepare_response = sdk
    .prepare_send_batch(PrepareSendBatchRequest { recipients })
    .await?;

// Show what the batch debits, one entry per token
for total in &prepare_response.totals {
    // Unset would mean sats, which a batch cannot send yet
    let token_id = total.token_identifier.as_deref().unwrap_or_default();
    info!("Token ID: {token_id}");
    info!("Total: {} token base units", total.amount);
}

// If the totals are acceptable, send the batch
let send_response = sdk
    .send_batch(SendBatchRequest { prepare_response })
    .await?;

// One payment per recipient, in the order they were requested
for payment in send_response.payments {
    info!("Payment: {payment:?}");
}
```

### Swift

```swift
// Each recipient is a Spark address or a Spark invoice. An invoice that
// names its own token and amount needs neither here.
let recipients = [
    BatchRecipient(
        paymentRequest: "<spark address>",
        amount: BInt(1_000),
        tokenIdentifier: "<token identifier>"
    ),
    BatchRecipient(
        paymentRequest: "<spark invoice>",
        amount: nil,
        tokenIdentifier: nil
    ),
]

let prepareResponse = try await sdk.prepareSendBatch(
    request: PrepareSendBatchRequest(recipients: recipients))

// Show what the batch debits, one entry per token
for total in prepareResponse.totals {
    // Unset would mean sats, which a batch cannot send yet
    print("Token ID: \(total.tokenIdentifier ?? "")")
    print("Total: \(total.amount) token base units")
}

// If the totals are acceptable, send the batch
let sendResponse = try await sdk.sendBatch(
    request: SendBatchRequest(prepareResponse: prepareResponse))

// One payment per recipient, in the order they were requested
for payment in sendResponse.payments {
    print("Payment: \(payment)")
}
```

### Kotlin

```kotlin
try {
    // Each recipient is a Spark address or a Spark invoice. An invoice that
    // names its own token and amount needs neither here.
    val recipients = listOf(
        BatchRecipient(
            paymentRequest = "<spark address>",
            // Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer)
            amount = BigInteger.fromLong(1_000L),
            // Android (BigInteger from java.math)
            // amount = BigInteger.valueOf(1_000L),
            tokenIdentifier = "<token identifier>"
        ),
        BatchRecipient(
            paymentRequest = "<spark invoice>",
            amount = null,
            tokenIdentifier = null
        )
    )

    val prepareResponse =
        sdk.prepareSendBatch(PrepareSendBatchRequest(recipients = recipients))

    // Show what the batch debits, one entry per token
    for (total in prepareResponse.totals) {
        // Unset would mean sats, which a batch cannot send yet
        println("Token ID: ${total.tokenIdentifier}")
        println("Total: ${total.amount} token base units")
    }

    // If the totals are acceptable, send the batch
    val sendResponse =
        sdk.sendBatch(SendBatchRequest(prepareResponse = prepareResponse))

    // One payment per recipient, in the order they were requested
    for (payment in sendResponse.payments) {
        println("Payment: $payment")
    }
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
// Each recipient is a Spark address or a Spark invoice. An invoice that
// names its own token and amount needs neither here.
var recipients = new BatchRecipient[] {
    new BatchRecipient(
        paymentRequest: "<spark address>",
        amount: new BigInteger(1_000),
        tokenIdentifier: "<token identifier>"
    ),
    new BatchRecipient(
        paymentRequest: "<spark invoice>",
        amount: null,
        tokenIdentifier: null
    )
};

var prepareResponse = await sdk.PrepareSendBatch(
    request: new PrepareSendBatchRequest(recipients: recipients)
);

// Show what the batch debits, one entry per token
foreach (var total in prepareResponse.totals)
{
    // Unset would mean sats, which a batch cannot send yet
    Console.WriteLine($"Token ID: {total.tokenIdentifier}");
    Console.WriteLine($"Total: {total.amount} token base units");
}

// If the totals are acceptable, send the batch
var sendResponse = await sdk.SendBatch(
    request: new SendBatchRequest(prepareResponse: prepareResponse)
);

// One payment per recipient, in the order they were requested
foreach (var payment in sendResponse.payments)
{
    Console.WriteLine($"Payment: {payment}");
}
```

### Javascript (Wasm)

```typescript
// Each recipient is a Spark address or a Spark invoice. An invoice that
// names its own token and amount needs neither here.
const recipients: BatchRecipient[] = [
  {
    paymentRequest: '<spark address>',
    amount: BigInt(1_000),
    tokenIdentifier: '<token identifier>'
  },
  {
    paymentRequest: '<spark invoice>',
    amount: undefined,
    tokenIdentifier: undefined
  }
]

const prepareResponse = await sdk.prepareSendBatch({ recipients })

// Show what the batch debits, one entry per token
for (const total of prepareResponse.totals) {
  // Unset would mean sats, which a batch cannot send yet
  console.log(`Token ID: ${total.tokenIdentifier}`)
  console.log(`Total: ${total.amount} token base units`)
}

// If the totals are acceptable, send the batch
const sendResponse = await sdk.sendBatch({ prepareResponse })

// One payment per recipient, in the order they were requested
for (const payment of sendResponse.payments) {
  console.log(`Payment: ${JSON.stringify(payment)}`)
}
```

### React Native

```typescript
// Each recipient is a Spark address or a Spark invoice. An invoice that
// names its own token and amount needs neither here.
const recipients: BatchRecipient[] = [
  {
    paymentRequest: '<spark address>',
    amount: BigInt(1_000),
    tokenIdentifier: '<token identifier>'
  },
  {
    paymentRequest: '<spark invoice>',
    amount: undefined,
    tokenIdentifier: undefined
  }
]

const prepareResponse = await sdk.prepareSendBatch({ recipients })

// Show what the batch debits, one entry per token
for (const total of prepareResponse.totals) {
  // Unset would mean sats, which a batch cannot send yet
  console.log(`Token ID: ${total.tokenIdentifier}`)
  console.log(`Total: ${total.amount} token base units`)
}

// If the totals are acceptable, send the batch
const sendResponse = await sdk.sendBatch({ prepareResponse })

// One payment per recipient, in the order they were requested
for (const payment of sendResponse.payments) {
  console.log(`Payment: ${JSON.stringify(payment)}`)
}
```

### Flutter

```dart
// Each recipient is a Spark address or a Spark invoice. An invoice that
// names its own token and amount needs neither here.
final recipients = [
  BatchRecipient(
    paymentRequest: '<spark address>',
    amount: BigInt.from(1000),
    tokenIdentifier: '<token identifier>',
  ),
  BatchRecipient(
    paymentRequest: '<spark invoice>',
    amount: null,
    tokenIdentifier: null,
  ),
];

final prepareResponse = await sdk.prepareSendBatch(
  request: PrepareSendBatchRequest(recipients: recipients),
);

// Show what the batch debits, one entry per token
for (final total in prepareResponse.totals) {
  // Unset would mean sats, which a batch cannot send yet
  print('Token ID: ${total.tokenIdentifier}');
  print('Total: ${total.amount} token base units');
}

// If the totals are acceptable, send the batch
final sendResponse = await sdk.sendBatch(
  request: SendBatchRequest(prepareResponse: prepareResponse),
);

// One payment per recipient, in the order they were requested
for (final payment in sendResponse.payments) {
  print('Payment: $payment');
}
```

### Python

```python
try:
    # Each recipient is a Spark address or a Spark invoice. An invoice that
    # names its own token and amount needs neither here.
    recipients = [
        BatchRecipient(
            payment_request="<spark address>",
            amount=1_000,
            token_identifier="<token identifier>",
        ),
        BatchRecipient(
            payment_request="<spark invoice>",
            amount=None,
            token_identifier=None,
        ),
    ]

    prepare_response = await sdk.prepare_send_batch(
        request=PrepareSendBatchRequest(recipients=recipients)
    )

    # Show what the batch debits, one entry per token
    for total in prepare_response.totals:
        # Unset would mean sats, which a batch cannot send yet
        print(f"Token ID: {total.token_identifier}")
        print(f"Total: {total.amount} token base units")

    # If the totals are acceptable, send the batch
    send_response = await sdk.send_batch(
        request=SendBatchRequest(prepare_response=prepare_response)
    )

    # One payment per recipient, in the order they were requested
    for payment in send_response.payments:
        print(f"Payment: {payment}")
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
// Each recipient is a Spark address or a Spark invoice. An invoice that
// names its own token and amount needs neither here.
amount := new(big.Int).SetInt64(1_000)
tokenIdentifier := "<token identifier>"
recipients := []breez_sdk_spark.BatchRecipient{
	{
		PaymentRequest:  "<spark address>",
		Amount:          &amount,
		TokenIdentifier: &tokenIdentifier,
	},
	{
		PaymentRequest:  "<spark invoice>",
		Amount:          nil,
		TokenIdentifier: nil,
	},
}

prepareResponse, err := sdk.PrepareSendBatch(breez_sdk_spark.PrepareSendBatchRequest{
	Recipients: recipients,
})

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}

// Show what the batch debits, one entry per token
for _, total := range prepareResponse.Totals {
	// Unset would mean sats, which a batch cannot send yet
	tokenID := ""
	if total.TokenIdentifier != nil {
		tokenID = *total.TokenIdentifier
	}
	log.Printf("Token ID: %s", tokenID)
	log.Printf("Total: %v token base units", total.Amount)
}

// If the totals are acceptable, send the batch
sendResponse, err := sdk.SendBatch(breez_sdk_spark.SendBatchRequest{
	PrepareResponse: prepareResponse,
})

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}

// One payment per recipient, in the order they were requested
for _, payment := range sendResponse.Payments {
	log.Printf("Payment: %#v", payment)
}
```



## Sending the batch

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.send_batch

Sending returns one payment per recipient, in the order the recipients were requested. Every payment carries the same transaction hash, so your application can group them later by filtering the payment list on that hash. See [Listing payments](./list_payments.md).

Each payment also raises its own `SdkEvent::PaymentSucceeded` event, the same as a single-recipient send.

**Developer note**

A token send carries no idempotency key. If a batch fails in a way that leaves the outcome unknown, look for the transaction first (filtering the payment list by transaction hash) instead of sending it again, which would pay every recipient twice.

## External signing

A batch can also be signed outside the SDK. The flow matches the single-recipient one described in [Client signing](./client_signing.md), with two differences: the unsigned package is built from the batch prepare response, and publishing it returns every payment rather than one. The package carries the same per-asset totals, so the signer can show the user what they are approving.

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
