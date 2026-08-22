# Sending payments

Once the SDK is initialized, you can directly begin sending payments. The send process takes two steps:

1. [Preparing the Payment](send_payment.md#preparing-payments)
2. [Sending the Payment](send_payment.md#sending-payments)

For sending payments via LNURL, see [LNURL-Pay](lnurl_pay.md).

## Preparing Payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_payment

During the prepare step, the SDK ensures that the inputs are valid with respect to the payment request type,
and also returns the fees related to the payment so they can be confirmed.

The payment request field supports Lightning invoices, Bitcoin addresses, Spark addresses and Spark invoices.

**Developer note**

Payments can be sent without holding Bitcoin by converting on-the-fly as a step before sending a payment. See <a href="./token_conversion.md">Converting tokens</a> for more information.

### Lightning

#### BOLT11 invoice

For BOLT11 invoices the amount can be optionally set. It is only required if the invoice doesn't specify an amount. If the invoice specifies an amount, providing a different amount is not supported.

If the invoice also contains a Spark address, the payment can be sent directly via a Spark transfer instead. When this is the case, the prepare response includes the Spark transfer fee. Note that only one fee is paid: either the Lightning fee or the Spark transfer fee, depending on which payment method is ultimately used. See [Lightning](send_payment.md#lightning-1) for how to select the payment method.

##### Rust

```rust
let payment_request = "<bolt11 invoice>".to_string();
// Optionally set the amount you wish to pay the receiver
let optional_amount_sats = Some(5_000);

let prepare_response = sdk
    .prepare_send_payment(PrepareSendPaymentRequest {
        payment_request: PaymentRequest::Input {
            input: payment_request,
        },
        amount: optional_amount_sats,
        token_identifier: None,
        conversion_options: None,
        fee_policy: None,
    })
    .await?;

// If the fees are acceptable, continue to create the Send Payment
if let SendPaymentMethod::Bolt11Invoice {
    spark_transfer_fee_sats,
    lightning_fee_sats,
    ..
} = prepare_response.payment_method
{
    // Fees to pay via Lightning
    info!("Lightning Fees: {lightning_fee_sats} sats");
    // Or fees to pay (if available) via a Spark transfer
    info!("Spark Transfer Fees: {spark_transfer_fee_sats:?} sats");
}
```

##### Swift

```swift
let paymentRequest = "<bolt11 invoice>"
// Optionally set the amount you wish to pay the receiver
let optionalAmountSats: BInt? = BInt(5_000)

let prepareResponse = try await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
        paymentRequest: .input(input: paymentRequest),
        amount: optionalAmountSats,
        tokenIdentifier: nil,
        conversionOptions: nil,
        feePolicy: nil
    ))

if case let .bolt11Invoice(_, sparkTransferFeeSats, lightningFeeSats) = prepareResponse
    .paymentMethod
{
    // Fees to pay via Lightning
    print("Lightning Fees: \(lightningFeeSats) sats")
    // Or fees to pay (if available) via a Spark transfer
    if let sparkTransferFeeSats = sparkTransferFeeSats {
        print("Spark Transfer Fees: \(sparkTransferFeeSats) sats")
    }
}
```

##### Kotlin

```kotlin
val paymentRequest = "<bolt11 invoice>"
// Optionally set the amount you wish to pay the receiver
// Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer)
val optionalAmountSats = BigInteger.fromLong(5_000L)
// Android (BigInteger from java.math)
// val optionalAmountSats = BigInteger.valueOf(5_000L)

try {
    val req = PrepareSendPaymentRequest(
        paymentRequest = PaymentRequest.Input(input = paymentRequest),
        amount = optionalAmountSats,
        tokenIdentifier = null,
        conversionOptions = null,
        feePolicy = null,
    )
    val prepareResponse = sdk.prepareSendPayment(req)

    // If the fees are acceptable, continue to create the Send Payment
    val paymentMethod = prepareResponse.paymentMethod
    if (paymentMethod is SendPaymentMethod.Bolt11Invoice) {
        // Fees to pay via Lightning
        val lightningFeeSats = paymentMethod.lightningFeeSats
        // Or fees to pay (if available) via a Spark transfer
        val sparkTransferFeeSats = paymentMethod.sparkTransferFeeSats
        // Log.v("Breez", "Lightning Fees: ${lightningFeeSats} sats")
        // Log.v("Breez", "Spark Transfer Fees: ${sparkTransferFeeSats} sats")
    }
} catch (e: Exception) {
    // handle error
}
```

##### C#

```csharp
var paymentRequest = "<bolt11 invoice>";
// Optionally set the amount you wish to pay the receiver
ulong? optionalAmountSats = 5_000UL;

var request = new PrepareSendPaymentRequest(
    paymentRequest: new PaymentRequest.Input(input: paymentRequest),
    amount: optionalAmountSats,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: null
);
var prepareResponse = await sdk.PrepareSendPayment(request: request);

// If the fees are acceptable, continue to create the Send Payment
if (prepareResponse.paymentMethod is SendPaymentMethod.Bolt11Invoice bolt11Method)
{
    // Fees to pay via Lightning
    var lightningFeeSats = bolt11Method.lightningFeeSats;
    // Or fees to pay (if available) via a Spark transfer
    var sparkTransferFeeSats = bolt11Method.sparkTransferFeeSats;
    Console.WriteLine($"Lightning Fees: {lightningFeeSats} sats");
    Console.WriteLine($"Spark Transfer Fees: {sparkTransferFeeSats} sats");
}
```

##### Javascript (Wasm)

```typescript
const paymentRequest = '<bolt11 invoice>'
// Optionally set the amount you wish to pay the receiver
const optionalAmountSats = BigInt(5_000)

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: { type: 'input', input: paymentRequest },
  amount: optionalAmountSats,
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy: undefined
})

// If the fees are acceptable, continue to create the Send Payment
if (prepareResponse.paymentMethod.type === 'bolt11Invoice') {
  // Fees to pay via Lightning
  const lightningFeeSats = prepareResponse.paymentMethod.lightningFeeSats
  // Or fees to pay (if available) via a Spark transfer
  const sparkTransferFeeSats = prepareResponse.paymentMethod.sparkTransferFeeSats
  console.debug(`Lightning Fees: ${lightningFeeSats} sats`)
  console.debug(`Spark Transfer Fees: ${sparkTransferFeeSats} sats`)
}
```

##### React Native

```typescript
const paymentRequest = '<bolt11 invoice>'
// Optionally set the amount you wish to pay the receiver
const optionalAmountSats = BigInt(5_000)

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: new PaymentRequest.Input({ input: paymentRequest }),
  amount: optionalAmountSats,
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy: undefined
})

// If the fees are acceptable, continue to create the Send Payment
if (prepareResponse.paymentMethod?.tag === SendPaymentMethod_Tags.Bolt11Invoice) {
  // Fees to pay via Lightning
  const lightningFeeSats = prepareResponse.paymentMethod.inner.lightningFeeSats
  // Or fees to pay (if available) via a Spark transfer
  const sparkTransferFeeSats = prepareResponse.paymentMethod.inner.sparkTransferFeeSats
  console.debug(`Lightning Fees: ${lightningFeeSats} sats`)
  console.debug(`Spark Transfer Fees: ${sparkTransferFeeSats} sats`)
}
```

##### Flutter

```dart
String paymentRequest = "<bolt11 invoice>";
// Optionally set the amount you wish to pay the receiver
BigInt? optionalAmountSats = BigInt.from(5000);

final request = PrepareSendPaymentRequest(
    paymentRequest: PaymentRequest.input(input: paymentRequest),
    amount: optionalAmountSats,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: null);
final response = await sdk.prepareSendPayment(request: request);

// If the fees are acceptable, continue to create the Send Payment
final paymentMethod = response.paymentMethod;
if (paymentMethod is SendPaymentMethod_Bolt11Invoice) {
  // Fees to pay via Lightning
  final lightningFeeSats = paymentMethod.lightningFeeSats;
  // Or fees to pay (if available) via a Spark transfer
  final sparkTransferFeeSats = paymentMethod.sparkTransferFeeSats;
  print("Lightning Fees: $lightningFeeSats sats");
  print("Spark Transfer Fees: $sparkTransferFeeSats sats");
}
```

##### Python

```python
payment_request = "<bolt11 invoice>"
optional_amount_sats = 5_000
try:
    request = PrepareSendPaymentRequest(
        payment_request=PaymentRequest.INPUT(input=payment_request),
        amount=optional_amount_sats,
        token_identifier=None,
        conversion_options=None,
        fee_policy=None,
    )
    prepare_response = await sdk.prepare_send_payment(request=request)

    # If the fees are acceptable, continue to create the Send Payment
    if isinstance(
        prepare_response.payment_method, SendPaymentMethod.BOLT11_INVOICE
    ):
        # Fees to pay via Lightning
        lightning_fee_sats = prepare_response.payment_method.lightning_fee_sats
        # Or fees to pay (if available) via a Spark transfer
        spark_transfer_fee_sats = (
            prepare_response.payment_method.spark_transfer_fee_sats
        )
        logging.debug(f"Lightning Fees: {lightning_fee_sats} sats")
        logging.debug(f"Spark Transfer Fees: {spark_transfer_fee_sats} sats")
except Exception as error:
    logging.error(error)
    raise
```

##### Go

```go
paymentRequest := "<bolt11 invoice>"
// Optionally set the amount you wish to pay the receiver
optionalAmountSats := new(big.Int).SetInt64(5_000)

request := breez_sdk_spark.PrepareSendPaymentRequest{
	PaymentRequest:    breez_sdk_spark.PaymentRequestInput{Input: paymentRequest},
	Amount:            &optionalAmountSats,
	TokenIdentifier:   nil,
	ConversionOptions: nil,
	FeePolicy:         nil,
}
response, err := sdk.PrepareSendPayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

// If the fees are acceptable, continue to create the Send Payment
switch paymentMethod := response.PaymentMethod.(type) {
case breez_sdk_spark.SendPaymentMethodBolt11Invoice:
	// Fees to pay via Lightning
	lightningFeeSats := paymentMethod.LightningFeeSats
	// Or fees to pay (if available) via a Spark transfer
	sparkTransferFeeSats := paymentMethod.SparkTransferFeeSats
	log.Printf("Lightning Fees: %v sats", lightningFeeSats)
	log.Printf("Spark Transfer Fees: %v sats", sparkTransferFeeSats)
}
```



### Bitcoin

For Bitcoin addresses, the amount must be set in the request. The prepare response includes fee quotes for three payment speeds: Slow, Medium, and Fast.

#### Rust

```rust
let payment_request = "<bitcoin address>".to_string();
// Set the amount you wish to pay the receiver
let amount_sats = Some(50_000);

let prepare_response = sdk
    .prepare_send_payment(PrepareSendPaymentRequest {
        payment_request: PaymentRequest::Input {
            input: payment_request,
        },
        amount: amount_sats,
        token_identifier: None,
        conversion_options: None,
        fee_policy: None,
    })
    .await?;

// Review the fee quote for each confirmation speed
if let SendPaymentMethod::BitcoinAddress { fee_quote, .. } = &prepare_response.payment_method {
    info!("Slow fee: {} sats", fee_quote.speed_slow.total_fee_sat());
    info!(
        "Medium fee: {} sats",
        fee_quote.speed_medium.total_fee_sat()
    );
    info!("Fast fee: {} sats", fee_quote.speed_fast.total_fee_sat());
}
```

#### Swift

```swift
let paymentRequest = "<bitcoin address>"
// Set the amount you wish to pay the receiver
let amountSats = BInt(50_000)

let prepareResponse = try await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
        paymentRequest: .input(input: paymentRequest),
        amount: amountSats,
        tokenIdentifier: nil,
        conversionOptions: nil,
        feePolicy: nil
    ))

// Review the fee quote for each confirmation speed
if case let .bitcoinAddress(address: _, feeQuote: feeQuote) = prepareResponse.paymentMethod {
    let slowFeeSats = feeQuote.speedSlow.userFeeSat + feeQuote.speedSlow.l1BroadcastFeeSat
    let mediumFeeSats = feeQuote.speedMedium.userFeeSat + feeQuote.speedMedium.l1BroadcastFeeSat
    let fastFeeSats = feeQuote.speedFast.userFeeSat + feeQuote.speedFast.l1BroadcastFeeSat
    print("Slow fee: \(slowFeeSats) sats")
    print("Medium fee: \(mediumFeeSats) sats")
    print("Fast fee: \(fastFeeSats) sats")
}
```

#### Kotlin

```kotlin
val paymentRequest = "<bitcoin address>"
// Set the amount you wish to pay the receiver
// Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer)
val amountSats = BigInteger.fromLong(50_000L)
// Android (BigInteger from java.math)
// val amountSats = BigInteger.valueOf(50_000L)

try {
    val req = PrepareSendPaymentRequest(
        paymentRequest = PaymentRequest.Input(input = paymentRequest),
        amount = amountSats,
        tokenIdentifier = null,
        conversionOptions = null,
        feePolicy = null,
    )
    val prepareResponse = sdk.prepareSendPayment(req)

    // Review the fee quote for each confirmation speed
    val paymentMethod = prepareResponse.paymentMethod
    if (paymentMethod is SendPaymentMethod.BitcoinAddress) {
        val feeQuote = paymentMethod.feeQuote
        val slowFeeSats = feeQuote.speedSlow.userFeeSat + feeQuote.speedSlow.l1BroadcastFeeSat
        val mediumFeeSats = feeQuote.speedMedium.userFeeSat + feeQuote.speedMedium.l1BroadcastFeeSat
        val fastFeeSats = feeQuote.speedFast.userFeeSat + feeQuote.speedFast.l1BroadcastFeeSat
        // Log.v("Breez", "Slow fee: $slowFeeSats sats")
        // Log.v("Breez", "Medium fee: $mediumFeeSats sats")
        // Log.v("Breez", "Fast fee: $fastFeeSats sats")
    }
} catch (e: Exception) {
    // handle error
}
```

#### C#

```csharp
var paymentRequest = "<bitcoin address>";
// Set the amount you wish to pay the receiver
ulong? amountSats = 50_000UL;

var request = new PrepareSendPaymentRequest(
    paymentRequest: new PaymentRequest.Input(input: paymentRequest),
    amount: amountSats,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: null
);
var prepareResponse = await sdk.PrepareSendPayment(request: request);

// Review the fee quote for each confirmation speed
if (prepareResponse.paymentMethod is SendPaymentMethod.BitcoinAddress bitcoinMethod)
{
    var feeQuote = bitcoinMethod.feeQuote;
    var slowFeeSats = feeQuote.speedSlow.userFeeSat + feeQuote.speedSlow.l1BroadcastFeeSat;
    var mediumFeeSats = feeQuote.speedMedium.userFeeSat + feeQuote.speedMedium.l1BroadcastFeeSat;
    var fastFeeSats = feeQuote.speedFast.userFeeSat + feeQuote.speedFast.l1BroadcastFeeSat;
    Console.WriteLine($"Slow fee: {slowFeeSats} sats");
    Console.WriteLine($"Medium fee: {mediumFeeSats} sats");
    Console.WriteLine($"Fast fee: {fastFeeSats} sats");
}
```

#### Javascript (Wasm)

```typescript
const paymentRequest = '<bitcoin address>'
// Set the amount you wish to pay the receiver
const amountSats = BigInt(50_000)

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: { type: 'input', input: paymentRequest },
  amount: amountSats,
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy: undefined
})

// Review the fee quote for each confirmation speed
if (prepareResponse.paymentMethod.type === 'bitcoinAddress') {
  const feeQuote = prepareResponse.paymentMethod.feeQuote
  const slowFeeSats = feeQuote.speedSlow.userFeeSat + feeQuote.speedSlow.l1BroadcastFeeSat
  const mediumFeeSats = feeQuote.speedMedium.userFeeSat + feeQuote.speedMedium.l1BroadcastFeeSat
  const fastFeeSats = feeQuote.speedFast.userFeeSat + feeQuote.speedFast.l1BroadcastFeeSat
  console.debug(`Slow fee: ${slowFeeSats} sats`)
  console.debug(`Medium fee: ${mediumFeeSats} sats`)
  console.debug(`Fast fee: ${fastFeeSats} sats`)
}
```

#### React Native

```typescript
const paymentRequest = '<bitcoin address>'
// Set the amount you wish to pay the receiver
const amountSats = BigInt(50_000)

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: new PaymentRequest.Input({ input: paymentRequest }),
  amount: amountSats,
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy: undefined
})

// Review the fee quote for each confirmation speed
if (prepareResponse.paymentMethod?.tag === SendPaymentMethod_Tags.BitcoinAddress) {
  const feeQuote = prepareResponse.paymentMethod.inner.feeQuote
  const slowFeeSats = feeQuote.speedSlow.userFeeSat + feeQuote.speedSlow.l1BroadcastFeeSat
  const mediumFeeSats = feeQuote.speedMedium.userFeeSat + feeQuote.speedMedium.l1BroadcastFeeSat
  const fastFeeSats = feeQuote.speedFast.userFeeSat + feeQuote.speedFast.l1BroadcastFeeSat
  console.debug(`Slow fee: ${slowFeeSats} sats`)
  console.debug(`Medium fee: ${mediumFeeSats} sats`)
  console.debug(`Fast fee: ${fastFeeSats} sats`)
}
```

#### Flutter

```dart
String paymentRequest = "<bitcoin address>";
// Set the amount you wish to pay the receiver
BigInt? amountSats = BigInt.from(50000);

final request = PrepareSendPaymentRequest(
    paymentRequest: PaymentRequest.input(input: paymentRequest),
    amount: amountSats,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: null);
final response = await sdk.prepareSendPayment(request: request);

// Review the fee quote for each confirmation speed
final paymentMethod = response.paymentMethod;
if (paymentMethod is SendPaymentMethod_BitcoinAddress) {
  final feeQuote = paymentMethod.feeQuote;
  final slowFeeSats = feeQuote.speedSlow.userFeeSat + feeQuote.speedSlow.l1BroadcastFeeSat;
  final mediumFeeSats = feeQuote.speedMedium.userFeeSat + feeQuote.speedMedium.l1BroadcastFeeSat;
  final fastFeeSats = feeQuote.speedFast.userFeeSat + feeQuote.speedFast.l1BroadcastFeeSat;
  print("Slow fee: $slowFeeSats sats");
  print("Medium fee: $mediumFeeSats sats");
  print("Fast fee: $fastFeeSats sats");
}
```

#### Python

```python
payment_request = "<bitcoin address>"
amount_sats = 50_000
try:
    request = PrepareSendPaymentRequest(
        payment_request=PaymentRequest.INPUT(input=payment_request),
        amount=amount_sats,
        token_identifier=None,
        conversion_options=None,
        fee_policy=None,
    )
    prepare_response = await sdk.prepare_send_payment(request=request)

    # Review the fee quote for each confirmation speed
    if isinstance(
        prepare_response.payment_method, SendPaymentMethod.BITCOIN_ADDRESS
    ):
        fee_quote = prepare_response.payment_method.fee_quote
        slow_fee_sats = (
            fee_quote.speed_slow.user_fee_sat
            + fee_quote.speed_slow.l1_broadcast_fee_sat
        )
        medium_fee_sats = (
            fee_quote.speed_medium.user_fee_sat
            + fee_quote.speed_medium.l1_broadcast_fee_sat
        )
        fast_fee_sats = (
            fee_quote.speed_fast.user_fee_sat
            + fee_quote.speed_fast.l1_broadcast_fee_sat
        )
        logging.debug(f"Slow fee: {slow_fee_sats} sats")
        logging.debug(f"Medium fee: {medium_fee_sats} sats")
        logging.debug(f"Fast fee: {fast_fee_sats} sats")
except Exception as error:
    logging.error(error)
    raise
```

#### Go

```go
paymentRequest := "<bitcoin address>"
// Set the amount you wish to pay the receiver
amountSats := new(big.Int).SetInt64(50_000)

request := breez_sdk_spark.PrepareSendPaymentRequest{
	PaymentRequest:    breez_sdk_spark.PaymentRequestInput{Input: paymentRequest},
	Amount:            &amountSats,
	TokenIdentifier:   nil,
	ConversionOptions: nil,
	FeePolicy:         nil,
}
response, err := sdk.PrepareSendPayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

// Review the fee quote for each confirmation speed
switch paymentMethod := response.PaymentMethod.(type) {
case breez_sdk_spark.SendPaymentMethodBitcoinAddress:
	feeQuote := paymentMethod.FeeQuote
	slowFeeSats := feeQuote.SpeedSlow.UserFeeSat + feeQuote.SpeedSlow.L1BroadcastFeeSat
	mediumFeeSats := feeQuote.SpeedMedium.UserFeeSat + feeQuote.SpeedMedium.L1BroadcastFeeSat
	fastFeeSats := feeQuote.SpeedFast.UserFeeSat + feeQuote.SpeedFast.L1BroadcastFeeSat
	log.Printf("Slow fee: %v sats", slowFeeSats)
	log.Printf("Medium fee: %v sats", mediumFeeSats)
	log.Printf("Fast fee: %v sats", fastFeeSats)
}
```



### Spark

#### Spark address

For Spark addresses, the amount must be set in the request. Sending to a Spark address uses a direct Spark transfer.

##### Rust

```rust
let payment_request = "<spark address>".to_string();
// Set the amount you wish to pay the receiver
let amount_sats = Some(50_000);

let prepare_response = sdk
    .prepare_send_payment(PrepareSendPaymentRequest {
        payment_request: PaymentRequest::Input {
            input: payment_request,
        },
        amount: amount_sats,
        token_identifier: None,
        conversion_options: None,
        fee_policy: None,
    })
    .await?;

// If the fees are acceptable, continue to create the Send Payment
if let SendPaymentMethod::SparkAddress { fee, .. } = prepare_response.payment_method {
    info!("Fees: {} sats", fee);
}
```

##### Swift

```swift
let paymentRequest = "<spark address>"
// Set the amount you wish to pay the receiver
let amountSats = BInt(50_000)

let prepareResponse = try await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
        paymentRequest: .input(input: paymentRequest),
        amount: amountSats,
        tokenIdentifier: nil,
        conversionOptions: nil,
        feePolicy: nil
    ))

if case let .sparkAddress(_, feeSats, _) = prepareResponse.paymentMethod {
    print("Fees: \(feeSats) sats")
}
```

##### Kotlin

```kotlin
val paymentRequest = "<spark address>"
// Set the amount you wish to pay the receiver
// Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer)
val amountSats = BigInteger.fromLong(50_000L)
// Android (BigInteger from java.math)
// val amountSats = BigInteger.valueOf(50_000L)

try {
    val req = PrepareSendPaymentRequest(
        paymentRequest = PaymentRequest.Input(input = paymentRequest),
        amount = amountSats,
        tokenIdentifier = null,
        conversionOptions = null,
        feePolicy = null,
    )
    val prepareResponse = sdk.prepareSendPayment(req)

    // If the fees are acceptable, continue to create the Send Payment
    val paymentMethod = prepareResponse.paymentMethod
    if (paymentMethod is SendPaymentMethod.SparkAddress) {
        val feeSats = paymentMethod.fee
        // Log.v("Breez", "Fees: ${feeSats} sats")
    }
} catch (e: Exception) {
    // handle error
}
```

##### C#

```csharp
var paymentRequest = "<spark address>";
// Set the amount you wish to pay the receiver
ulong? amountSats = 50_000UL;

var request = new PrepareSendPaymentRequest(
    paymentRequest: new PaymentRequest.Input(input: paymentRequest),
    amount: amountSats,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: null
);
var prepareResponse = await sdk.PrepareSendPayment(request: request);

// If the fees are acceptable, continue to create the Send Payment
if (prepareResponse.paymentMethod is SendPaymentMethod.SparkAddress sparkMethod)
{
    var fee = sparkMethod.fee;
    Console.WriteLine($"Fees: {fee} sats");
}
```

##### Javascript (Wasm)

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

// If the fees are acceptable, continue to create the Send Payment
if (prepareResponse.paymentMethod.type === 'sparkAddress') {
  const feeSats = prepareResponse.paymentMethod.fee
  console.debug(`Fees: ${feeSats} sats`)
}
```

##### React Native

```typescript
const paymentRequest = '<spark address>'
// Set the amount you wish to pay the receiver
const amountSats = BigInt(50_000)

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: new PaymentRequest.Input({ input: paymentRequest }),
  amount: amountSats,
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy: undefined
})

// If the fees are acceptable, continue to create the Send Payment
if (prepareResponse.paymentMethod?.tag === SendPaymentMethod_Tags.SparkAddress) {
  const feeSats = prepareResponse.paymentMethod.inner.fee
  console.debug(`Fees: ${feeSats} sats`)
}
```

##### Flutter

```dart
String paymentRequest = "<spark address>";
// Set the amount you wish to pay the receiver
BigInt? amountSats = BigInt.from(50000);

final request = PrepareSendPaymentRequest(
    paymentRequest: PaymentRequest.input(input: paymentRequest),
    amount: amountSats,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: null);
final response = await sdk.prepareSendPayment(request: request);

// If the fees are acceptable, continue to create the Send Payment
final paymentMethod = response.paymentMethod;
if (paymentMethod is SendPaymentMethod_SparkAddress) {
  final feeSats = paymentMethod.fee;
  print("Fees: $feeSats sats");
}
```

##### Python

```python
payment_request = "<spark address>"
amount_sats = 50_000
try:
    request = PrepareSendPaymentRequest(
        payment_request=PaymentRequest.INPUT(input=payment_request),
        amount=amount_sats,
        token_identifier=None,
        conversion_options=None,
        fee_policy=None,
    )
    prepare_response = await sdk.prepare_send_payment(request=request)

    # If the fees are acceptable, continue to create the Send Payment
    if isinstance(prepare_response.payment_method, SendPaymentMethod.SPARK_ADDRESS):
        fee = prepare_response.payment_method.fee
        logging.debug(f"Fees: {fee} sats")
except Exception as error:
    logging.error(error)
    raise
```

##### Go

```go
paymentRequest := "<spark address>"
// Set the amount you wish to pay the receiver
amountSats := new(big.Int).SetInt64(50_000)

request := breez_sdk_spark.PrepareSendPaymentRequest{
	PaymentRequest:    breez_sdk_spark.PaymentRequestInput{Input: paymentRequest},
	Amount:            &amountSats,
	TokenIdentifier:   nil,
	ConversionOptions: nil,
	FeePolicy:         nil,
}
response, err := sdk.PrepareSendPayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

// If the fees are acceptable, continue to create the Send Payment
switch paymentMethod := response.PaymentMethod.(type) {
case breez_sdk_spark.SendPaymentMethodSparkAddress:
	feeSats := paymentMethod.Fee
	log.Printf("Fees: %v sats", feeSats)
}
```



#### Spark invoice

For Spark invoices, the amount can be optionally set. It is only required if the invoice doesn't specify an amount. If the invoice specifies an amount, providing a different amount is not supported.

**Developer note**

Spark invoices may require a token (non-Bitcoin) as the payment asset. To determine the requirements of a Spark invoice and any restrictions it may impose, see the <a href="./parse.md">Parsing inputs</a> page. To learn more about tokens, see the <a href="./tokens.md">Handling tokens</a> page.

##### Rust

```rust
let payment_request = "<spark invoice>".to_string();
// Optionally set the amount you wish to pay the receiver
let optional_amount_sats = Some(50_000);

let prepare_response = sdk
    .prepare_send_payment(PrepareSendPaymentRequest {
        payment_request: PaymentRequest::Input {
            input: payment_request,
        },
        amount: optional_amount_sats,
        token_identifier: None,
        conversion_options: None,
        fee_policy: None,
    })
    .await?;

// If the fees are acceptable, continue to create the Send Payment
if let SendPaymentMethod::SparkInvoice { fee, .. } = prepare_response.payment_method {
    info!("Fees: {} sats", fee);
}
```

##### Swift

```swift
let paymentRequest = "<spark invoice>"
// Optionally set the amount you wish to pay the receiver
let optionalAmountSats: BInt? = BInt(50_000)

let prepareResponse = try await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
        paymentRequest: .input(input: paymentRequest),
        amount: optionalAmountSats,
        tokenIdentifier: nil,
        conversionOptions: nil,
        feePolicy: nil
    ))

