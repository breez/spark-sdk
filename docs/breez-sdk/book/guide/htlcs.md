# Conditional Payments

Conditional payments use Hash Time-Locked Contracts (HTLCs) to lock funds with a cryptographic hash of a secret preimage and an expiration time. The payment can only be claimed by revealing the preimage before expiration. If not claimed in time, the funds are automatically returned to the sender. This enables use cases like atomic cross-chain swaps.

The SDK supports both sending conditional payments via Spark HTLCs and receiving them via HODL invoices.

**Developer note**

Preimages are required to be unique and are not managed by the SDK. It is your responsibility as a developer to manage them, including how to generate them, store them, and provide them when claiming payments.

## Sending Spark HTLC payments

HTLC payments use the standard payment API described in [Sending payments](send_payment.md). To create an HTLC payment, prepare the payment normally, then provide the Spark HTLC options when [sending](send_payment.md#spark). These options include the payment hash (SHA-256 hash of the preimage) and the expiry duration.

### Rust

```rust
let payment_request = "<spark address>".to_string();
// Set the amount you wish to pay the receiver
let amount_sats = Some(50_000);
let prepare_request = PrepareSendPaymentRequest {
    payment_request: PaymentRequest::Input {
        input: payment_request,
    },
    amount: amount_sats,
    token_identifier: None,
    conversion_options: None,
    fee_policy: None,
};
let prepare_response = sdk.prepare_send_payment(prepare_request).await?;

// If the fees are acceptable, continue to create the HTLC Payment
if let SendPaymentMethod::SparkAddress { fee, .. } = prepare_response.payment_method {
    info!("Fees: {} sats", fee);
}

let preimage = "<32-byte unique preimage hex>";
let preimage_bytes = hex::decode(preimage)?;
let payment_hash_bytes = sha256::digest(preimage_bytes);
let payment_hash = hex::encode(payment_hash_bytes);

// Set the HTLC options
let options = SendPaymentOptions::SparkAddress {
    htlc_options: Some(SparkHtlcOptions {
        payment_hash,
        expiry_duration_secs: 1000,
    }),
};

let request = SendPaymentRequest {
    prepare_response,
    options: Some(options),
    idempotency_key: None,
};
let send_response = sdk.send_payment(request).await?;
let payment = send_response.payment;
```

### Swift

```swift
let paymentRequest = "<spark address>"
// Set the amount you wish to pay the receiver
let amountSats = BInt(50_000)
let prepareRequest = PrepareSendPaymentRequest(
    paymentRequest: .input(input: paymentRequest),
    amount: amountSats,
    tokenIdentifier: nil,
    conversionOptions: nil,
    feePolicy: nil
)
let prepareResponse = try await sdk.prepareSendPayment(request: prepareRequest)

// If the fees are acceptable, continue to create the HTLC Payment
if case let .sparkAddress(_, fee, _) = prepareResponse.paymentMethod {
    print("Fees: \(fee) sats")
}

let preimage = "<32-byte unique preimage hex>"
let preimageData = Data(hexString: preimage)!
let paymentHashDigest = SHA256.hash(data: preimageData)
let paymentHash = Data(paymentHashDigest).hexEncodedString()

// Set the HTLC options
let htlcOptions = SparkHtlcOptions(
    paymentHash: paymentHash,
    expiryDurationSecs: 1000
)
let options = SendPaymentOptions.sparkAddress(htlcOptions: htlcOptions)

let request = SendPaymentRequest(
    prepareResponse: prepareResponse,
    options: options
)
let sendResponse = try await sdk.sendPayment(request: request)
let payment = sendResponse.payment
```

### Kotlin

```kotlin
val paymentRequest = "<spark address>"
// Set the amount you wish the pay the receiver
// Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer)
val amountSats = BigInteger.fromLong(50_000L)
// Android (BigInteger from java.math)
// val amountSats = BigInteger.valueOf(50_000L)
try {
    val prepareRequest = PrepareSendPaymentRequest(
        paymentRequest = PaymentRequest.Input(input = paymentRequest),
        amount = amountSats,
        tokenIdentifier = null,
        conversionOptions = null,
        feePolicy = null,
    )
    val prepareResponse = sdk.prepareSendPayment(prepareRequest)

    // If the fees are acceptable, continue to create the HTLC Payment
    val paymentMethod = prepareResponse.paymentMethod
    if (paymentMethod is SendPaymentMethod.SparkAddress) {
        val fee = paymentMethod.fee
        // Log.v("Breez", "Fees: ${fee} sats")
    }

    val preimage = "<32-byte unique preimage hex>"
    val preimageBytes = preimage.hexToByteArray()
    val digest = SHA256()
    digest.update(preimageBytes)
    val paymentHashBytes = digest.digest()
    val paymentHash = paymentHashBytes.toHexString()

    // Set the HTLC options
    val htlcOptions = SparkHtlcOptions(
        paymentHash = paymentHash,
        expiryDurationSecs = 1000u
    )
    val options = SendPaymentOptions.SparkAddress(htlcOptions = htlcOptions)

    val request = SendPaymentRequest(
        prepareResponse = prepareResponse,
        options = options
    )
    val sendResponse = sdk.sendPayment(request)
    val payment = sendResponse.payment
} catch (e: Exception) {
    // handle error
    throw e
}
```

### C#

```csharp
var paymentRequest = "<spark address>";
// Set the amount you wish the pay the receiver
ulong? amountSats = 50_000UL;
var prepareRequest = new PrepareSendPaymentRequest(
    paymentRequest: new PaymentRequest.Input(input: paymentRequest),
    amount: amountSats,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: null
);
var prepareResponse = await sdk.PrepareSendPayment(request: prepareRequest);

// If the fees are acceptable, continue to create the HTLC Payment
if (prepareResponse.paymentMethod is SendPaymentMethod.SparkAddress sparkMethod)
{
    var fee = sparkMethod.fee;
    Console.WriteLine($"Fees: {fee} sats");
}

var preimage = "<32-byte unique preimage hex>";
var preimageBytes = Convert.FromHexString(preimage);
var paymentHashBytes = System.Security.Cryptography.SHA256.HashData(preimageBytes);
var paymentHash = Convert.ToHexString(paymentHashBytes).ToLower();

// Set the HTLC options
var options = new SendPaymentOptions.SparkAddress(
    htlcOptions: new SparkHtlcOptions(
        paymentHash: paymentHash,
        expiryDurationSecs: 1000
    )
);

var request = new SendPaymentRequest(
    prepareResponse: prepareResponse,
    options: options
);
var sendResponse = await sdk.SendPayment(request: request);
var payment = sendResponse.payment;
```

### Javascript (Wasm)

```typescript
const paymentRequest = '<spark address>'
// Set the amount you wish to pay the receiver
const amountSats = BigInt(50_000)

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: { type: 'input', input: paymentRequest },
  amount: amountSats,
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy: undefined
})

// If the fees are acceptable, continue to create the HTLC Payment
if (prepareResponse.paymentMethod.type === 'sparkAddress') {
  const fee = prepareResponse.paymentMethod.fee
  console.debug(`Fees: ${fee} sats`)
}

const preimage = '<32-byte unique preimage hex>'
const preimageBuffer = Buffer.from(preimage, 'hex')
const paymentHash = createHash('sha256').update(preimageBuffer).digest('hex')

const sendResponse = await sdk.sendPayment({
  prepareResponse,
  options: {
    type: 'sparkAddress',
    htlcOptions: {
      paymentHash,
      expiryDurationSecs: 1000
    }
  }
})
const payment = sendResponse.payment
```

### React Native

```typescript
const paymentRequest = '<spark address>'
// Set the amount you wish to pay the receiver
const amountSats = BigInt(50_000)
const prepareRequest = {
  paymentRequest: new PaymentRequest.Input({ input: paymentRequest }),
  amount: amountSats,
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy: undefined
}
const prepareResponse = await sdk.prepareSendPayment(prepareRequest)

// If the fees are acceptable, continue to create the HTLC Payment
if (prepareResponse.paymentMethod?.tag === SendPaymentMethod_Tags.SparkAddress) {
  const fee = prepareResponse.paymentMethod.inner.fee
  console.debug(`Fees: ${fee} sats`)
}

const preimage = '<32-byte unique preimage hex>'
const preimageBuffer = Buffer.from(preimage, 'hex')
const paymentHash = createHash('sha256').update(preimageBuffer).digest('hex')

// Set the HTLC options
const options = new SendPaymentOptions.SparkAddress({
  htlcOptions: {
    paymentHash,
    expiryDurationSecs: BigInt(1000)
  }
})

const request = {
  prepareResponse,
  options,
  idempotencyKey: undefined
}
const sendResponse = await sdk.sendPayment(request)
const payment = sendResponse.payment
```

### Flutter

```dart
String paymentRequest = "<spark address>";
// Set the amount you wish the pay the receiver
BigInt? amountSats = BigInt.from(50000);
final prepareRequest = PrepareSendPaymentRequest(
    paymentRequest: PaymentRequest.input(input: paymentRequest),
    amount: amountSats,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: null);
final prepareResponse = await sdk.prepareSendPayment(request: prepareRequest);

// If the fees are acceptable, continue to create the HTLC Payment
final paymentMethod = prepareResponse.paymentMethod;
if (paymentMethod is SendPaymentMethod_SparkAddress) {
  final fee = paymentMethod.fee;
  print("Fees: $fee sats");
}

String preimage = "<32-byte unique preimage hex>";
List<int> preimageBytes = hex.decode(preimage);
Digest paymentHashDigest = sha256.convert(preimageBytes);
String paymentHash = hex.encode(paymentHashDigest.bytes);

// Set the HTLC options
final htlcOptions = SparkHtlcOptions(
    paymentHash: paymentHash, expiryDurationSecs: BigInt.from(1000));
final options = SendPaymentOptions.sparkAddress(htlcOptions: htlcOptions);

final request =
    SendPaymentRequest(prepareResponse: prepareResponse, options: options);
final sendResponse = await sdk.sendPayment(request: request);
final payment = sendResponse.payment;
```

### Python

```python
payment_request = "<spark address>"
amount_sats = 50_000
prepare_request = PrepareSendPaymentRequest(
    payment_request=PaymentRequest.INPUT(input=payment_request),
    amount=amount_sats,
    token_identifier=None,
    conversion_options=None,
    fee_policy=None,
)
prepare_response = await sdk.prepare_send_payment(request=prepare_request)

# If the fees are acceptable, continue to create the HTLC Payment
if hasattr(prepare_response.payment_method, "fee"):
    fee = prepare_response.payment_method.fee
    logging.debug(f"Fees: {fee} sats")

preimage = "<32-byte unique preimage hex>"
preimage_bytes = bytes.fromhex(preimage)
payment_hash_bytes = hashlib.sha256(preimage_bytes).digest()
payment_hash = payment_hash_bytes.hex()

# Set the HTLC options
options = SendPaymentOptions.SPARK_ADDRESS(
    htlc_options=SparkHtlcOptions(
        payment_hash=payment_hash, expiry_duration_secs=1000
    )
)

request = SendPaymentRequest(
    prepare_response=prepare_response, options=options
)
send_response = await sdk.send_payment(request=request)
payment = send_response.payment
```

### Go

```go
paymentRequest := "<spark address>"
// Set the amount you wish to pay the receiver
amountSats := new(big.Int).SetInt64(50_000)
prepareRequest := breez_sdk_spark.PrepareSendPaymentRequest{
	PaymentRequest:    breez_sdk_spark.PaymentRequestInput{Input: paymentRequest},
	Amount:            &amountSats,
	TokenIdentifier:   nil,
	ConversionOptions: nil,
	FeePolicy:         nil,
}
prepareResponse, err := sdk.PrepareSendPayment(prepareRequest)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

// If the fees are acceptable, continue to create the HTLC Payment
switch paymentMethod := prepareResponse.PaymentMethod.(type) {
case breez_sdk_spark.SendPaymentMethodSparkAddress:
	fee := paymentMethod.Fee
	log.Printf("Fees: %v sats", fee)
}

preimage := "<32-byte unique preimage hex>"
preimageBytes, err := hex.DecodeString(preimage)
if err != nil {
	return nil, err
}
paymentHashBytes := sha256.Sum256(preimageBytes)
paymentHash := hex.EncodeToString(paymentHashBytes[:])

// Set the HTLC options
htlcOptions := breez_sdk_spark.SparkHtlcOptions{
	PaymentHash:        paymentHash,
	ExpiryDurationSecs: 1000,
}
var options breez_sdk_spark.SendPaymentOptions = breez_sdk_spark.SendPaymentOptionsSparkAddress{
	HtlcOptions: &htlcOptions,
}

request := breez_sdk_spark.SendPaymentRequest{
	PrepareResponse: prepareResponse,
	Options:         &options,
}
sendResponse, err := sdk.SendPayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

payment := sendResponse.Payment
```



## Receiving using HODL invoices

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

You can receive using HODL invoices — Lightning invoices where the payment is held until you claim it by revealing the preimage. To create one, provide a `payment_hash` when calling `receive_payment` with the `ReceivePaymentMethod::Bolt11Invoice` payment method.

### Rust

```rust
let preimage = "<32-byte unique preimage hex>";
let preimage_bytes = hex::decode(preimage)?;
let payment_hash_bytes = sha256::digest(preimage_bytes);
let payment_hash = hex::encode(payment_hash_bytes);

let response = sdk
    .receive_payment(ReceivePaymentRequest {
        payment_method: ReceivePaymentMethod::Bolt11Invoice {
            description: "HODL invoice".to_string(),
            amount_sats: Some(50_000),
            expiry_secs: None,
            payment_hash: Some(payment_hash),
            receiver_identity_public_key: None,
        },
    })
    .await?;

let invoice = response.payment_request;
info!("HODL invoice: {invoice}");
```

### Swift

```swift
let preimage = "<32-byte unique preimage hex>"
let preimageData = Data(hexString: preimage)!
let paymentHashDigest = SHA256.hash(data: preimageData)
let paymentHash = Data(paymentHashDigest).hexEncodedString()

let response = try await sdk.receivePayment(
    request: ReceivePaymentRequest(
        paymentMethod: ReceivePaymentMethod.bolt11Invoice(
            description: "HODL invoice",
            amountSats: 50_000,
            expirySecs: nil,
            paymentHash: paymentHash,
            receiverIdentityPublicKey: nil
        )
    )
)

let invoice = response.paymentRequest
print("HODL invoice: \(invoice)")
```

### Kotlin

```kotlin
try {
    val preimage = "<32-byte unique preimage hex>"
    val preimageBytes = preimage.hexToByteArray()
    val digest = SHA256()
    digest.update(preimageBytes)
    val paymentHashBytes = digest.digest()
    val paymentHash = paymentHashBytes.toHexString()

    val response = sdk.receivePayment(
        ReceivePaymentRequest(
            paymentMethod = ReceivePaymentMethod.Bolt11Invoice(
                description = "HODL invoice",
                amountSats = 50_000u,
                expirySecs = null,
                paymentHash = paymentHash,
                receiverIdentityPublicKey = null
            )
        )
    )

    val invoice = response.paymentRequest
    // Log.v("Breez", "HODL invoice: $invoice")
} catch (e: Exception) {
    // handle error
    throw e
}
```

### C#

```csharp
var preimage = "<32-byte unique preimage hex>";
var preimageBytes = Convert.FromHexString(preimage);
var paymentHashBytes = System.Security.Cryptography.SHA256.HashData(preimageBytes);
var paymentHash = Convert.ToHexString(paymentHashBytes).ToLower();

var response = await sdk.ReceivePayment(
    request: new ReceivePaymentRequest(
        paymentMethod: new ReceivePaymentMethod.Bolt11Invoice(
            description: "HODL invoice",
            amountSats: 50_000UL,
            expirySecs: null,
            paymentHash: paymentHash,
            receiverIdentityPublicKey: null
        )
    )
);

var invoice = response.paymentRequest;
Console.WriteLine($"HODL invoice: {invoice}");
```

### Javascript (Wasm)

```typescript
const preimage = '<32-byte unique preimage hex>'
const preimageBuffer = Buffer.from(preimage, 'hex')
const paymentHash = createHash('sha256').update(preimageBuffer).digest('hex')

const response = await sdk.receivePayment({
  paymentMethod: {
    type: 'bolt11Invoice',
    description: 'HODL invoice',
    amountSats: 50_000,
    expirySecs: undefined,
    paymentHash,
    receiverIdentityPublicKey: undefined
  }
})

const invoice = response.paymentRequest
console.log(`HODL invoice: ${invoice}`)
```

### React Native

```typescript
const preimage = '<32-byte unique preimage hex>'
const preimageBuffer = Buffer.from(preimage, 'hex')
const paymentHash = createHash('sha256').update(preimageBuffer).digest('hex')

const response = await sdk.receivePayment({
  paymentMethod: new ReceivePaymentMethod.Bolt11Invoice({
    description: 'HODL invoice',
    amountSats: BigInt(50_000),
    expirySecs: undefined,
    paymentHash,
    receiverIdentityPublicKey: undefined
  })
})

const invoice = response.paymentRequest
console.log(`HODL invoice: ${invoice}`)
```

### Flutter

```dart
String preimage = "<32-byte unique preimage hex>";
List<int> preimageBytes = hex.decode(preimage);
Digest paymentHashDigest = sha256.convert(preimageBytes);
String paymentHash = hex.encode(paymentHashDigest.bytes);

final response = await sdk.receivePayment(
    request: ReceivePaymentRequest(
        paymentMethod: ReceivePaymentMethod.bolt11Invoice(
            description: "HODL invoice",
            amountSats: BigInt.from(50000),
            expirySecs: null,
            paymentHash: paymentHash)));

final invoice = response.paymentRequest;
print("HODL invoice: $invoice");
```

### Python

```python
preimage = "<32-byte unique preimage hex>"
preimage_bytes = bytes.fromhex(preimage)
payment_hash_bytes = hashlib.sha256(preimage_bytes).digest()
payment_hash = payment_hash_bytes.hex()

response = await sdk.receive_payment(
    request=ReceivePaymentRequest(
        payment_method=ReceivePaymentMethod.BOLT11_INVOICE(
            description="HODL invoice",
            amount_sats=50_000,
            expiry_secs=None,
            payment_hash=payment_hash,
            receiver_identity_public_key=None,
        )
    )
)

invoice = response.payment_request
logging.debug(f"HODL invoice: {invoice}")
```

### Go

```go
preimage := "<32-byte unique preimage hex>"
preimageBytes, err := hex.DecodeString(preimage)
if err != nil {
	return err
}
paymentHashBytes := sha256.Sum256(preimageBytes)
paymentHash := hex.EncodeToString(paymentHashBytes[:])

amountSats := uint64(50_000)
response, err := sdk.ReceivePayment(breez_sdk_spark.ReceivePaymentRequest{
	PaymentMethod: breez_sdk_spark.ReceivePaymentMethodBolt11Invoice{
		Description: "HODL invoice",
		AmountSats:  &amountSats,
		ExpirySecs:  nil,
		PaymentHash: &paymentHash,
	},
})

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}

invoice := response.PaymentRequest
log.Printf("HODL invoice: %v", invoice)
```



## Listing claimable conditional payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

Once detected, claimable HTLC payments are immediately listed as pending in the [list of payments](/guide/list_payments.md). Additionally, a `SdkEvent::PaymentPending` event is emitted to notify your application. See [Listening to events](/guide/events.md) for more details.

To list only claimable HTLC payments, you can filter by HTLC status. This works for both Spark HTLC payments and HODL invoices.

### Rust

```rust
let request = ListPaymentsRequest {
    type_filter: Some(vec![PaymentType::Receive]),
    status_filter: Some(vec![PaymentStatus::Pending]),
    payment_details_filter: Some(vec![
        PaymentDetailsFilter::Spark {
            htlc_status: Some(vec![SparkHtlcStatus::WaitingForPreimage]),
            conversion_refund_needed: None,
        },
        PaymentDetailsFilter::Lightning {
            htlc_status: Some(vec![SparkHtlcStatus::WaitingForPreimage]),
        },
    ]),
    ..Default::default()
};

let response = sdk.list_payments(request).await?;
let payments = response.payments;

for payment in &payments {
    match &payment.details {
        Some(PaymentDetails::Spark {
            htlc_details: Some(htlc),
            ..
        }) => {
            info!("Spark HTLC expiry time: {}", htlc.expiry_time);
        }
        Some(PaymentDetails::Lightning {
            htlc_details: htlc, ..
        }) => {
            info!("Lightning HTLC expiry time: {}", htlc.expiry_time);
        }
        _ => {}
    }
}
```

### Swift

```swift
let request = ListPaymentsRequest(
    typeFilter: [PaymentType.receive],
    statusFilter: [PaymentStatus.pending],
    paymentDetailsFilter: [
        PaymentDetailsFilter.spark(
            htlcStatus: [SparkHtlcStatus.waitingForPreimage],
            conversionRefundNeeded: nil
        ),
        PaymentDetailsFilter.lightning(
            htlcStatus: [SparkHtlcStatus.waitingForPreimage]
        ),
    ]
)

let response = try await sdk.listPayments(request: request)
let payments = response.payments

for payment in payments {
    if case let .spark(_, htlcDetails, _) = payment.details, let htlc = htlcDetails {
        print("Spark HTLC expiry time: \(htlc.expiryTime)")
    } else if case let .lightning(_, _, _, htlcDetails, _, _, _, _) = payment.details {
        print("Lightning HTLC expiry time: \(htlcDetails.expiryTime)")
    }
}
```

### Kotlin

```kotlin
try {
    val request = ListPaymentsRequest(
        typeFilter = listOf(PaymentType.RECEIVE),
        statusFilter = listOf(PaymentStatus.PENDING),
        paymentDetailsFilter = listOf(
            PaymentDetailsFilter.Spark(
                htlcStatus = listOf(SparkHtlcStatus.WAITING_FOR_PREIMAGE),
                conversionRefundNeeded = null
            ),
            PaymentDetailsFilter.Lightning(
                htlcStatus = listOf(SparkHtlcStatus.WAITING_FOR_PREIMAGE)
            )
        )
    )

    val response = sdk.listPayments(request)
    val payments = response.payments

    for (payment in payments) {
        val details = payment.details
        when (details) {
            is PaymentDetails.Spark -> {
                val htlc = details.htlcDetails
                if (htlc != null) {
                    // Log.v("Breez", "Spark HTLC expiry time: ${htlc.expiryTime}")
                }
            }
            is PaymentDetails.Lightning -> {
                val htlc = details.htlcDetails
                // Log.v("Breez", "Lightning HTLC expiry time: ${htlc.expiryTime}")
            }
            else -> {}
        }
    }
} catch (e: Exception) {
    // handle error
    throw e
}
```

### C#

```csharp
var request = new ListPaymentsRequest(
    typeFilter: new PaymentType[] { PaymentType.Receive },
    statusFilter: new PaymentStatus[] { PaymentStatus.Pending },
    paymentDetailsFilter: new PaymentDetailsFilter[] {
        new PaymentDetailsFilter.Spark(
            htlcStatus: new SparkHtlcStatus[] {
                SparkHtlcStatus.WaitingForPreimage
            },
            conversionRefundNeeded: null
        ),
        new PaymentDetailsFilter.Lightning(
            htlcStatus: new SparkHtlcStatus[] {
                SparkHtlcStatus.WaitingForPreimage
            }
        )
    }
);

var response = await sdk.ListPayments(request: request);
var payments = response.payments;

foreach (var payment in payments)
{
    if (payment.details is PaymentDetails.Spark sparkDetails && sparkDetails.htlcDetails != null)
    {
        Console.WriteLine($"Spark HTLC expiry time: {sparkDetails.htlcDetails.expiryTime}");
    }
    else if (payment.details is PaymentDetails.Lightning lightningDetails)
    {
        Console.WriteLine($"Lightning HTLC expiry time: {lightningDetails.htlcDetails.expiryTime}");
    }
}
```

### Javascript (Wasm)

```typescript
const response = await sdk.listPayments({
  typeFilter: ['receive'],
  statusFilter: ['pending'],
  paymentDetailsFilter: [{
    type: 'spark',
    htlcStatus: ['waitingForPreimage']
  }, {
    type: 'lightning',
    htlcStatus: ['waitingForPreimage']
  }],
  assetFilter: undefined
})
const payments = response.payments

for (const payment of payments) {
  if (payment.details?.type === 'spark' && payment.details.htlcDetails != null) {
    console.log(`Spark HTLC expiry time: ${payment.details.htlcDetails.expiryTime}`)
  } else if (payment.details?.type === 'lightning') {
    console.log(`Lightning HTLC expiry time: ${payment.details.htlcDetails.expiryTime}`)
  }
}
```

### React Native

```typescript
const request = {
  typeFilter: [PaymentType.Receive],
  statusFilter: [PaymentStatus.Pending],
  paymentDetailsFilter: [new PaymentDetailsFilter.Spark({
    htlcStatus: [SparkHtlcStatus.WaitingForPreimage],
    conversionRefundNeeded: undefined
  }), new PaymentDetailsFilter.Lightning({
    htlcStatus: [SparkHtlcStatus.WaitingForPreimage]
  })],
  assetFilter: undefined,
  fromTimestamp: undefined,
  toTimestamp: undefined,
  offset: undefined,
  limit: undefined,
  sortAscending: undefined
}

const response = await sdk.listPayments(request)
const payments = response.payments

for (const payment of payments) {
  if (payment.details?.tag === PaymentDetails_Tags.Spark) {
    const htlc = payment.details.inner.htlcDetails
    if (htlc != null) {
      console.log(`Spark HTLC expiry time: ${htlc.expiryTime}`)
    }
  } else if (payment.details?.tag === PaymentDetails_Tags.Lightning) {
    const htlc = payment.details.inner.htlcDetails
    console.log(`Lightning HTLC expiry time: ${htlc.expiryTime}`)
  }
}
```

### Flutter

```dart
final request = ListPaymentsRequest(
  typeFilter: [PaymentType.receive],
  statusFilter: [PaymentStatus.pending],
  paymentDetailsFilter: [
    PaymentDetailsFilter.spark(
      htlcStatus: [SparkHtlcStatus.waitingForPreimage],
    ),
    PaymentDetailsFilter.lightning(
      htlcStatus: [SparkHtlcStatus.waitingForPreimage],
    ),
  ],
);

final response = await sdk.listPayments(request: request);
final payments = response.payments;

for (final payment in payments) {
  final details = payment.details;
  if (details is PaymentDetails_Spark && details.htlcDetails != null) {
    print("Spark HTLC expiry time: ${details.htlcDetails!.expiryTime}");
  } else if (details is PaymentDetails_Lightning) {
    print("Lightning HTLC expiry time: ${details.htlcDetails.expiryTime}");
  }
}
```

### Python

```python
request = ListPaymentsRequest(
    type_filter=[PaymentType.RECEIVE],
    status_filter=[PaymentStatus.PENDING],
    payment_details_filter=[
        cast(PaymentDetailsFilter, PaymentDetailsFilter.SPARK(
            htlc_status=[SparkHtlcStatus.WAITING_FOR_PREIMAGE],
            conversion_refund_needed=None
        )),
        cast(PaymentDetailsFilter, PaymentDetailsFilter.LIGHTNING(
            htlc_status=[SparkHtlcStatus.WAITING_FOR_PREIMAGE],
        )),
    ],
)

response = await sdk.list_payments(request=request)
payments = response.payments

for payment in payments:
    if isinstance(payment.details, PaymentDetails.SPARK):
        if payment.details.htlc_details is not None:
            logging.debug(f"Spark HTLC expiry time: {payment.details.htlc_details.expiry_time}")
    elif isinstance(payment.details, PaymentDetails.LIGHTNING):
        expiry = payment.details.htlc_details.expiry_time
        logging.debug(f"Lightning HTLC expiry time: {expiry}")
```

### Go

```go
typeFilter := []breez_sdk_spark.PaymentType{
	breez_sdk_spark.PaymentTypeReceive,
}
statusFilter := []breez_sdk_spark.PaymentStatus{
	breez_sdk_spark.PaymentStatusPending,
}
paymentDetailsFilter := []breez_sdk_spark.PaymentDetailsFilter{
	breez_sdk_spark.PaymentDetailsFilterSpark{
		HtlcStatus: &[]breez_sdk_spark.SparkHtlcStatus{
			breez_sdk_spark.SparkHtlcStatusWaitingForPreimage,
		},
	},
	breez_sdk_spark.PaymentDetailsFilterLightning{
		HtlcStatus: &[]breez_sdk_spark.SparkHtlcStatus{
			breez_sdk_spark.SparkHtlcStatusWaitingForPreimage,
		},
	},
}

request := breez_sdk_spark.ListPaymentsRequest{
	TypeFilter:            &typeFilter,
	StatusFilter:          &statusFilter,
	PaymentDetailsFilter:  &paymentDetailsFilter,
}

response, err := sdk.ListPayments(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

payments := response.Payments

for _, payment := range payments {
	if payment.Details != nil {
		switch details := (*payment.Details).(type) {
		case breez_sdk_spark.PaymentDetailsSpark:
			if details.HtlcDetails != nil {
				log.Printf("Spark HTLC expiry time: %v", details.HtlcDetails.ExpiryTime)
			}
		case breez_sdk_spark.PaymentDetailsLightning:
			log.Printf("Lightning HTLC expiry time: %v", details.HtlcDetails.ExpiryTime)
		}
	}
}
```



## Claiming conditional payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.claim_htlc_payment

To claim an HTLC payment, provide the preimage that matches the payment hash. This works for both Spark HTLC payments and HODL invoices.

### Rust

```rust
let preimage = "<preimage hex>".to_string();
let response = sdk
    .claim_htlc_payment(ClaimHtlcPaymentRequest { preimage })
    .await?;
let payment = response.payment;
```

### Swift

```swift
let preimage = "<preimage hex>"
let response = try await sdk.claimHtlcPayment(
    request: ClaimHtlcPaymentRequest(preimage: preimage)
)
let payment = response.payment
```

### Kotlin

```kotlin
try {
    val preimage = "<preimage hex>"
    val request = ClaimHtlcPaymentRequest(preimage = preimage)
    val response = sdk.claimHtlcPayment(request)
    val payment = response.payment
} catch (e: Exception) {
    // handle error
    throw e
}
```

### C#

```csharp
var preimage = "<preimage hex>";
var response = await sdk.ClaimHtlcPayment(
    request: new ClaimHtlcPaymentRequest(preimage: preimage)
);
var payment = response.payment;
```

### Javascript (Wasm)

```typescript
const preimage = '<preimage hex>'
const response = await sdk.claimHtlcPayment({
  preimage
})
const payment = response.payment
```

### React Native

```typescript
const preimage = '<preimage hex>'
const response = await sdk.claimHtlcPayment(
  { preimage }
)
const payment = response.payment
```

### Flutter

```dart
String preimage = "<preimage hex>";
final response = await sdk.claimHtlcPayment(
    request: ClaimHtlcPaymentRequest(preimage: preimage));
final payment = response.payment;
```

### Python

```python
preimage = "<preimage hex>"
response = await sdk.claim_htlc_payment(
    request=ClaimHtlcPaymentRequest(preimage=preimage)
)
payment = response.payment
```

### Go

```go
preimage := "<preimage hex>"
request := breez_sdk_spark.ClaimHtlcPaymentRequest{
	Preimage: preimage,
}
response, err := sdk.ClaimHtlcPayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

payment := response.Payment
```



---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
