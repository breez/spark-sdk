# Receiving payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

Once the SDK is initialized, you can directly begin receiving payments. The SDK currently supports three methods of receiving: Lightning, Bitcoin and Spark.

## Lightning

#### BOLT11 invoice

When receiving via Lightning, we can generate a BOLT11 invoice to be paid. Setting the invoice amount fixes the amount the sender should pay.

**Note:** the payment may fallback to a direct Spark payment (if the payer's client supports this).

##### Rust

```rust
let description = "<invoice description>".to_string();
// Optionally set the invoice amount you wish the payer to send
let optional_amount_sats = Some(5_000);
// Optionally set the expiry duration in seconds
let optional_expiry_secs = Some(3600_u32);

let response = sdk
    .receive_payment(ReceivePaymentRequest {
        payment_method: ReceivePaymentMethod::Bolt11Invoice {
            description,
            amount_sats: optional_amount_sats,
            expiry_secs: optional_expiry_secs,
            payment_hash: None,
        },
    })
    .await?;

let payment_request = response.payment_request;
info!("Payment request: {payment_request}");
let receive_fee_sats = response.fee;
info!("Fees: {receive_fee_sats} sats");
```

##### Swift

```swift
let description = "<invoice description>"
// Optionally set the invoice amount you wish the payer to send
let optionalAmountSats: UInt64 = 5_000
// Optionally set the expiry duration in seconds
let optionalExpirySecs: UInt32 = 3600
let response =
    try await sdk
    .receivePayment(
        request: ReceivePaymentRequest(
            paymentMethod: ReceivePaymentMethod.bolt11Invoice(
                description: description,
                amountSats: optionalAmountSats,
                expirySecs: optionalExpirySecs,
                paymentHash: nil
            )
        ))

let paymentRequest = response.paymentRequest
print("Payment Request: {}", paymentRequest)
let receiveFeeSats = response.fee
print("Fees: {} sats", receiveFeeSats)
```

##### Kotlin

```kotlin
try {
    val description = "<invoice description>"
    // Optionally set the invoice amount you wish the payer to send
    val optionalAmountSats = 5_000.toULong()
    // Optionally set the expiry duration in seconds
    val optionalExpirySecs = 3600.toUInt()

    val request = ReceivePaymentRequest(
        ReceivePaymentMethod.Bolt11Invoice(
            description,
            optionalAmountSats,
            optionalExpirySecs,
            null
        )
    )
    val response = sdk.receivePayment(request)

    val paymentRequest = response.paymentRequest
    // Log.v("Breez", "Payment Request: ${paymentRequest}")
    val receiveFeeSats = response.fee
    // Log.v("Breez", "Fees: ${receiveFeeSats} sats")
} catch (e: Exception) {
    // handle error
}
```

##### C#

```csharp
var description = "<invoice description>";
// Optionally set the invoice amount you wish the payer to send
var optionalAmountSats = 5_000UL;
// Optionally set the expiry duration in seconds
var optionalExpirySecs = 3600U;
var paymentMethod = new ReceivePaymentMethod.Bolt11Invoice(
    description: description,
    amountSats: optionalAmountSats,
    expirySecs: optionalExpirySecs,
    paymentHash: null
);
var request = new ReceivePaymentRequest(paymentMethod: paymentMethod);
var response = await sdk.ReceivePayment(request: request);

var paymentRequest = response.paymentRequest;
Console.WriteLine($"Payment Request: {paymentRequest}");
var receiveFeeSats = response.fee;
Console.WriteLine($"Fees: {receiveFeeSats} sats");
```

##### Javascript (Wasm)

```typescript
const description = '<invoice description>'
// Optionally set the invoice amount you wish the payer to send
const optionalAmountSats = 5_000
// Optionally set the expiry duration in seconds
const optionalExpirySecs = 3600

const response = await sdk.receivePayment({
  paymentMethod: {
    type: 'bolt11Invoice',
    description,
    amountSats: optionalAmountSats,
    expirySecs: optionalExpirySecs,
    paymentHash: undefined
  }
})

const paymentRequest = response.paymentRequest
console.log(`Payment Request: ${paymentRequest}`)
const receiveFeeSats = response.fee
console.log(`Fees: ${receiveFeeSats} sats`)
```

##### React Native

```typescript
const description = '<invoice description>'
// Optionally set the invoice amount you wish the payer to send
const optionalAmountSats = BigInt(5_000)
// Optionally set the expiry duration in seconds
const optionalExpirySecs = 3600

const response = await sdk.receivePayment({
  paymentMethod: new ReceivePaymentMethod.Bolt11Invoice({
    description,
    amountSats: optionalAmountSats,
    expirySecs: optionalExpirySecs,
    paymentHash: undefined
  })
})

const paymentRequest = response.paymentRequest
console.log(`Payment Request: ${paymentRequest}`)
const receiveFeeSats = response.fee
console.log(`Fees: ${receiveFeeSats} sats`)
```

##### Flutter

```dart
String description = "<invoice description>";
// Optionally set the invoice amount you wish the payer to send
BigInt optionalAmountSats = BigInt.from(5000);
// Optionally set the expiry duration in seconds
int optionalExpirySecs = 3600;

// Create an invoice and set the amount you wish the payer to send
ReceivePaymentRequest request = ReceivePaymentRequest(
    paymentMethod: ReceivePaymentMethod.bolt11Invoice(
        description: description,
        amountSats: optionalAmountSats,
        expirySecs: optionalExpirySecs,
        paymentHash: null));
ReceivePaymentResponse response = await sdk.receivePayment(
  request: request,
);

String paymentRequest = response.paymentRequest;
print("Payment request: $paymentRequest");
BigInt receiveFeeSats = response.fee;
print("Fees: $receiveFeeSats sats");
```

##### Python

```python
try:
    description = "<invoice description>"
    # Optionally set the invoice amount you wish the payer to send
    optional_amount_sats = 5_000
    # Optionally set the expiry duration in seconds
    optional_expiry_secs = 3600
    payment_method = ReceivePaymentMethod.BOLT11_INVOICE(
        description=description,
        amount_sats=optional_amount_sats,
        expiry_secs=optional_expiry_secs,
        payment_hash=None,
    )
    request = ReceivePaymentRequest(payment_method=payment_method)
    response = await sdk.receive_payment(request=request)

    payment_request = response.payment_request
    logging.debug(f"Payment Request: {payment_request}")
    receive_fee_sats = response.fee
    logging.debug(f"Fees: {receive_fee_sats} sats")
    return response
except Exception as error:
    logging.error(error)
    raise
```

##### Go

```go
description := "<invoice description>"
// Optionally set the invoice amount you wish the payer to send
optionalAmountSats := uint64(5_000)
// Optionally set the expiry duration in seconds
optionalExpirySecs := uint32(3600)

request := breez_sdk_spark.ReceivePaymentRequest{
	PaymentMethod: breez_sdk_spark.ReceivePaymentMethodBolt11Invoice{
		Description: description,
		AmountSats:  &optionalAmountSats,
		ExpirySecs:  &optionalExpirySecs,
		PaymentHash: nil,
	},
}

response, err := sdk.ReceivePayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

paymentRequest := response.PaymentRequest
log.Printf("Payment Request: %v", paymentRequest)
receiveFeesSat := response.Fee
log.Printf("Fees: %v sats", receiveFeesSat)
```



#### LNURL-Pay & Lightning address

To receive via LNURL-Pay and/or a Lightning address, follow [these instructions](/guide/receive_lnurl_pay.md).

> Note: Lightning payments work in Spark even if the receiver is offline. To understand how it works under the hood, read [this](https://docs.spark.money/learn/lightning).

## Bitcoin

For on-chain payments you can generate a Bitcoin deposit address to receive payments. By default the existing address is returned; you can optionally request a new address to rotate to a fresh one for improved privacy. All previously generated addresses remain monitored.

On-chain deposits go through the following lifecycle:

1. **Detected** — The SDK detects the deposit and emits a `SdkEvent::NewDeposits` event. The deposit may or may not have sufficient confirmations to be claimed yet.
2. **Sufficient confirmations** — After **3 on-chain confirmations**, the deposit has sufficient confirmations and the SDK automatically attempts to claim it.
3. **Claimed or unclaimed** — If claiming succeeds, the funds are added to your balance. If it fails (e.g. fees too high), the deposit remains unclaimed and can be [manually claimed or refunded](/guide/onchain_claims.md).

### Rust

```rust
let new_address = None; // Set to Some(true) to get a new address
let response = sdk
    .receive_payment(ReceivePaymentRequest {
        payment_method: ReceivePaymentMethod::BitcoinAddress { new_address },
    })
    .await?;

let payment_request = response.payment_request;
info!("Payment request: {payment_request}");
let receive_fee_sats = response.fee;
info!("Fees: {receive_fee_sats} sats");
```

### Swift

```swift
let newAddress: Bool? = nil // Set to true to get a new address
let response =
    try await sdk
    .receivePayment(
        request: ReceivePaymentRequest(
            paymentMethod: ReceivePaymentMethod.bitcoinAddress(
                newAddress: newAddress)
        ))

let paymentRequest = response.paymentRequest
print("Payment Request: {}", paymentRequest)
let receiveFeeSats = response.fee
print("Fees: {} sats", receiveFeeSats)
```

### Kotlin

```kotlin
try {
    val newAddress: Boolean? = null // Set to true to get a new address
    val request = ReceivePaymentRequest(
        ReceivePaymentMethod.BitcoinAddress(newAddress = newAddress)
    )
    val response = sdk.receivePayment(request)

    val paymentRequest = response.paymentRequest
    // Log.v("Breez", "Payment Request: ${paymentRequest}")
    val receiveFeeSats = response.fee
    // Log.v("Breez", "Fees: ${receiveFeeSats} sats")
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
bool? newAddress = null; // Set to true to get a new address
var request = new ReceivePaymentRequest(
    paymentMethod: new ReceivePaymentMethod.BitcoinAddress(
        newAddress: newAddress)
);
var response = await sdk.ReceivePayment(request: request);

var paymentRequest = response.paymentRequest;
Console.WriteLine($"Payment Request: {paymentRequest}");
var receiveFeeSats = response.fee;
Console.WriteLine($"Fees: {receiveFeeSats} sats");
```

### Javascript (Wasm)

```typescript
const newAddress = undefined // Set to true to get a new address
const response = await sdk.receivePayment({
  paymentMethod: { type: 'bitcoinAddress', newAddress }
})

const paymentRequest = response.paymentRequest
console.log(`Payment Request: ${paymentRequest}`)
const receiveFeeSats = response.fee
console.log(`Fees: ${receiveFeeSats} sats`)
```

### React Native

```typescript
const newAddress = undefined // Set to true to get a new address
const response = await sdk.receivePayment({
  paymentMethod: new ReceivePaymentMethod.BitcoinAddress({
    newAddress
  })
})

const paymentRequest = response.paymentRequest
console.log(`Payment Request: ${paymentRequest}`)
const receiveFeeSats = response.fee
console.log(`Fees: ${receiveFeeSats} sats`)
```

### Flutter

```dart
bool? newAddress; // Set to true to get a new address
ReceivePaymentRequest request = ReceivePaymentRequest(
    paymentMethod: ReceivePaymentMethod.bitcoinAddress(
        newAddress: newAddress));
ReceivePaymentResponse response = await sdk.receivePayment(
  request: request,
);

String paymentRequest = response.paymentRequest;
print("Payment request: $paymentRequest");
BigInt receiveFeeSats = response.fee;
print("Fees: $receiveFeeSats sats");
```

### Python

```python
try:
    new_address = None  # Set to True to get a new address
    request = ReceivePaymentRequest(
        payment_method=ReceivePaymentMethod.BITCOIN_ADDRESS(
            new_address=new_address)
    )
    response = await sdk.receive_payment(request=request)

    payment_request = response.payment_request
    logging.debug(f"Payment Request: {payment_request}")
    receive_fee_sats = response.fee
    logging.debug(f"Fees: {receive_fee_sats} sats")
    return response
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
var newAddress *bool // To get a new address: t := true; newAddress = &t
request := breez_sdk_spark.ReceivePaymentRequest{
	PaymentMethod: breez_sdk_spark.ReceivePaymentMethodBitcoinAddress{
		NewAddress: newAddress,
	},
}

response, err := sdk.ReceivePayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

paymentRequest := response.PaymentRequest
log.Printf("Payment Request: %v", paymentRequest)
receiveFeesSat := response.Fee
log.Printf("Fees: %v sats", receiveFeesSat)
```



To track pending deposits, use `list_unclaimed_deposits` and filter by the `is_mature` field:

### Rust

```rust
let request = ListUnclaimedDepositsRequest {};
let response = sdk.list_unclaimed_deposits(request).await?;

let pending_deposits: Vec<&DepositInfo> =
    response.deposits.iter().filter(|d| !d.is_mature).collect();

for deposit in pending_deposits {
    info!("Pending deposit: {}:{}", deposit.txid, deposit.vout);
    info!("Amount: {} sats", deposit.amount_sats);
}
```

### Swift

```swift
let request = ListUnclaimedDepositsRequest()
let response = try await sdk.listUnclaimedDeposits(request: request)

let pendingDeposits = response.deposits.filter { !$0.isMature }

for deposit in pendingDeposits {
    print("Pending deposit: \(deposit.txid):\(deposit.vout)")
    print("Amount: \(deposit.amountSats) sats")
}
```

### Kotlin

```kotlin
try {
    val request = ListUnclaimedDepositsRequest
    val response = sdk.listUnclaimedDeposits(request)

    val pendingDeposits = response.deposits.filter { !it.isMature }

    for (deposit in pendingDeposits) {
        // Log.v("Breez", "Pending deposit: ${deposit.txid}:${deposit.vout}")
        // Log.v("Breez", "Amount: ${deposit.amountSats} sats")
    }
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
var request = new ListUnclaimedDepositsRequest();
var response = await sdk.ListUnclaimedDeposits(request: request);

var pendingDeposits = response.deposits.Where(d => !d.isMature).ToList();

foreach (var deposit in pendingDeposits)
{
    Console.WriteLine($"Pending deposit: {deposit.txid}:{deposit.vout}");
    Console.WriteLine($"Amount: {deposit.amountSats} sats");
}
```

### Javascript (Wasm)

```typescript
const request: ListUnclaimedDepositsRequest = {}
const response = await sdk.listUnclaimedDeposits(request)

const pendingDeposits = response.deposits.filter((d) => !d.isMature)

for (const deposit of pendingDeposits) {
  console.log(`Pending deposit: ${deposit.txid}:${deposit.vout}`)
  console.log(`Amount: ${deposit.amountSats} sats`)
}
```

### React Native

```typescript
const request: ListUnclaimedDepositsRequest = {}
const response = await sdk.listUnclaimedDeposits(request)

const pendingDeposits = response.deposits.filter((d) => !d.isMature)

for (const deposit of pendingDeposits) {
  console.log(`Pending deposit: ${deposit.txid}:${deposit.vout}`)
  console.log(`Amount: ${deposit.amountSats} sats`)
}
```

### Flutter

```dart
final request = ListUnclaimedDepositsRequest();
final response = await sdk.listUnclaimedDeposits(request: request);

final pendingDeposits =
    response.deposits.where((d) => !d.isMature).toList();

for (DepositInfo deposit in pendingDeposits) {
  print("Pending deposit: ${deposit.txid}:${deposit.vout}");
  print("Amount: ${deposit.amountSats} sats");
}
```

### Python

```python
try:
    request = ListUnclaimedDepositsRequest()
    response = await sdk.list_unclaimed_deposits(request=request)

    pending_deposits = [d for d in response.deposits if not d.is_mature]

    for deposit in pending_deposits:
        logging.info(f"Pending deposit: {deposit.txid}:{deposit.vout}")
        logging.info(f"Amount: {deposit.amount_sats} sats")
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
request := breez_sdk_spark.ListUnclaimedDepositsRequest{}
response, err := sdk.ListUnclaimedDeposits(request)
if err != nil {
	return err
}

var pendingDeposits []breez_sdk_spark.DepositInfo
for _, deposit := range response.Deposits {
	if !deposit.IsMature {
		pendingDeposits = append(pendingDeposits, deposit)
	}
}

for _, deposit := range pendingDeposits {
	log.Printf("Pending deposit: %v:%v", deposit.Txid, deposit.Vout)
	log.Printf("Amount: %v sats", deposit.AmountSats)
}
```



## Spark

For payments between Spark users, you can use a Spark address or generate a Spark invoice to receive payments.

#### Spark address

Spark addresses are static.

##### Rust

```rust
let response = sdk
    .receive_payment(ReceivePaymentRequest {
        payment_method: ReceivePaymentMethod::SparkAddress,
    })
    .await?;

let payment_request = response.payment_request;
info!("Payment request: {payment_request}");
let receive_fee_sats = response.fee;
info!("Fees: {receive_fee_sats} sats");
```

##### Swift

```swift
let response =
    try await sdk
    .receivePayment(
        request: ReceivePaymentRequest(
            paymentMethod: ReceivePaymentMethod.sparkAddress
        ))

let paymentRequest = response.paymentRequest
print("Payment Request: {}", paymentRequest)
let receiveFeeSats = response.fee
print("Fees: {} sats", receiveFeeSats)
```

##### Kotlin

```kotlin
try {
    val request = ReceivePaymentRequest(ReceivePaymentMethod.SparkAddress)
    val response = sdk.receivePayment(request)

    val paymentRequest = response.paymentRequest
    // Log.v("Breez", "Payment Request: ${paymentRequest}")
    val receiveFeeSats = response.fee
    // Log.v("Breez", "Fees: ${receiveFeeSats} sats")
} catch (e: Exception) {
    // handle error
}
```

##### C#

```csharp
var request = new ReceivePaymentRequest(
    paymentMethod: new ReceivePaymentMethod.SparkAddress()
);
var response = await sdk.ReceivePayment(request: request);

var paymentRequest = response.paymentRequest;
Console.WriteLine($"Payment Request: {paymentRequest}");
var receiveFeeSats = response.fee;
Console.WriteLine($"Fees: {receiveFeeSats} sats");
```

##### Javascript (Wasm)

```typescript
const response = await sdk.receivePayment({
  paymentMethod: { type: 'sparkAddress' }
})

const paymentRequest = response.paymentRequest
console.log(`Payment Request: ${paymentRequest}`)
const receiveFeeSats = response.fee
console.log(`Fees: ${receiveFeeSats} sats`)
```

##### React Native

```typescript
const response = await sdk.receivePayment({
  paymentMethod: new ReceivePaymentMethod.SparkAddress()
})

const paymentRequest = response.paymentRequest
console.log(`Payment Request: ${paymentRequest}`)
const receiveFeeSats = response.fee
console.log(`Fees: ${receiveFeeSats} sats`)
```

##### Flutter

```dart
ReceivePaymentRequest request =
    ReceivePaymentRequest(paymentMethod: ReceivePaymentMethod.sparkAddress());
ReceivePaymentResponse response = await sdk.receivePayment(
  request: request,
);

String paymentRequest = response.paymentRequest;
print("Payment request: $paymentRequest");
BigInt receiveFeeSats = response.fee;
print("Fees: $receiveFeeSats sats");
```

##### Python

```python
try:
    request = ReceivePaymentRequest(
        payment_method=ReceivePaymentMethod.SPARK_ADDRESS()
    )
    response = await sdk.receive_payment(request=request)

    payment_request = response.payment_request
    logging.debug(f"Payment Request: {payment_request}")
    receive_fee_sats = response.fee
    logging.debug(f"Fees: {receive_fee_sats} sats")
    return response
except Exception as error:
    logging.error(error)
    raise
```

##### Go

```go
request := breez_sdk_spark.ReceivePaymentRequest{
	PaymentMethod: breez_sdk_spark.ReceivePaymentMethodSparkAddress{},
}

response, err := sdk.ReceivePayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

paymentRequest := response.PaymentRequest
log.Printf("Payment Request: %v", paymentRequest)
receiveFeesSat := response.Fee
log.Printf("Fees: %v sats", receiveFeesSat)
```



#### Spark invoice

Spark invoices are single-use and may impose restrictions on the payment, such as amount, expiry, and who is able to pay it.

##### Rust

```rust
let optional_description = "<invoice description>".to_string();
let optional_amount_sats = Some(5_000);
// Optionally set the expiry UNIX timestamp in seconds
let optional_expiry_time_seconds = Some(1716691200);
let optional_sender_public_key = Some("<sender public key>".to_string());

let response = sdk
    .receive_payment(ReceivePaymentRequest {
        payment_method: ReceivePaymentMethod::SparkInvoice {
            token_identifier: None,
            description: Some(optional_description),
            amount: optional_amount_sats,
            expiry_time: optional_expiry_time_seconds,
            sender_public_key: optional_sender_public_key,
        },
    })
    .await?;

let payment_request = response.payment_request;
info!("Payment request: {payment_request}");
let receive_fee_sats = response.fee;
info!("Fees: {receive_fee_sats} sats");
```

##### Swift

```swift
let optionalDescription = "<invoice description>"
let optionalAmountSats = BInt(5_000)
// Optionally set the expiry UNIX timestamp in seconds
let optionalExpiryTimeSeconds: UInt64 = 1_716_691_200
let optionalSenderPublicKey = "<sender public key>"

let response =
    try await sdk
    .receivePayment(
        request: ReceivePaymentRequest(
            paymentMethod: ReceivePaymentMethod.sparkInvoice(
                amount: optionalAmountSats,
                tokenIdentifier: nil,
                expiryTime: optionalExpiryTimeSeconds,
                description: optionalDescription,
                senderPublicKey: optionalSenderPublicKey
            )
        ))

let paymentRequest = response.paymentRequest
print("Payment Request: {}", paymentRequest)
let receiveFeeSats = response.fee
print("Fees: {} sats", receiveFeeSats)
```

##### Kotlin

```kotlin
try {
    val optionalDescription = "<invoice description>"
    // Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer, which is included in
    // package)
    val optionalAmountSats = BigInteger.fromLong(5_000L)
    // Android (BigInteger from java.math)
    // val optionalAmountSats = BigInteger.valueOf(5_000L)
    // Optionally set the expiry UNIX timestamp in seconds
    val optionalExpiryTimeSeconds = 1716691200.toULong()
    val optionalSenderPublicKey = "<sender public key>"

    val request = ReceivePaymentRequest(
        ReceivePaymentMethod.SparkInvoice(
            tokenIdentifier = null,
            description = optionalDescription,
            amount = optionalAmountSats,
            expiryTime = optionalExpiryTimeSeconds,
            senderPublicKey = optionalSenderPublicKey
        )
    )
    val response = sdk.receivePayment(request)

    val paymentRequest = response.paymentRequest
    // Log.v("Breez", "Payment Request: ${paymentRequest}")
    val receiveFeeSats = response.fee
    // Log.v("Breez", "Fees: ${receiveFeeSats} sats")
} catch (e: Exception) {
    // handle error
}
```

##### C#

```csharp
var optionalDescription = "<invoice description>";
var optionalAmountSats = new BigInteger(5000);
// Optionally set the expiry UNIX timestamp in seconds
var optionalExpiryTimeSeconds = 1716691200UL;
var optionalSenderPublicKey = "<sender public key>";

var request = new ReceivePaymentRequest(
    paymentMethod: new ReceivePaymentMethod.SparkInvoice(
        description: optionalDescription,
        amount: optionalAmountSats,
        expiryTime: optionalExpiryTimeSeconds,
        senderPublicKey: optionalSenderPublicKey,
        tokenIdentifier: null
    )
);
var response = await sdk.ReceivePayment(request: request);

var paymentRequest = response.paymentRequest;
Console.WriteLine($"Payment Request: {paymentRequest}");
var receiveFeeSats = response.fee;
Console.WriteLine($"Fees: {receiveFeeSats} sats");
```

##### Javascript (Wasm)

```typescript
const optionalDescription = '<invoice description>'
const optionalAmountSats = '5000'
// Optionally set the expiry UNIX timestamp in seconds
const optionalExpiryTimeSeconds = 1716691200
const optionalSenderPublicKey = '<sender public key>'

const response = await sdk.receivePayment({
  paymentMethod: {
    type: 'sparkInvoice',
    description: optionalDescription,
    amount: optionalAmountSats,
    expiryTime: optionalExpiryTimeSeconds,
    senderPublicKey: optionalSenderPublicKey
  }
})

const paymentRequest = response.paymentRequest
console.log(`Payment Request: ${paymentRequest}`)
const receiveFeeSats = response.fee
console.log(`Fees: ${receiveFeeSats} sats`)
```

##### React Native

```typescript
const optionalDescription = '<invoice description>'
const optionalAmountSats = BigInt(5_000)
// Optionally set the expiry UNIX timestamp in seconds
const optionalExpiryTimeSeconds = BigInt(1716691200)
const optionalSenderPublicKey = '<sender public key>'

const response = await sdk.receivePayment({
  paymentMethod: new ReceivePaymentMethod.SparkInvoice({
    description: optionalDescription,
    amount: optionalAmountSats,
    expiryTime: optionalExpiryTimeSeconds,
    senderPublicKey: optionalSenderPublicKey,
    tokenIdentifier: undefined
  })
})

const paymentRequest = response.paymentRequest
console.log(`Payment Request: ${paymentRequest}`)
const receiveFeeSats = response.fee
console.log(`Fees: ${receiveFeeSats} sats`)
```

##### Flutter

```dart
String optionalDescription = "<invoice description>";
BigInt optionalAmountSats = BigInt.from(5000);
// Optionally set the expiry UNIX timestamp in seconds
BigInt optionalExpiryTimeSeconds = BigInt.from(1716691200);
String optionalSenderPublicKey = "<sender public key>";

ReceivePaymentRequest request =
    ReceivePaymentRequest(paymentMethod: ReceivePaymentMethod.sparkInvoice(
      description: optionalDescription,
      amount: optionalAmountSats,
      expiryTime: optionalExpiryTimeSeconds,
      senderPublicKey: optionalSenderPublicKey,
    ));
ReceivePaymentResponse response = await sdk.receivePayment(
  request: request,
);

String paymentRequest = response.paymentRequest;
print("Payment request: $paymentRequest");
BigInt receiveFeeSats = response.fee;
print("Fees: $receiveFeeSats sats");
```

##### Python

```python
try:
    optional_description = "<invoice description>"
    optional_amount_sats = 5_000
    # Optionally set the expiry UNIX timestamp in seconds
    optional_expiry_time_seconds = 1716691200
    optional_sender_public_key = "<sender public key>"

    request = ReceivePaymentRequest(
        payment_method=ReceivePaymentMethod.SPARK_INVOICE(
            description=optional_description,
            amount=optional_amount_sats,
            expiry_time=optional_expiry_time_seconds,
            sender_public_key=optional_sender_public_key,
            token_identifier=None,
        )
    )
    response = await sdk.receive_payment(request=request)

    payment_request = response.payment_request
    logging.debug(f"Payment Request: {payment_request}")
    receive_fee_sats = response.fee
    logging.debug(f"Fees: {receive_fee_sats} sats")
    return response
except Exception as error:
    logging.error(error)
    raise
```

##### Go

```go
optionalDescription := "<invoice description>"
optionalAmountSats := new(big.Int).SetInt64(5_000)
// Optionally set the expiry UNIX timestamp in seconds
optionalExpiryTimeSeconds := uint64(1716691200)
optionalSenderPublicKey := "<sender public key>"

request := breez_sdk_spark.ReceivePaymentRequest{
	PaymentMethod: breez_sdk_spark.ReceivePaymentMethodSparkInvoice{
		Description:     &optionalDescription,
		Amount:          &optionalAmountSats,
		ExpiryTime:      &optionalExpiryTimeSeconds,
		SenderPublicKey: &optionalSenderPublicKey,
	},
}

response, err := sdk.ReceivePayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

paymentRequest := response.PaymentRequest
log.Printf("Payment Request: %v", paymentRequest)
receiveFeesSat := response.Fee
log.Printf("Fees: %v sats", receiveFeesSat)
```



## Event Flows

Once a receive payment is initiated, you can follow and react to the different payment events using the guide below for each payment method. See [listening to events](/guide/events.md) for how to subscribe to events. 

The `SdkEvent::Synced` event is also emitted as the SDK syncs in the background. See [fetching the balance](/guide/get_info.md) for the recommended pattern for refreshing the balance and payments list.

#### Lightning

| Event                | Description                                                       | UX Suggestion                                    |
| -------------------- | ----------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer was detected and the claim process will start. | Show payment as pending.                         |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete.        | Show the payment as complete and call `get_info` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/guide/get_info.md). |

#### Bitcoin

The following events are emitted in order during the deposit lifecycle. See [Listening to events](/guide/events.md) for how to subscribe.

| Event                 | Description                                                                                                                              | UX Suggestion                                                                                               |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| **NewDeposits**       | New deposits were detected. Each deposit includes a `is_mature` field indicating whether it has enough confirmations to be claimed. | Show the deposit to the user. If it does not yet have sufficient confirmations, show it as pending.          |
| **ClaimedDeposits**   | The SDK successfully claimed confirmed deposits.                                                                                         |                                                                                                             |
| **UnclaimedDeposits** | Claiming failed (e.g. fee exceeded the configured maximum or the UTXO could not be found).                                               | Allow the user to manually claim or refund. See [Claiming on-chain deposits](/guide/onchain_claims.md). |
| **PaymentPending**    | The Spark transfer was detected and the claim process will start.                                                                        | Show payment as pending.                                                                                    |
| **PaymentSucceeded**  | The Spark transfer is claimed and the payment is complete.                                                                               | Show the payment as complete and call `get_info` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/guide/get_info.md).                                                            |

#### Spark

| Event                | Description                                                                                                                                                                                          | UX Suggestion                                    |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer was detected and the claim process will start. For Spark HTLC payments, the claim will only start once the HTLC is claimed. For more details see [Spark HTLC payments](htlcs.md). | Show payment as pending.                         |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete.                                                                                                                                           | Show the payment as complete and call `get_info` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/guide/get_info.md). |

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