if case let .sparkInvoice(_, feeSats, _) = prepareResponse.paymentMethod {
    print("Fees: \(feeSats) sats")
}
```

##### Kotlin

```kotlin
val paymentRequest = "<spark invoice>"
// Optionally set the amount you wish to pay the receiver
// Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer)
val optionalAmountSats = BigInteger.fromLong(50_000L)
// Android (BigInteger from java.math)
// val optionalAmountSats = BigInteger.valueOf(50_000L)

try {
    val req = PrepareSendPaymentRequest(
        paymentRequest = PaymentRequest.Input(input = paymentRequest),
        amount = optionalAmountSats,
        tokenIdentifier = null,
        conversionOptions = null,
        feePolicy = null,
    )
    val prepareResponse = sdk.prepareSendPayment(req)

    // If the fees are acceptable, continue to create the Send Payment
    val paymentMethod = prepareResponse.paymentMethod
    if (paymentMethod is SendPaymentMethod.SparkInvoice) {
        val feeSats = paymentMethod.fee
        // Log.v("Breez", "Fees: ${feeSats} sats")
    }
} catch (e: Exception) {
    // handle error
}
```

##### C#

```csharp
var paymentRequest = "<spark invoice>";
// Optionally set the amount you wish to pay the receiver
ulong? optionalAmountSats = 50_000UL;

var request = new PrepareSendPaymentRequest(
    paymentRequest: new PaymentRequest.Input(input: paymentRequest),
    amount: optionalAmountSats,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: null
);
var prepareResponse = await sdk.PrepareSendPayment(request: request);

// If the fees are acceptable, continue to create the Send Payment
if (prepareResponse.paymentMethod is SendPaymentMethod.SparkInvoice sparkInvoiceMethod)
{
    var fee = sparkInvoiceMethod.fee;
    Console.WriteLine($"Fees: {fee} sats");
}
```

##### Javascript (Wasm)

```typescript
const paymentRequest = '<spark invoice>'
// Optionally set the amount you wish to pay the receiver
const optionalAmountSats = BigInt(50_000)

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: { type: 'input', input: paymentRequest },
  amount: optionalAmountSats,
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy: undefined
})

// If the fees are acceptable, continue to create the Send Payment
if (prepareResponse.paymentMethod.type === 'sparkInvoice') {
  const feeSats = prepareResponse.paymentMethod.fee
  console.debug(`Fees: ${feeSats} sats`)
}
```

##### React Native

```typescript
const paymentRequest = '<spark invoice>'
// Optionally set the amount you wish to pay the receiver
const optionalAmountSats = BigInt(50_000)

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: new PaymentRequest.Input({ input: paymentRequest }),
  amount: optionalAmountSats,
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy: undefined
})

// If the fees are acceptable, continue to create the Send Payment
if (prepareResponse.paymentMethod?.tag === SendPaymentMethod_Tags.SparkInvoice) {
  const feeSats = prepareResponse.paymentMethod.inner.fee
  console.debug(`Fees: ${feeSats} sats`)
}
```

##### Flutter

```dart
String paymentRequest = "<spark invoice>";
// Optionally set the amount you wish to pay the receiver
BigInt? optionalAmountSats = BigInt.from(50000);

final request = PrepareSendPaymentRequest(
    paymentRequest: PaymentRequest.input(input: paymentRequest),
    amount: optionalAmountSats,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: null);
final response = await sdk.prepareSendPayment(request: request);

// If the fees are acceptable, continue to create the Send Payment
final paymentMethod = response.paymentMethod;
if (paymentMethod is SendPaymentMethod_SparkInvoice) {
  final feeSats = paymentMethod.fee;
  print("Fees: $feeSats sats");
}
```

##### Python

```python
payment_request = "<spark invoice>"
optional_amount_sats = 50_000
try:
    request = PrepareSendPaymentRequest(
        payment_request=PaymentRequest.INPUT(input=payment_request),
        amount=optional_amount_sats,
        token_identifier=None,
        conversion_options=None,
        fee_policy=None,
    )
    prepare_response = await sdk.prepare_send_payment(request=request)

    # If the fees are acceptable, continue to create the Send Payment
    if isinstance(prepare_response.payment_method, SendPaymentMethod.SPARK_INVOICE):
        fee = prepare_response.payment_method.fee
        logging.debug(f"Fees: {fee} sats")
except Exception as error:
    logging.error(error)
    raise
```

##### Go

```go
paymentRequest := "<spark invoice>"
// Optionally set the amount you wish to pay the receiver
optionalAmountSats := new(big.Int).SetInt64(50_000)

request := breez_sdk_spark.PrepareSendPaymentRequest{
	PaymentRequest:    breez_sdk_spark.PaymentRequestInput{Input: paymentRequest},
	Amount:            &optionalAmountSats,
	TokenIdentifier:   nil,
	ConversionOptions: nil,
	FeePolicy:         nil,
}
response, err := sdk.PrepareSendPayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

// If the fees are acceptable, continue to create the Send Payment
switch paymentMethod := response.PaymentMethod.(type) {
case breez_sdk_spark.SendPaymentMethodSparkInvoice:
	feeSats := paymentMethod.Fee
	log.Printf("Fees: %v sats", feeSats)
}
```



### USDC/USDT

Send USDC or USDT from a Spark wallet to a recipient on one of several supported chains: Ethereum-family chains (Arbitrum, Base, and similar EVM networks), Solana, and Tron. The source on the Spark side is BTC sats or USDB. This feature must be enabled in [the SDK configuration](./config.md#send-usdc-usdt) before using. See [Send USDC/USDT](./cross_chain.md) for provider details and the status lifecycle.

After [parsing](./parse.md) the recipient address into `InputType::CrossChainAddress`, call `get_cross_chain_routes` with `CrossChainRouteFilter::Send` carrying the parsed `CrossChainAddressDetails`. The returned `CrossChainRoutePair`s name the provider, destination chain and asset, decimals, optional token contract address, and which source assets (BTC sats or USDB) each route accepts.

#### Rust

```rust
let input = "<recipient address>";
let InputType::CrossChainAddress(address_details) = sdk.parse(input).await? else {
    anyhow::bail!("Not a cross-chain address");
};

let routes = sdk
    .get_cross_chain_routes(&CrossChainRouteFilter::Send {
        address_details: address_details.clone(),
    })
    .await?;

for route in &routes {
    info!(
        "Route via {:?}: {}/{}",
        route.provider, route.chain, route.asset
    );
}
```

#### Swift

```swift
let input = "<recipient address>"
let parsed = try await sdk.parse(input: input)
guard case let .crossChainAddress(v1: addressDetails) = parsed else {
    throw NSError(domain: "CrossChain", code: 1)
}

let routes = try await sdk.getCrossChainRoutes(
    filter: .send(addressDetails: addressDetails))

for route in routes {
    print("Route via \(route.provider): \(route.chain)/\(route.asset)")
}
```

#### Kotlin

```kotlin
val input = "<recipient address>"

try {
    val parsed = sdk.parse(input)
    if (parsed !is InputType.CrossChainAddress) {
        throw IllegalArgumentException("Not a cross-chain address")
    }
    val addressDetails = parsed.v1

    val routes = sdk.getCrossChainRoutes(
        CrossChainRouteFilter.Send(addressDetails = addressDetails)
    )

    for (route in routes) {
        // Log.v("Breez", "Route via ${route.provider}: ${route.chain}/${route.asset}")
    }
} catch (e: Exception) {
    // handle error
}
```

#### C#

```csharp
var inputStr = "<recipient address>";
var parsed = await sdk.Parse(input: inputStr);
if (parsed is not InputType.CrossChainAddress crossChain)
{
    throw new InvalidOperationException("Not a cross-chain address");
}
var addressDetails = crossChain.v1;

var filter = new CrossChainRouteFilter.Send(addressDetails: addressDetails);
var routes = await sdk.GetCrossChainRoutes(filter: filter);

foreach (var route in routes)
{
    Console.WriteLine($"Route via {route.provider}: {route.chain}/{route.asset}");
}
```

#### Javascript (Wasm)

```typescript
const input = '<recipient address>'
const parsed = await sdk.parse(input)
if (parsed.type !== 'crossChainAddress') {
  throw new Error('Not a cross-chain address')
}

const routes = await sdk.getCrossChainRoutes({
  type: 'send',
  addressDetails: parsed
})

for (const route of routes) {
  console.debug(`Route via ${route.provider}: ${route.chain}/${route.asset}`)
}
```

#### React Native

```typescript
const input = '<recipient address>'
const parsed = await sdk.parse(input)
if (parsed.tag !== InputType_Tags.CrossChainAddress) {
  throw new Error('Not a cross-chain address')
}
const addressDetails = parsed.inner[0]

const routes = await sdk.getCrossChainRoutes(
  new CrossChainRouteFilter.Send({ addressDetails })
)

for (const route of routes) {
  console.debug(`Route via ${route.provider}: ${route.chain}/${route.asset}`)
}
```

#### Flutter

```dart
String input = "<recipient address>";
InputType parsed = await sdk.parse(input: input);
if (parsed is! InputType_CrossChainAddress) {
  throw Exception("Not a cross-chain address");
}
CrossChainAddressDetails addressDetails = parsed.field0;

List<CrossChainRoutePair> routes = await sdk.getCrossChainRoutes(
  filter: CrossChainRouteFilter.send(addressDetails: addressDetails),
);

for (var route in routes) {
  print("Route via ${route.provider}: ${route.chain}/${route.asset}");
}
```

#### Python

```python
input_str = "<recipient address>"
try:
    parsed = await sdk.parse(input=input_str)
    if not isinstance(parsed, InputType.CROSS_CHAIN_ADDRESS):
        raise ValueError("Not a cross-chain address")
    address_details = parsed[0]

    routes = await sdk.get_cross_chain_routes(
        filter=CrossChainRouteFilter.SEND(address_details=address_details)
    )

    for route in routes:
        logging.debug(
            f"Route via {route.provider}: {route.chain}/{route.asset}"
        )
except Exception as error:
    logging.error(error)
    raise
```

#### Go

```go
inputStr := "<recipient address>"
input, err := sdk.Parse(inputStr)
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError
	}
	return nil, err
}

addressInput, ok := input.(breez_sdk_spark.InputTypeCrossChainAddress)
if !ok {
	return nil, errors.New("not a cross-chain address")
}
addressDetails := addressInput.Field0

filter := breez_sdk_spark.CrossChainRouteFilterSend{AddressDetails: addressDetails}
routes, err := sdk.GetCrossChainRoutes(filter)
if err != nil {
	return nil, err
}

for _, route := range routes {
	log.Printf("Route via %v: %s/%s", route.Provider, route.Chain, route.Asset)
}
```



Build `PaymentRequest::CrossChain` with the recipient address, the chosen route, and an optional `max_slippage_bps` (10 to 500 basis points). The amount on the prepare request is denominated in the source asset's base units: sats for a BTC source, USDB base units for a USDB source.

The prepare response carries a quote `expires_at` timestamp. Re-prepare and pick a fresh route if it lapses before send.

#### Rust

```rust
// Optionally set the maximum slippage in basis points (10 to 500)
let optional_max_slippage_bps = Some(100);

let prepare_response = sdk
    .prepare_send_payment(PrepareSendPaymentRequest {
        payment_request: PaymentRequest::CrossChain {
            address: address_details.address.clone(),
            route,
            max_slippage_bps: optional_max_slippage_bps,
            target_overpay_bps: None,
        },
        amount: Some(50_000),
        token_identifier: None,
        conversion_options: None,
        fee_policy: None,
    })
    .await?;

if let SendPaymentMethod::CrossChainAddress {
    amount_in,
    estimated_out,
    fee_amount,
    expires_at,
    ..
} = &prepare_response.payment_method
{
    info!("Amount in: {amount_in}");
    info!("Estimated out: {estimated_out}");
    info!("Provider fee: {fee_amount}");
    info!("Quote expires at: {expires_at}");
}
```

#### Swift

```swift
// Optionally set the maximum slippage in basis points (10 to 500)
let optionalMaxSlippageBps: UInt32? = 100

let prepareResponse = try await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
        paymentRequest: .crossChain(
            address: addressDetails.address,
            route: route,
            maxSlippageBps: optionalMaxSlippageBps,
            targetOverpayBps: nil
        ),
        amount: BInt(50_000),
        tokenIdentifier: nil,
        conversionOptions: nil,
        feePolicy: nil
    ))

if case let .crossChainAddress(
    _, _, amountIn, _, estimatedOut, feeAmount, _, _, _, _, expiresAt, _
) = prepareResponse.paymentMethod {
    print("Amount in: \(amountIn)")
    print("Estimated out: \(estimatedOut)")
    print("Provider fee: \(feeAmount)")
    print("Quote expires at: \(expiresAt)")
}
```

#### Kotlin

```kotlin
// Optionally set the maximum slippage in basis points (10 to 500)
val optionalMaxSlippageBps: UInt? = 100u

try {
    val req = PrepareSendPaymentRequest(
        paymentRequest = PaymentRequest.CrossChain(
            address = addressDetails.address,
            route = route,
            maxSlippageBps = optionalMaxSlippageBps,
            targetOverpayBps = null,
        ),
        amount = BigInteger.fromLong(50_000L),
        tokenIdentifier = null,
        conversionOptions = null,
        feePolicy = null,
    )
    val prepareResponse = sdk.prepareSendPayment(req)

    val paymentMethod = prepareResponse.paymentMethod
    if (paymentMethod is SendPaymentMethod.CrossChainAddress) {
        val amountIn = paymentMethod.amountIn
        val estimatedOut = paymentMethod.estimatedOut
        val feeAmount = paymentMethod.feeAmount
        val expiresAt = paymentMethod.expiresAt
        // Log.v("Breez", "Amount in: $amountIn")
        // Log.v("Breez", "Estimated out: $estimatedOut")
        // Log.v("Breez", "Provider fee: $feeAmount")
        // Log.v("Breez", "Quote expires at: $expiresAt")
    }
} catch (e: Exception) {
    // handle error
}
```

#### C#

```csharp
// Optionally set the maximum slippage in basis points (10 to 500)
uint? optionalMaxSlippageBps = 100;

var request = new PrepareSendPaymentRequest(
    paymentRequest: new PaymentRequest.CrossChain(
        address: addressDetails.address,
        route: route,
        maxSlippageBps: optionalMaxSlippageBps,
        targetOverpayBps: null
    ),
    amount: 50_000UL,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: null
);
var prepareResponse = await sdk.PrepareSendPayment(request: request);

if (prepareResponse.paymentMethod is SendPaymentMethod.CrossChainAddress method)
{
    Console.WriteLine($"Amount in: {method.amountIn}");
    Console.WriteLine($"Estimated out: {method.estimatedOut}");
    Console.WriteLine($"Provider fee: {method.feeAmount}");
    Console.WriteLine($"Quote expires at: {method.expiresAt}");
}
```

#### Javascript (Wasm)

```typescript
// Optionally set the maximum slippage in basis points (10 to 500)
const optionalMaxSlippageBps = 100

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: {
    type: 'crossChain',
    address: addressDetails.address,
    route,
    maxSlippageBps: optionalMaxSlippageBps
  },
  amount: BigInt(50_000),
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy: undefined
})

if (prepareResponse.paymentMethod.type === 'crossChainAddress') {
  const { amountIn, estimatedOut, feeAmount, expiresAt } = prepareResponse.paymentMethod
  console.debug(`Amount in: ${amountIn}`)
  console.debug(`Estimated out: ${estimatedOut}`)
  console.debug(`Provider fee: ${feeAmount}`)
  console.debug(`Quote expires at: ${expiresAt}`)
}
```

#### React Native

```typescript
// Optionally set the maximum slippage in basis points (10 to 500)
const optionalMaxSlippageBps = 100

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: new PaymentRequest.CrossChain({
    address: addressDetails.address,
    route,
    maxSlippageBps: optionalMaxSlippageBps,
    targetOverpayBps: undefined
  }),
  amount: BigInt(50_000),
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy: undefined
})

if (prepareResponse.paymentMethod?.tag === SendPaymentMethod_Tags.CrossChainAddress) {
  const inner = prepareResponse.paymentMethod.inner
  console.debug(`Amount in: ${inner.amountIn}`)
  console.debug(`Estimated out: ${inner.estimatedOut}`)
  console.debug(`Provider fee: ${inner.feeAmount}`)
  console.debug(`Quote expires at: ${inner.expiresAt}`)
}
```

#### Flutter

```dart
// Optionally set the maximum slippage in basis points (10 to 500)
int? optionalMaxSlippageBps = 100;

final request = PrepareSendPaymentRequest(
  paymentRequest: PaymentRequest.crossChain(
    address: addressDetails.address,
    route: route,
    maxSlippageBps: optionalMaxSlippageBps,
    targetOverpayBps: null,
  ),
  amount: BigInt.from(50000),
  tokenIdentifier: null,
  conversionOptions: null,
  feePolicy: null,
);
final response = await sdk.prepareSendPayment(request: request);

final paymentMethod = response.paymentMethod;
if (paymentMethod is SendPaymentMethod_CrossChainAddress) {
  print("Amount in: ${paymentMethod.amountIn}");
  print("Estimated out: ${paymentMethod.estimatedOut}");
  print("Provider fee: ${paymentMethod.feeAmount}");
  print("Quote expires at: ${paymentMethod.expiresAt}");
}
```

#### Python

```python
# Optionally set the maximum slippage in basis points (10 to 500)
optional_max_slippage_bps = 100
try:
    request = PrepareSendPaymentRequest(
        payment_request=PaymentRequest.CROSS_CHAIN(
            address=address_details.address,
            route=route,
            max_slippage_bps=optional_max_slippage_bps,
            target_overpay_bps=None,
        ),
        amount=50_000,
        token_identifier=None,
        conversion_options=None,
        fee_policy=None,
    )
    prepare_response = await sdk.prepare_send_payment(request=request)

    if isinstance(
        prepare_response.payment_method, SendPaymentMethod.CROSS_CHAIN_ADDRESS
    ):
        method = prepare_response.payment_method
        logging.debug(f"Amount in: {method.amount_in}")
        logging.debug(f"Estimated out: {method.estimated_out}")
        logging.debug(f"Provider fee: {method.fee_amount}")
        logging.debug(f"Quote expires at: {method.expires_at}")
except Exception as error:
    logging.error(error)
    raise
```

#### Go

```go
// Optionally set the maximum slippage in basis points (10 to 500)
optionalMaxSlippageBps := uint32(100)
amount := new(big.Int).SetInt64(50_000)

request := breez_sdk_spark.PrepareSendPaymentRequest{
	PaymentRequest: breez_sdk_spark.PaymentRequestCrossChain{
		Address:           addressDetails.Address,
		Route:             route,
		MaxSlippageBps:    &optionalMaxSlippageBps,
		TargetOverpayBps:  nil,
	},
	Amount:            &amount,
	TokenIdentifier:   nil,
	ConversionOptions: nil,
	FeePolicy:         nil,
}
response, err := sdk.PrepareSendPayment(request)
if err != nil {
	return nil, err
}

switch paymentMethod := response.PaymentMethod.(type) {
case breez_sdk_spark.SendPaymentMethodCrossChainAddress:
	log.Printf("Amount in: %v", paymentMethod.AmountIn)
	log.Printf("Estimated out: %v", paymentMethod.EstimatedOut)
	log.Printf("Provider fee: %v", paymentMethod.FeeAmount)
	log.Printf("Quote expires at: %s", paymentMethod.ExpiresAt)
}
```



## Fee Policy

By default, fees are added on top of the amount (`FeePolicy::FeesExcluded`). Use `FeePolicy::FeesIncluded` to deduct fees from the amount instead—the receiver gets the amount minus fees.

This is particularly useful when you want to spend your entire balance in a single payment—simply provide your full balance as the amount. Note: `FeePolicy::FeesIncluded` is not compatible with payment requests that specify an amount (e.g., BOLT11 invoices and Spark invoices with amount).

### Rust

```rust
// By default (FeePolicy::FeesExcluded), fees are added on top of the amount.
// Use FeePolicy::FeesIncluded to deduct fees from the amount instead.
// The receiver gets amount minus fees.
let payment_request = "<payment request>".to_string();
let amount_sats = Some(50_000);

let prepare_response = sdk
    .prepare_send_payment(PrepareSendPaymentRequest {
        payment_request: PaymentRequest::Input {
            input: payment_request,
        },
        amount: amount_sats,
        token_identifier: None,
        conversion_options: None,
        fee_policy: Some(FeePolicy::FeesIncluded),
    })
    .await?;

// The response shows the fee policy used
info!("Fee policy: {:?}", prepare_response.fee_policy);
info!("Amount: {}", prepare_response.amount);
// The receiver gets amount - fees (fees are available in prepare_response.payment_method)
```

### Swift

```swift
// By default (.feesExcluded), fees are added on top of the amount.
// Use .feesIncluded to deduct fees from the amount instead.
// The receiver gets amount minus fees.
let paymentRequest = "<payment request>"
let amountSats: BInt? = BInt(50_000)

let prepareResponse = try await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
        paymentRequest: .input(input: paymentRequest),
        amount: amountSats,
        tokenIdentifier: nil,
        conversionOptions: nil,
        feePolicy: .feesIncluded
    ))

// The response shows the fee policy used
print("Fee policy: \(String(describing: prepareResponse.feePolicy))")
print("Amount: \(String(describing: prepareResponse.amount))")
// The receiver gets amount - fees (fees are available in prepareResponse.paymentMethod)
```

### Kotlin

```kotlin
// By default (FeePolicy.FEES_EXCLUDED), fees are added on top of the amount.
// Use FeePolicy.FEES_INCLUDED to deduct fees from the amount instead.
// The receiver gets amount minus fees.
val paymentRequest = "<payment request>"
// Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer)
val amountSats = BigInteger.fromLong(50_000L)
// Android (BigInteger from java.math)
// val amountSats = BigInteger.valueOf(50_000L)

try {
    val req = PrepareSendPaymentRequest(
        paymentRequest = PaymentRequest.Input(input = paymentRequest),
        amount = amountSats,
        tokenIdentifier = null,
        conversionOptions = null,
        feePolicy = FeePolicy.FEES_INCLUDED,
    )
    val prepareResponse = sdk.prepareSendPayment(req)

    // The response shows the fee policy used
    // Log.v("Breez", "Fee policy: ${prepareResponse.feePolicy}")
    // Log.v("Breez", "Amount: ${prepareResponse.amount}")
    // The receiver gets amount - fees (fees are available in prepareResponse.paymentMethod)
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
// By default (FeePolicy.FeesExcluded), fees are added on top of the amount.
// Use FeePolicy.FeesIncluded to deduct fees from the amount instead.
// The receiver gets amount minus fees.
var paymentRequest = "<payment request>";
ulong? amountSats = 50_000UL;

var request = new PrepareSendPaymentRequest(
    paymentRequest: new PaymentRequest.Input(input: paymentRequest),
    amount: amountSats,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: FeePolicy.FeesIncluded
);
var prepareResponse = await sdk.PrepareSendPayment(request: request);

// The response shows the fee policy used
Console.WriteLine($"Fee policy: {prepareResponse.feePolicy}");
Console.WriteLine($"Amount: {prepareResponse.amount}");
// The receiver gets amount - fees (fees are available in prepareResponse.paymentMethod)
```

### Javascript (Wasm)

```typescript
// By default ({ type: 'feesExcluded' }), fees are added on top of the amount.
// Use { type: 'feesIncluded' } to deduct fees from the amount instead.
// The receiver gets amount minus fees.
const paymentRequest = '<payment request>'
const amountSats = BigInt(50_000)
const feePolicy: FeePolicy = 'feesIncluded'

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: { type: 'input', input: paymentRequest },
  amount: amountSats,
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy
})

// The response shows the fee policy used
console.log(`Fee policy: ${JSON.stringify(prepareResponse.feePolicy)}`)
console.log(`Amount: ${prepareResponse.amount}`)
// The receiver gets amount - fees (fees are available in prepareResponse.paymentMethod)
```

### React Native

```typescript
// By default (FeePolicy.FeesExcluded), fees are added on top of the amount.
// Use FeePolicy.FeesIncluded to deduct fees from the amount instead.
// The receiver gets amount minus fees.
const paymentRequest = '<payment request>'
const amountSats = BigInt(50_000)

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: new PaymentRequest.Input({ input: paymentRequest }),
  amount: amountSats,
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy: FeePolicy.FeesIncluded
})

// The response shows the fee policy used
console.log(`Fee policy: ${JSON.stringify(prepareResponse.feePolicy)}`)
console.log(`Amount: ${prepareResponse.amount}`)
// The receiver gets amount - fees (fees are available in prepareResponse.paymentMethod)
```

### Flutter

```dart
// By default (FeePolicy.feesExcluded), fees are added on top of the amount.
// Use FeePolicy.feesIncluded to deduct fees from the amount instead.
// The receiver gets amount minus fees.
String paymentRequest = "<payment request>";
BigInt? amountSats = BigInt.from(50000);

final request = PrepareSendPaymentRequest(
    paymentRequest: PaymentRequest.input(input: paymentRequest),
    amount: amountSats,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: FeePolicy.feesIncluded);
final response = await sdk.prepareSendPayment(request: request);

// The response shows the fee policy used
print("Fee policy: ${response.feePolicy}");
print("Amount: ${response.amount}");
// The receiver gets amount - fees (fees are available in response.paymentMethod)
```

### Python

```python
# By default (FeePolicy.FEES_EXCLUDED), fees are added on top of the amount.
# Use FeePolicy.FEES_INCLUDED to deduct fees from the amount instead.
# The receiver gets amount minus fees.
payment_request = "<payment request>"
amount_sats = 50_000
try:
    request = PrepareSendPaymentRequest(
        payment_request=PaymentRequest.INPUT(input=payment_request),
        amount=amount_sats,
        token_identifier=None,
        conversion_options=None,
        fee_policy=FeePolicy.FEES_INCLUDED,
    )
    prepare_response = await sdk.prepare_send_payment(request=request)

    # The response shows the fee policy used
    logging.debug(f"Fee policy: {prepare_response.fee_policy}")
    logging.debug(f"Amount: {prepare_response.amount}")
    # The receiver gets amount - fees (fees are available in prepare_response.payment_method)
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
// By default (FeePolicyFeesExcluded), fees are added on top of the amount.
// Use FeePolicyFeesIncluded to deduct fees from the amount instead.
// The receiver gets amount minus fees.
paymentRequest := "<payment request>"
amountSats := new(big.Int).SetInt64(50_000)
feePolicy := breez_sdk_spark.FeePolicyFeesIncluded

request := breez_sdk_spark.PrepareSendPaymentRequest{
	PaymentRequest:    breez_sdk_spark.PaymentRequestInput{Input: paymentRequest},
	Amount:            &amountSats,
	TokenIdentifier:   nil,
	ConversionOptions: nil,
	FeePolicy:         &feePolicy,
}
response, err := sdk.PrepareSendPayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

// The response shows the fee policy used
log.Printf("Fee policy: %v", response.FeePolicy)
log.Printf("Amount: %v", response.Amount)
// The receiver gets amount - fees (fees are available in response.PaymentMethod)
```



When [stable balance](./stable_balance.md) is active, you can send your entire wallet balance — both the token balance and any remaining sats — by combining `FeePolicy::FeesIncluded` with `ConversionType::ToBitcoin` conversion options. See [Sending entire balance](./stable_balance.md#sending-entire-balance) for details.

### Rust

```rust
let payment_request = "<payment request>".to_string();
let token_identifier = "<token identifier>".to_string();

let info = sdk
    .get_info(GetInfoRequest {
        ensure_synced: Some(false),
    })
    .await?;

let token_balance = info
    .token_balances
    .get(&token_identifier)
    .ok_or_else(|| anyhow::anyhow!("Token balance not found"))?;

let conversion_options = Some(ConversionOptions {
    conversion_type: ConversionType::ToBitcoin {
        from_token_identifier: token_identifier.clone(),
    },
    max_slippage_bps: None,
    completion_timeout_secs: None,
});

let prepare_response = sdk
    .prepare_send_payment(PrepareSendPaymentRequest {
        payment_request: PaymentRequest::Input {
            input: payment_request,
        },
        amount: Some(token_balance.balance),
        token_identifier: Some(token_identifier),
        conversion_options,
        fee_policy: Some(FeePolicy::FeesIncluded),
    })
    .await?;

// The response amount is the estimated total sats available
// (converted sats + existing sat balance)
info!("Total sats available: {}", prepare_response.amount);

if let Some(conversion_estimate) = &prepare_response.conversion_estimate {
    info!(
        "Converting {} token units → ~{} sats",
        conversion_estimate.amount_in, conversion_estimate.amount_out
    );
    info!("Conversion fee: {} token units", conversion_estimate.fee);
}
```

### Swift

```swift
let paymentRequest = "<payment request>"
let tokenIdentifier = "<token identifier>"

let info = try await sdk.getInfo(
    request: GetInfoRequest(ensureSynced: false))

guard let tokenBalance = info.tokenBalances[tokenIdentifier] else {
    throw SdkError.InvalidInput("Token balance not found")
}

let conversionOptions = ConversionOptions(
    conversionType: ConversionType.toBitcoin(
        fromTokenIdentifier: tokenIdentifier
    ),
    maxSlippageBps: nil,
    completionTimeoutSecs: nil
)

let prepareResponse = try await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
        paymentRequest: .input(input: paymentRequest),
        amount: tokenBalance.balance,
        tokenIdentifier: tokenIdentifier,
        conversionOptions: conversionOptions,
        feePolicy: .feesIncluded
    ))

// The response amount is the estimated total sats available
// (converted sats + existing sat balance)
print("Total sats available: \(prepareResponse.amount)")

if let conversionEstimate = prepareResponse.conversionEstimate {
    print(
        "Converting \(conversionEstimate.amountIn) token units "
            + "→ ~\(conversionEstimate.amountOut) sats")
    print("Conversion fee: \(conversionEstimate.fee) token units")
}
```

### Kotlin

```kotlin
val paymentRequest = "<payment request>"
val tokenIdentifier = "<token identifier>"

try {
    val info = sdk.getInfo(GetInfoRequest(false))
    val tokenBalance = info.tokenBalances[tokenIdentifier]
        ?: throw Exception("Token balance not found")

    val conversionOptions = ConversionOptions(
        conversionType = ConversionType.ToBitcoin(
            tokenIdentifier
        ),
        maxSlippageBps = null,
        completionTimeoutSecs = null
    )

    val req = PrepareSendPaymentRequest(
        paymentRequest = PaymentRequest.Input(input = paymentRequest),
        amount = tokenBalance.balance,
        tokenIdentifier = tokenIdentifier,
        conversionOptions = conversionOptions,
        feePolicy = FeePolicy.FEES_INCLUDED,
    )
    val prepareResponse = sdk.prepareSendPayment(req)

    // The response amount is the estimated total sats available
    // (converted sats + existing sat balance)
    // Log.v("Breez", "Total sats available: ${prepareResponse.amount}")

    prepareResponse.conversionEstimate?.let { conversionEstimate ->
        // Log.v("Breez", "Converting ${conversionEstimate.amountIn} token
        // units → ~${conversionEstimate.amountOut} sats")
        // Log.v("Breez", "Conversion fee: ${conversionEstimate.fee} token units")
    }
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
var paymentRequest = "<payment request>";
var tokenIdentifier = "<token identifier>";

var info = await sdk.GetInfo(request: new GetInfoRequest(ensureSynced: false));
if (!info.tokenBalances.TryGetValue(tokenIdentifier, out var tokenBalance))
{
    throw new Exception("Token balance not found");
}

var conversionOptions = new ConversionOptions(
    conversionType: new ConversionType.ToBitcoin(
        fromTokenIdentifier: tokenIdentifier
    ),
    maxSlippageBps: null,
    completionTimeoutSecs: null
);

var request = new PrepareSendPaymentRequest(
    paymentRequest: new PaymentRequest.Input(input: paymentRequest),
    amount: tokenBalance.balance,
    tokenIdentifier: tokenIdentifier,
    conversionOptions: conversionOptions,
    feePolicy: FeePolicy.FeesIncluded
);
var prepareResponse = await sdk.PrepareSendPayment(request: request);

// The response amount is the estimated total sats available
// (converted sats + existing sat balance)
Console.WriteLine($"Total sats available: {prepareResponse.amount}");

if (prepareResponse.conversionEstimate != null)
{
    Console.WriteLine("Converting " +
        $"{prepareResponse.conversionEstimate.amountIn} token units " +
        $"→ ~{prepareResponse.conversionEstimate.amountOut} sats");
    Console.WriteLine("Conversion fee: " +
        $"{prepareResponse.conversionEstimate.fee} token units");
}
```

### Javascript (Wasm)

```typescript
const paymentRequest = '<payment request>'
const tokenIdentifier = '<token identifier>'

const info = await sdk.getInfo({ ensureSynced: false })
const tokenBalance = info.tokenBalances.get(tokenIdentifier)
if (tokenBalance === undefined) {
  throw new Error('Token balance not found')
}

const conversionOptions: ConversionOptions = {
  conversionType: {
    type: 'toBitcoin',
    fromTokenIdentifier: tokenIdentifier
  }
}
const feePolicy: FeePolicy = 'feesIncluded'

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: { type: 'input', input: paymentRequest },
  amount: tokenBalance.balance,
  tokenIdentifier,
  conversionOptions,
  feePolicy
})

// The response amount is the estimated total sats available
// (converted sats + existing sat balance)
console.log(`Total sats available: ${prepareResponse.amount}`)

if (prepareResponse.conversionEstimate !== undefined) {
  const estimate = prepareResponse.conversionEstimate
  console.log(`Converting ${estimate.amountIn} token units → ~${estimate.amountOut} sats`)
  console.log(`Conversion fee: ${estimate.fee} token units`)
}
```

### React Native

```typescript
const paymentRequest = '<payment request>'
const tokenIdentifier = '<token identifier>'

const info = await sdk.getInfo({ ensureSynced: false })
const tokenBalance = info.tokenBalances.get(tokenIdentifier)
if (tokenBalance === undefined) {
  throw new Error('Token balance not found')
}

const conversionOptions = {
  conversionType: new ConversionType.ToBitcoin({
    fromTokenIdentifier: tokenIdentifier
  }),
  maxSlippageBps: undefined,
  completionTimeoutSecs: undefined
}

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: new PaymentRequest.Input({ input: paymentRequest }),
  amount: tokenBalance.balance,
  tokenIdentifier,
  conversionOptions,
  feePolicy: FeePolicy.FeesIncluded
})

// The response amount is the estimated total sats available
// (converted sats + existing sat balance)
console.log(`Total sats available: ${prepareResponse.amount}`)

if (prepareResponse.conversionEstimate !== undefined) {
  const estimate = prepareResponse.conversionEstimate
  console.log(`Converting ${estimate.amountIn} token units → ~${estimate.amountOut} sats`)
  console.log(`Conversion fee: ${estimate.fee} token units`)
}
```

### Flutter

```dart
String paymentRequest = "<payment request>";
String tokenIdentifier = "<token identifier>";

final info = await sdk.getInfo(request: GetInfoRequest(ensureSynced: false));
final tokenBalance = info.tokenBalances[tokenIdentifier];
if (tokenBalance == null) {
  throw Exception("Token balance not found");
}

final conversionOptions = ConversionOptions(
  conversionType: ConversionType.toBitcoin(
    fromTokenIdentifier: tokenIdentifier,
  ),
);

final request = PrepareSendPaymentRequest(
    paymentRequest: PaymentRequest.input(input: paymentRequest),
    amount: tokenBalance.balance,
    tokenIdentifier: tokenIdentifier,
    conversionOptions: conversionOptions,
    feePolicy: FeePolicy.feesIncluded);
final response = await sdk.prepareSendPayment(request: request);

// The response amount is the estimated total sats available
// (converted sats + existing sat balance)
print("Total sats available: ${response.amount}");

if (response.conversionEstimate != null) {
  print(
      "Converting ${response.conversionEstimate!.amountIn} token units "
      "→ ~${response.conversionEstimate!.amountOut} sats");
  print(
      "Conversion fee: ${response.conversionEstimate!.fee} token units");
}
```

### Python

```python
payment_request = "<payment request>"
token_identifier = "<token identifier>"
try:
    info = await sdk.get_info(request=GetInfoRequest(ensure_synced=False))
    token_balance = info.token_balances.get(token_identifier)
    if token_balance is None:
        raise ValueError("Token balance not found")

    conversion_options = ConversionOptions(
        conversion_type=ConversionType.TO_BITCOIN(
            from_token_identifier=token_identifier
        ),
    )

    request = PrepareSendPaymentRequest(
        payment_request=PaymentRequest.INPUT(input=payment_request),
        amount=token_balance.balance,
        token_identifier=token_identifier,
        conversion_options=conversion_options,
        fee_policy=FeePolicy.FEES_INCLUDED,
    )
    prepare_response = await sdk.prepare_send_payment(request=request)

    # The response amount is the estimated total sats available
    # (converted sats + existing sat balance)
    logging.debug(f"Total sats available: {prepare_response.amount}")

    if prepare_response.conversion_estimate is not None:
        conversion_estimate = prepare_response.conversion_estimate
        logging.debug(
            f"Converting {conversion_estimate.amount_in}"
            f" token units → ~{conversion_estimate.amount_out} sats"
        )
        logging.debug(
            f"Conversion fee: {conversion_estimate.fee} token units"
        )
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
paymentRequest := "<payment request>"
tokenIdentifier := "<token identifier>"

ensureSynced := false
info, err := sdk.GetInfo(breez_sdk_spark.GetInfoRequest{
	EnsureSynced: &ensureSynced,
})
if err != nil {
	return nil, err
}

tokenBalance, ok := info.TokenBalances[tokenIdentifier]
if !ok {
	return nil, errors.New("token balance not found")
}

conversionOptions := breez_sdk_spark.ConversionOptions{
	ConversionType: breez_sdk_spark.ConversionTypeToBitcoin{
		FromTokenIdentifier: tokenIdentifier,
	},
}
feePolicy := breez_sdk_spark.FeePolicyFeesIncluded

request := breez_sdk_spark.PrepareSendPaymentRequest{
	PaymentRequest:    breez_sdk_spark.PaymentRequestInput{Input: paymentRequest},
	Amount:            &tokenBalance.Balance,
	TokenIdentifier:   &tokenIdentifier,
	ConversionOptions: &conversionOptions,
	FeePolicy:         &feePolicy,
}
response, err := sdk.PrepareSendPayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

// The response amount is the estimated total sats available
// (converted sats + existing sat balance)
log.Printf("Total sats available: %v", response.Amount)

if response.ConversionEstimate != nil {
	log.Printf(
		"Converting %v token units → ~%v sats",
		response.ConversionEstimate.AmountIn,
		response.ConversionEstimate.AmountOut,
	)
	log.Printf("Conversion fee: %v token units", response.ConversionEstimate.Fee)
}
```



## Sending Payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.send_payment

Once the payment has been prepared and the fees are accepted, the payment can be sent by passing:

- **Prepare Response** - The response from the [Preparing the Payment](send_payment.md#preparing-payments) step.
- **Options** - Any payment method specific options for the payment (see below).
- **Idempotency Key** - An optional UUID that identifies the payment. If set, providing the same idempotency key for multiple requests will ensure that only one payment is made.

### Lightning

In the optional send payment options for BOLT11 invoices, you can set:

- **Prefer Spark** - Set the preference to use Spark to transfer the payment if the invoice contains a Spark address. By default, using Spark transfers are disabled.
- **Completion Timeout** - By default, this function returns immediately. You can override this behavior by specifying a completion timeout in seconds. If the timeout is reached, a pending payment object is returned. If the payment completes within the timeout, the completed payment object is returned.

#### Rust

```rust
let options = Some(SendPaymentOptions::Bolt11Invoice {
    prefer_spark: false,
    completion_timeout_secs: Some(10),
});
let optional_idempotency_key = Some("<idempotency key uuid>".to_string());
let send_response = sdk
    .send_payment(SendPaymentRequest {
        prepare_response,
        options,
        idempotency_key: optional_idempotency_key,
    })
    .await?;
let payment = send_response.payment;
info!("Payment: {payment:?}");
```

#### Swift

```swift
let options = SendPaymentOptions.bolt11Invoice(preferSpark: false, completionTimeoutSecs: 10)
let optionalIdempotencyKey = "<idempotency key uuid>"
let sendResponse = try await sdk.sendPayment(
    request: SendPaymentRequest(
        prepareResponse: prepareResponse,
        options: options,
        idempotencyKey: optionalIdempotencyKey
    ))
let payment = sendResponse.payment
```

#### Kotlin

```kotlin
try {
    val options = SendPaymentOptions.Bolt11Invoice(
        preferSpark = false, 
        completionTimeoutSecs = 10u
    )
    val optionalIdempotencyKey = "<idempotency key uuid>"
    val sendResponse = sdk.sendPayment(
        SendPaymentRequest(
            prepareResponse,
            options,
            optionalIdempotencyKey
        )
    )
    val payment = sendResponse.payment
} catch (e: Exception) {
    // handle error
}
```

#### C#

```csharp
var options = new SendPaymentOptions.Bolt11Invoice(
    preferSpark: false,
    completionTimeoutSecs: 10
);
var optionalIdempotencyKey = "<idempotency key uuid>";
var request = new SendPaymentRequest(
    prepareResponse: prepareResponse,
    options: options,
    idempotencyKey: optionalIdempotencyKey
);
var sendResponse = await sdk.SendPayment(request: request);
var payment = sendResponse.payment;
```

#### Javascript (Wasm)

```typescript
const options: SendPaymentOptions = {
  type: 'bolt11Invoice',
  preferSpark: false,
  completionTimeoutSecs: 10
}
const optionalIdempotencyKey = '<idempotency key uuid>'
const sendResponse = await sdk.sendPayment({
  prepareResponse,
  options,
  idempotencyKey: optionalIdempotencyKey
})
const payment = sendResponse.payment
```

#### React Native

```typescript
const options = new SendPaymentOptions.Bolt11Invoice({
  preferSpark: false,
  completionTimeoutSecs: 10
})
const optionalIdempotencyKey = '<idempotency key uuid>'
const sendResponse = await sdk.sendPayment({
  prepareResponse,
  options,
  idempotencyKey: optionalIdempotencyKey
})
const payment = sendResponse.payment
```

#### Flutter

```dart
final options = SendPaymentOptions.bolt11Invoice(
    preferSpark: false, completionTimeoutSecs: 10);
String? optionalIdempotencyKey = "<idempotency key uuid>";
final request = SendPaymentRequest(
    prepareResponse: prepareResponse,
    options: options,
    idempotencyKey: optionalIdempotencyKey);
SendPaymentResponse response = await sdk.sendPayment(request: request);
Payment payment = response.payment;
```

#### Python

```python
try:
    options = SendPaymentOptions.BOLT11_INVOICE(
        prefer_spark=False, completion_timeout_secs=10
    )
    optional_idempotency_key = "<idempotency key uuid>"
    request = SendPaymentRequest(
        prepare_response=prepare_response,
        options=options,
        idempotency_key=optional_idempotency_key,
    )
    send_response = await sdk.send_payment(request=request)
    payment = send_response.payment
except Exception as error:
    logging.error(error)
    raise
```

#### Go

```go
var completionTimeoutSecs uint32 = 10
var options breez_sdk_spark.SendPaymentOptions = breez_sdk_spark.SendPaymentOptionsBolt11Invoice{
	PreferSpark:           false,
	CompletionTimeoutSecs: &completionTimeoutSecs,
}

optionalIdempotencyKey := "<idempotency key uuid>"
request := breez_sdk_spark.SendPaymentRequest{
	PrepareResponse: prepareResponse,
	Options:         &options,
	IdempotencyKey:  &optionalIdempotencyKey,
}
response, err := sdk.SendPayment(request)

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



### Bitcoin

In the optional send payment options for Bitcoin addresses, you can set:

- **Confirmation Speed** - The priority that the Bitcoin transaction confirms, that also effects the fee paid. By default, it is set to Fast.

#### Rust

```rust
// Select the confirmation speed for the on-chain transaction
let options = Some(SendPaymentOptions::BitcoinAddress {
    confirmation_speed: OnchainConfirmationSpeed::Medium,
});
let optional_idempotency_key = Some("<idempotency key uuid>".to_string());
let send_response = sdk
    .send_payment(SendPaymentRequest {
        prepare_response,
        options,
        idempotency_key: optional_idempotency_key,
    })
    .await?;
let payment = send_response.payment;
info!("Payment: {payment:?}");
```

#### Swift

```swift
// Select the confirmation speed for the on-chain transaction
let options = SendPaymentOptions.bitcoinAddress(
    confirmationSpeed: OnchainConfirmationSpeed.medium
)
let optionalIdempotencyKey = "<idempotency key uuid>"
let sendResponse = try await sdk.sendPayment(
    request: SendPaymentRequest(
        prepareResponse: prepareResponse,
        options: options,
        idempotencyKey: optionalIdempotencyKey
    ))
let payment = sendResponse.payment
```

#### Kotlin

```kotlin
try {
    // Select the confirmation speed for the on-chain transaction
    val options = SendPaymentOptions.BitcoinAddress(
        confirmationSpeed = OnchainConfirmationSpeed.MEDIUM
    )
    val optionalIdempotencyKey = "<idempotency key uuid>"
    val sendResponse = sdk.sendPayment(
        SendPaymentRequest(
            prepareResponse,
            options,
            optionalIdempotencyKey
        )
    )
    val payment = sendResponse.payment
} catch (e: Exception) {
    // handle error
}
```

#### C#

```csharp
// Select the confirmation speed for the on-chain transaction
var options = new SendPaymentOptions.BitcoinAddress(
    confirmationSpeed: OnchainConfirmationSpeed.Medium
);
var optionalIdempotencyKey = "<idempotency key uuid>";
var request = new SendPaymentRequest(
    prepareResponse: prepareResponse,
    options: options,
    idempotencyKey: optionalIdempotencyKey
);
var sendResponse = await sdk.SendPayment(request: request);
var payment = sendResponse.payment;
```

#### Javascript (Wasm)

```typescript
// Select the confirmation speed for the on-chain transaction
const options: SendPaymentOptions = {
  type: 'bitcoinAddress',
  confirmationSpeed: 'medium'
}
const optionalIdempotencyKey = '<idempotency key uuid>'
const sendResponse = await sdk.sendPayment({
  prepareResponse,
  options,
  idempotencyKey: optionalIdempotencyKey
})
const payment = sendResponse.payment
```

#### React Native

```typescript
// Select the confirmation speed for the on-chain transaction
const options = new SendPaymentOptions.BitcoinAddress({
  confirmationSpeed: OnchainConfirmationSpeed.Medium
})
const optionalIdempotencyKey = '<idempotency key uuid>'
const sendResponse = await sdk.sendPayment({
  prepareResponse,
  options,
  idempotencyKey: optionalIdempotencyKey
})
const payment = sendResponse.payment
```

#### Flutter

```dart
// Select the confirmation speed for the on-chain transaction
final options = SendPaymentOptions.bitcoinAddress(
    confirmationSpeed: OnchainConfirmationSpeed.medium);
String? optionalIdempotencyKey = "<idempotency key uuid>";
final request = SendPaymentRequest(
    prepareResponse: prepareResponse,
    options: options,
    idempotencyKey: optionalIdempotencyKey);
SendPaymentResponse response = await sdk.sendPayment(request: request);
Payment payment = response.payment;
```

#### Python

```python
try:
    # Select the confirmation speed for the on-chain transaction
    options = SendPaymentOptions.BITCOIN_ADDRESS(
        confirmation_speed=OnchainConfirmationSpeed.MEDIUM
    )
    optional_idempotency_key = "<idempotency key uuid>"
    request = SendPaymentRequest(
        prepare_response=prepare_response,
        options=options,
        idempotency_key=optional_idempotency_key,
    )
    send_response = await sdk.send_payment(request=request)
    payment = send_response.payment
except Exception as error:
    logging.error(error)
    raise
```

#### Go

```go
// Select the confirmation speed for the on-chain transaction
var options breez_sdk_spark.SendPaymentOptions = breez_sdk_spark.SendPaymentOptionsBitcoinAddress{
	ConfirmationSpeed: breez_sdk_spark.OnchainConfirmationSpeedMedium,
}
optionalIdempotencyKey := "<idempotency key uuid>"
request := breez_sdk_spark.SendPaymentRequest{
	PrepareResponse: prepareResponse,
	Options:         &options,
	IdempotencyKey:  &optionalIdempotencyKey,
}
response, err := sdk.SendPayment(request)

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



### Spark

In the optional send payment options for Spark addresses, you can set:

- **HTLC Options** - Enables Spark HTLC payments, which are an advanced feature that allows for conditional payments. See the [Spark HTLC Payments](htlcs.md) page for more details and example usage.

#### Rust

```rust
let optional_idempotency_key = Some("<idempotency key uuid>".to_string());
let send_response = sdk
    .send_payment(SendPaymentRequest {
        prepare_response,
        options: None,
        idempotency_key: optional_idempotency_key,
    })
    .await?;
let payment = send_response.payment;
info!("Payment: {payment:?}");
```

#### Swift

```swift
let optionalIdempotencyKey = "<idempotency key uuid>"
let sendResponse = try await sdk.sendPayment(
    request: SendPaymentRequest(
        prepareResponse: prepareResponse,
        idempotencyKey: optionalIdempotencyKey
    ))
let payment = sendResponse.payment
```

#### Kotlin

```kotlin
try {
    val optionalIdempotencyKey = "<idempotency key uuid>"
    val sendResponse = sdk.sendPayment(
        SendPaymentRequest(
            prepareResponse,
            idempotencyKey = optionalIdempotencyKey
        )
    )
    val payment = sendResponse.payment
} catch (e: Exception) {
    // handle error
}
```

#### C#

```csharp
var optionalIdempotencyKey = "<idempotency key uuid>";
var request = new SendPaymentRequest(
    prepareResponse: prepareResponse,
    idempotencyKey: optionalIdempotencyKey
);
var sendResponse = await sdk.SendPayment(request: request);
var payment = sendResponse.payment;
```

#### Javascript (Wasm)

```typescript
const optionalIdempotencyKey = '<idempotency key uuid>'
const sendResponse = await sdk.sendPayment({
  prepareResponse,
  idempotencyKey: optionalIdempotencyKey
})
const payment = sendResponse.payment
```

#### React Native

```typescript
const optionalIdempotencyKey = '<idempotency key uuid>'
const sendResponse = await sdk.sendPayment({
  prepareResponse,
  options: undefined,
  idempotencyKey: optionalIdempotencyKey
})
const payment = sendResponse.payment
```

#### Flutter

```dart
String? optionalIdempotencyKey = "<idempotency key uuid>";
final request = SendPaymentRequest(
    prepareResponse: prepareResponse, idempotencyKey: optionalIdempotencyKey);
SendPaymentResponse response = await sdk.sendPayment(request: request);
Payment payment = response.payment;
```

#### Python

```python
try:
    optional_idempotency_key = "<idempotency key uuid>"
    request = SendPaymentRequest(
        prepare_response=prepare_response, idempotency_key=optional_idempotency_key
    )
    send_response = await sdk.send_payment(request=request)
    payment = send_response.payment
except Exception as error:
    logging.error(error)
    raise
```

#### Go

```go
optionalIdempotencyKey := "<idempotency key uuid>"
request := breez_sdk_spark.SendPaymentRequest{
	PrepareResponse: prepareResponse,
	IdempotencyKey:  &optionalIdempotencyKey,
}
response, err := sdk.SendPayment(request)

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



### USDC/USDT

Send USDC/USDT has no additional send payment options.

#### Rust

```rust
// Only valid for sends with no token leg (see Retry safety).
let optional_idempotency_key = Some("<idempotency key uuid>".to_string());
let send_response = sdk
    .send_payment(SendPaymentRequest {
        prepare_response,
        options: None,
        idempotency_key: optional_idempotency_key,
    })
    .await?;
let payment = send_response.payment;
info!("Payment: {payment:?}");
```

#### Swift

```swift
// Only valid for sends with no token leg (see Retry safety).
let optionalIdempotencyKey = "<idempotency key uuid>"
let sendResponse = try await sdk.sendPayment(
    request: SendPaymentRequest(
        prepareResponse: prepareResponse,
        options: nil,
        idempotencyKey: optionalIdempotencyKey
    ))
let payment = sendResponse.payment
print(payment)
```

#### Kotlin

```kotlin
// Only valid for sends with no token leg (see Retry safety).
val optionalIdempotencyKey = "<idempotency key uuid>"
try {
    val req = SendPaymentRequest(
        prepareResponse = prepareResponse,
        options = null,
        idempotencyKey = optionalIdempotencyKey,
    )
    val sendResponse = sdk.sendPayment(req)
    val payment = sendResponse.payment
    // Log.v("Breez", "Payment: $payment")
} catch (e: Exception) {
    // handle error
}
```

#### C#

```csharp
// Only valid for sends with no token leg (see Retry safety).
var optionalIdempotencyKey = "<idempotency key uuid>";
var request = new SendPaymentRequest(
    prepareResponse: prepareResponse,
    options: null,
    idempotencyKey: optionalIdempotencyKey
);
var sendResponse = await sdk.SendPayment(request: request);
Console.WriteLine($"Payment: {sendResponse.payment}");
```

#### Javascript (Wasm)

```typescript
// Only valid for sends with no token leg (see Retry safety).
const optionalIdempotencyKey = '<idempotency key uuid>'
const sendResponse = await sdk.sendPayment({
  prepareResponse,
  options: undefined,
  idempotencyKey: optionalIdempotencyKey
})
const payment = sendResponse.payment
console.debug('Payment:', payment)
```

#### React Native

```typescript
// Only valid for sends with no token leg (see Retry safety).
const optionalIdempotencyKey = '<idempotency key uuid>'
const sendResponse = await sdk.sendPayment({
  prepareResponse,
  options: undefined,
  idempotencyKey: optionalIdempotencyKey
})
console.debug('Payment:', sendResponse.payment)
```

#### Flutter

```dart
// Only valid for sends with no token leg (see Retry safety).
String? optionalIdempotencyKey = "<idempotency key uuid>";
final request = SendPaymentRequest(
  prepareResponse: prepareResponse,
  options: null,
  idempotencyKey: optionalIdempotencyKey,
);
final response = await sdk.sendPayment(request: request);
print("Payment: ${response.payment}");
```

#### Python

```python
# Only valid for sends with no token leg (see Retry safety).
optional_idempotency_key = "<idempotency key uuid>"
try:
    request = SendPaymentRequest(
        prepare_response=prepare_response,
        options=None,
        idempotency_key=optional_idempotency_key,
    )
    send_response = await sdk.send_payment(request=request)
    payment = send_response.payment
    logging.debug(f"Payment: {payment}")
except Exception as error:
    logging.error(error)
    raise
```

#### Go

```go
// Only valid for sends with no token leg (see Retry safety).
optionalIdempotencyKey := "<idempotency key uuid>"
request := breez_sdk_spark.SendPaymentRequest{
	PrepareResponse: prepareResponse,
	Options:         nil,
	IdempotencyKey:  &optionalIdempotencyKey,
}
response, err := sdk.SendPayment(request)
if err != nil {
	return nil, err
}
log.Printf("Payment: %v", response.Payment)
```



## Event Flows

Once a send payment is initiated, you can follow and react to the different payment events using the guide below for each payment method. See [listening to events](/guide/events.html) for how to subscribe to events. 

The `SdkEvent::Synced` event is also emitted as the SDK syncs in the background. See [fetching the balance](/guide/get_info.md) for the recommended pattern for refreshing the balance and payments list.

#### Lightning

| Event                | Description                                                                       | UX Suggestion                                    |
| -------------------- | --------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer has been started. Awaiting Lightning payment completion.       | Show payment as pending.                         |
| **PaymentSucceeded** | The Lightning invoice has been paid either over Lightning or via a Spark transfer | Show the payment as complete and call `get_info` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/guide/get_info.md). |
| **PaymentFailed**    | The attempt to pay the Lightning invoice failed.                                  |                                                  |

#### Bitcoin

| Event                | Description                                                                   | UX Suggestion                                    |
| -------------------- | ----------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer has been started. Awaiting on-chain withdrawal completion. | Show payment as pending.                         |
| **PaymentSucceeded** | The payment amount was successfully withdrawn on-chain.                       | Show the payment as complete and call `get_info` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/guide/get_info.md). |

#### Spark

| Event                | Description                     | UX Suggestion                                    |
| -------------------- | ------------------------------- | ------------------------------------------------ |
| **PaymentSucceeded** | The Spark transfer is complete. | Show the payment as complete and call `get_info` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/guide/get_info.md). |

#### USDC/USDT

| Event                | Description                                                                                              | UX Suggestion                                    |
| -------------------- | -------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The deposit transfer has been submitted to the provider. The cross-chain leg is awaiting settlement.     | Show payment as pending; the bridge leg may take several minutes depending on the provider and destination chain. |
| **PaymentSucceeded** | The provider reports the cross-chain order terminal. The amount actually delivered to the recipient is carried on the conversion info. | Show the payment as complete and call `get_info` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/guide/get_info.md). |

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
