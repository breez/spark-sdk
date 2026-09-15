# Receiving payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

Once the SDK is initialized, you can directly begin receiving payments. The SDK supports receiving via Lightning, Bitcoin, Spark, and USDC/USDT into Spark from a supported external chain.

## Lightning

#### BOLT11 invoice

When receiving via Lightning, we can generate a BOLT11 invoice to be paid. Setting the invoice amount fixes the amount the sender should pay.

To create an invoice for another Spark wallet, set `receiver_identity_public_key` to that wallet's identity public key. Creating the invoice requires only the receiver's public key, not their private keys.

**Note:** the payment may fallback to a direct Spark payment (if the payer's client supports this).

##### Rust

```rust
let description = "<invoice description>".to_string();
// Optionally set the invoice amount you wish the payer to send
let optional_amount_sats = Some(5_000);
// Optionally set the expiry duration in seconds
let optional_expiry_secs = Some(3600_u32);
// Set this to create an invoice for another Spark identity
let optional_receiver_identity_public_key = None;

let response = sdk
    .receive_payment(ReceivePaymentRequest {
        payment_method: ReceivePaymentMethod::Bolt11Invoice {
            description,
            amount_sats: optional_amount_sats,
            expiry_secs: optional_expiry_secs,
            payment_hash: None,
            receiver_identity_public_key: optional_receiver_identity_public_key,
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
let optionalReceiverIdentityPublicKey: String? = nil
let response =
    try await sdk
    .receivePayment(
        request: ReceivePaymentRequest(
            paymentMethod: ReceivePaymentMethod.bolt11Invoice(
                description: description,
                amountSats: optionalAmountSats,
                expirySecs: optionalExpirySecs,
                paymentHash: nil,
                receiverIdentityPublicKey: optionalReceiverIdentityPublicKey
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
    val optionalReceiverIdentityPublicKey: String? = null

    val request = ReceivePaymentRequest(
        ReceivePaymentMethod.Bolt11Invoice(
            description,
            optionalAmountSats,
            optionalExpirySecs,
            null,
            optionalReceiverIdentityPublicKey
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
string? optionalReceiverIdentityPublicKey = null;
var paymentMethod = new ReceivePaymentMethod.Bolt11Invoice(
    description: description,
    amountSats: optionalAmountSats,
    expirySecs: optionalExpirySecs,
    paymentHash: null,
    receiverIdentityPublicKey: optionalReceiverIdentityPublicKey
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
// Set this to create an invoice for another Spark identity
const optionalReceiverIdentityPublicKey = undefined

const response = await sdk.receivePayment({
  paymentMethod: {
    type: 'bolt11Invoice',
    description,
    amountSats: optionalAmountSats,
    expirySecs: optionalExpirySecs,
    paymentHash: undefined,
    receiverIdentityPublicKey: optionalReceiverIdentityPublicKey
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
// Set this to create an invoice for another Spark identity
const optionalReceiverIdentityPublicKey = undefined

const response = await sdk.receivePayment({
  paymentMethod: new ReceivePaymentMethod.Bolt11Invoice({
    description,
    amountSats: optionalAmountSats,
    expirySecs: optionalExpirySecs,
    paymentHash: undefined,
    receiverIdentityPublicKey: optionalReceiverIdentityPublicKey
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
// Set this to create an invoice for another Spark identity
String? optionalReceiverIdentityPublicKey;

// Create an invoice and set the amount you wish the payer to send
ReceivePaymentRequest request = ReceivePaymentRequest(
    paymentMethod: ReceivePaymentMethod.bolt11Invoice(
        description: description,
        amountSats: optionalAmountSats,
        expirySecs: optionalExpirySecs,
        paymentHash: null,
        receiverIdentityPublicKey: optionalReceiverIdentityPublicKey));
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
    # Set this to create an invoice for another Spark identity
    optional_receiver_identity_public_key = None
    payment_method = ReceivePaymentMethod.BOLT11_INVOICE(
        description=description,
        amount_sats=optional_amount_sats,
        expiry_secs=optional_expiry_secs,
        payment_hash=None,
        receiver_identity_public_key=optional_receiver_identity_public_key,
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
// Set this to create an invoice for another Spark identity
var optionalReceiverIdentityPublicKey *string

request := breez_sdk_spark.ReceivePaymentRequest{
	PaymentMethod: breez_sdk_spark.ReceivePaymentMethodBolt11Invoice{
		Description: description,
		AmountSats:  &optionalAmountSats,
		ExpirySecs:  &optionalExpirySecs,
		PaymentHash: nil,
		ReceiverIdentityPublicKey: optionalReceiverIdentityPublicKey,
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



## USDC/USDT

Cross-chain receive is supported only via the Orchestra provider. Receive USDC or USDT from a sender on one of several supported chains: Ethereum-family chains (Arbitrum, Base, and similar EVM networks), Solana, and Tron. The receiver lands either BTC sats or [USDB](./stable_balance.md) (a 6-decimal USD-pegged token on Spark) on the Spark side. This feature must be enabled in [the SDK configuration](./config.md#usdc-usdt) before using. See [USDC/USDT](./cross_chain.md) for provider details and the status lifecycle.

Call `get_cross_chain_routes` with `CrossChainRouteFilter::Receive` to discover supported source assets. Each `CrossChainRoutePair` names the provider, source chain and asset, decimals, optional token contract address, and the Spark-side destinations the route lands (`CrossChainRoutePair.accepted_assets`).

### Rust

```rust
let routes = sdk
    .get_cross_chain_routes(&CrossChainRouteFilter::Receive {
        contract_address: None,
    })
    .await?;

for route in &routes {
    info!(
        "Route via {:?}: {}/{} -> Spark",
        route.provider, route.chain, route.asset
    );
}
```

### Swift

```swift
let routes = try await sdk.getCrossChainRoutes(
    filter: .receive(contractAddress: nil))

for route in routes {
    print("Route via \(route.provider): \(route.chain)/\(route.asset) -> Spark")
}
```

### Kotlin

```kotlin
try {
    val routes = sdk.getCrossChainRoutes(
        CrossChainRouteFilter.Receive(contractAddress = null)
    )

    for (route in routes) {
        println("Route via ${route.provider}: ${route.chain}/${route.asset} -> Spark")
    }
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
var filter = new CrossChainRouteFilter.Receive(contractAddress: null);
var routes = await sdk.GetCrossChainRoutes(filter: filter);

foreach (var route in routes)
{
    Console.WriteLine(
        $"Route via {route.provider}: {route.chain}/{route.asset} -> Spark"
    );
}
```

### Javascript (Wasm)

```typescript
const routes = await sdk.getCrossChainRoutes({
  type: 'receive',
  contractAddress: undefined
})

for (const route of routes) {
  console.debug(
    `Route via ${route.provider}: ${route.chain}/${route.asset} -> Spark`
  )
}
```

### React Native

```typescript
const routes = await sdk.getCrossChainRoutes(
  new CrossChainRouteFilter.Receive({ contractAddress: undefined })
)

for (const route of routes) {
  console.debug(
    `Route via ${route.provider}: ${route.chain}/${route.asset} -> Spark`
  )
}
```

### Flutter

```dart
List<CrossChainRoutePair> routes = await sdk.getCrossChainRoutes(
  filter: CrossChainRouteFilter.receive(contractAddress: null),
);

for (var route in routes) {
  print(
    "Route via ${route.provider}: ${route.chain}/${route.asset} -> Spark",
  );
}
```

### Python

```python
try:
    routes = await sdk.get_cross_chain_routes(
        filter=CrossChainRouteFilter.RECEIVE(contract_address=None)
    )

    for route in routes:
        logging.debug(
            f"Route via {route.provider}: {route.chain}/{route.asset} -> Spark"
        )
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
filter := breez_sdk_spark.CrossChainRouteFilterReceive{ContractAddress: nil}
routes, err := sdk.GetCrossChainRoutes(filter)
if err != nil {
	return nil, err
}

for _, route := range routes {
	log.Printf(
		"Route via %v: %s/%s -> Spark",
		route.Provider, route.Chain, route.Asset,
	)
}
```



Build `ReceivePaymentMethod::CrossChain` with the chosen route and an `amount`.

The `amount` on {{#name ReceivePaymentMethod::CrossChain}} is in the source asset's base units, per the route's `CrossChainRoutePair.decimals`. USD-stable sources sit at USD parity, so `1_000_000` is 1 USDC (6 decimals), about $1.

`fee_mode` controls what the amount means:

- `CrossChainFeeMode::FeesExcluded` (default): `amount` is the receiver's target on Spark. The SDK pads the sender's deposit to cover provider fees plus an overpay buffer.
- `CrossChainFeeMode::FeesIncluded`: `amount` is the deposit the sender pays. The receiver lands `amount - fees`.

`destination` picks which Spark-side asset the receiver wants delivered. Left unset, the SDK auto-picks the wallet's active stable-balance token if the route supports it, otherwise BTC (converted from the USD amount via the live BTC/USD rate).

`max_slippage_bps` (10 to 500) bounds the price movement tolerated between quote and delivery. `target_overpay_bps` (0 to 500) sets the FeesExcluded overpay buffer. Left unset, the SDK defaults apply.

The `payment_request` field carries an EIP-681 URI for EVM routes and the bare deposit address for Solana and Tron. The `cross_chain_info` block surfaces the bare deposit address, deposit amount, expected receive amount, destination denomination, and quote `expires_at`. The receiver pays no fee; the sender's deposit covers it.

### Rust

```rust
// amount is in the route's source-asset base units (USD-stable parity:
// 1_000_000 = $1 on 6-decimal routes). See the guide for fee_mode,
// destination, and the slippage/overpay overrides.
let amount = 1_000_000u128;
let optional_destination: Option<SparkAsset> = None;
let optional_max_slippage_bps = Some(100);
let optional_target_overpay_bps: Option<u32> = None;
let optional_fee_mode: Option<CrossChainFeeMode> = None;

let response = sdk
    .receive_payment(ReceivePaymentRequest {
        payment_method: ReceivePaymentMethod::CrossChain {
            route,
            amount,
            destination: optional_destination,
            fee_mode: optional_fee_mode,
            max_slippage_bps: optional_max_slippage_bps,
            target_overpay_bps: optional_target_overpay_bps,
        },
    })
    .await?;

info!("Payment request: {}", response.payment_request);
if let Some(info) = response.cross_chain_info {
    info!("Deposit address: {}", info.deposit_address);
    info!("Deposit amount: {}", info.deposit_amount);
    info!(
        "Expected received: {} {}",
        info.expected_received_amount, info.destination_asset
    );
    info!("Expires at: {}", info.expires_at);
}
```

### Swift

```swift
// amount is in the route's source-asset base units (USD-stable parity:
// 1_000_000 = $1 on 6-decimal routes). See the guide for feeMode,
// destination, and the slippage/overpay overrides.
let amount = BInt(1_000_000)
let optionalDestination: SparkAsset? = nil
let optionalMaxSlippageBps: UInt32? = 100
let optionalTargetOverpayBps: UInt32? = nil
let optionalFeeMode: CrossChainFeeMode? = nil

let response = try await sdk.receivePayment(
    request: ReceivePaymentRequest(
        paymentMethod: .crossChain(
            route: route,
            amount: amount,
            destination: optionalDestination,
            feeMode: optionalFeeMode,
            maxSlippageBps: optionalMaxSlippageBps,
            targetOverpayBps: optionalTargetOverpayBps
        )
    ))

print("Payment request: \(response.paymentRequest)")
if let info = response.crossChainInfo {
    print("Deposit address: \(info.depositAddress)")
    print("Deposit amount: \(info.depositAmount)")
    print(
        "Expected received: \(info.expectedReceivedAmount) "
            + "\(info.destinationAsset)"
    )
    print("Expires at: \(info.expiresAt)")
}
```

### Kotlin

```kotlin
// amount is in the route's source-asset base units (USD-stable parity:
// 1_000_000 = $1 on 6-decimal routes). See the guide for feeMode,
// destination, and the slippage/overpay overrides.
val amount = BigInteger.fromLong(1_000_000L)
val optionalDestination: SparkAsset? = null
val optionalMaxSlippageBps: UInt? = 100u
val optionalTargetOverpayBps: UInt? = null
val optionalFeeMode: CrossChainFeeMode? = null
try {
    val req = ReceivePaymentRequest(
        paymentMethod = ReceivePaymentMethod.CrossChain(
            route = route,
            amount = amount,
            destination = optionalDestination,
            feeMode = optionalFeeMode,
            maxSlippageBps = optionalMaxSlippageBps,
            targetOverpayBps = optionalTargetOverpayBps,
        ),
    )
    val response = sdk.receivePayment(req)
    println("Payment request: ${response.paymentRequest}")
    val info = response.crossChainInfo
    if (info != null) {
        println("Deposit address: ${info.depositAddress}")
        println("Deposit amount: ${info.depositAmount}")
        println(
            "Expected received: ${info.expectedReceivedAmount} " +
                "${info.destinationAsset}",
        )
        println("Expires at: ${info.expiresAt}")
    }
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
// amount is in the route's source-asset base units (USD-stable
// parity: 1_000_000 = $1 on 6-decimal routes). See the guide for
// feeMode, destination, and the slippage/overpay overrides.
var amount = new BigInteger(1_000_000);
SparkAsset? optionalDestination = null;
uint? optionalMaxSlippageBps = 100;
uint? optionalTargetOverpayBps = null;
CrossChainFeeMode? optionalFeeMode = null;

var request = new ReceivePaymentRequest(
    paymentMethod: new ReceivePaymentMethod.CrossChain(
        route: route,
        amount: amount,
        destination: optionalDestination,
        feeMode: optionalFeeMode,
        maxSlippageBps: optionalMaxSlippageBps,
        targetOverpayBps: optionalTargetOverpayBps
    )
);
var response = await sdk.ReceivePayment(request: request);

Console.WriteLine($"Payment request: {response.paymentRequest}");
if (response.crossChainInfo is { } info)
{
    Console.WriteLine($"Deposit address: {info.depositAddress}");
    Console.WriteLine($"Deposit amount: {info.depositAmount}");
    Console.WriteLine(
        "Expected received: "
            + $"{info.expectedReceivedAmount} {info.destinationAsset}"
    );
    Console.WriteLine($"Expires at: {info.expiresAt}");
}
```

### Javascript (Wasm)

```typescript
// amount is in the route's source-asset base units (USD-stable parity:
// 1_000_000 = $1 on 6-decimal routes). See the guide for feeMode,
// destination, and the slippage/overpay overrides.
const amount = '1000000'
const optionalDestination = undefined
const optionalMaxSlippageBps = 100
const optionalTargetOverpayBps = undefined
const optionalFeeMode = undefined

const response = await sdk.receivePayment({
  paymentMethod: {
    type: 'crossChain',
    route,
    amount,
    destination: optionalDestination,
    feeMode: optionalFeeMode,
    maxSlippageBps: optionalMaxSlippageBps,
    targetOverpayBps: optionalTargetOverpayBps
  }
})

console.debug(`Payment request: ${response.paymentRequest}`)
if (response.crossChainInfo !== undefined) {
  const {
    depositAddress,
    depositAmount,
    expectedReceivedAmount,
    destinationAsset,
    expiresAt
  } = response.crossChainInfo
  console.debug(`Deposit address: ${depositAddress}`)
  console.debug(`Deposit amount: ${depositAmount}`)
  console.debug(
    `Expected received: ${expectedReceivedAmount} ${destinationAsset}`
  )
  console.debug(`Expires at: ${expiresAt}`)
}
```

### React Native

```typescript
// amount is in the route's source-asset base units (USD-stable parity:
// 1_000_000 = $1 on 6-decimal routes). See the guide for feeMode,
// destination, and the slippage/overpay overrides.
const amount = BigInt(1_000_000)
const optionalDestination: SparkAsset | undefined = undefined
const optionalMaxSlippageBps = 100
const optionalTargetOverpayBps = undefined
const optionalFeeMode = undefined

const response = await sdk.receivePayment({
  paymentMethod: new ReceivePaymentMethod.CrossChain({
    route,
    amount,
    destination: optionalDestination,
    feeMode: optionalFeeMode,
    maxSlippageBps: optionalMaxSlippageBps,
    targetOverpayBps: optionalTargetOverpayBps
  })
})

console.debug(`Payment request: ${response.paymentRequest}`)
if (response.crossChainInfo !== undefined) {
  const {
    depositAddress,
    depositAmount,
    expectedReceivedAmount,
    destinationAsset,
    expiresAt
  } = response.crossChainInfo
  console.debug(`Deposit address: ${depositAddress}`)
  console.debug(`Deposit amount: ${depositAmount}`)
  console.debug(
    `Expected received: ${expectedReceivedAmount} ${destinationAsset}`
  )
  console.debug(`Expires at: ${expiresAt}`)
}
```

### Flutter

```dart
// amount is in the route's source-asset base units (USD-stable parity:
// 1_000_000 = $1 on 6-decimal routes). See the guide for feeMode,
// destination, and the slippage/overpay overrides.
final amount = BigInt.from(1000000);
SparkAsset? optionalDestination;
int? optionalMaxSlippageBps = 100;
int? optionalTargetOverpayBps;
CrossChainFeeMode? optionalFeeMode;

final request = ReceivePaymentRequest(
  paymentMethod: ReceivePaymentMethod.crossChain(
    route: route,
    amount: amount,
    destination: optionalDestination,
    feeMode: optionalFeeMode,
    maxSlippageBps: optionalMaxSlippageBps,
    targetOverpayBps: optionalTargetOverpayBps,
  ),
);
final response = await sdk.receivePayment(request: request);

print("Payment request: ${response.paymentRequest}");
final info = response.crossChainInfo;
if (info != null) {
  print("Deposit address: ${info.depositAddress}");
  print("Deposit amount: ${info.depositAmount}");
  print(
    "Expected received: ${info.expectedReceivedAmount} ${info.destinationAsset}",
  );
  print("Expires at: ${info.expiresAt}");
}
```

### Python

```python
# amount is in the route's source-asset base units (USD-stable parity:
# 1_000_000 = $1 on 6-decimal routes). See the guide for fee_mode,
# destination, and the slippage/overpay overrides.
amount = 1_000_000
optional_destination = None
optional_max_slippage_bps = 100
optional_target_overpay_bps = None
optional_fee_mode = None
try:
    request = ReceivePaymentRequest(
        payment_method=ReceivePaymentMethod.CROSS_CHAIN(
            route=route,
            amount=amount,
            destination=optional_destination,
            fee_mode=optional_fee_mode,
            max_slippage_bps=optional_max_slippage_bps,
            target_overpay_bps=optional_target_overpay_bps,
        )
    )
    response = await sdk.receive_payment(request=request)
    logging.debug(f"Payment request: {response.payment_request}")
    info = response.cross_chain_info
    if info is not None:
        logging.debug(f"Deposit address: {info.deposit_address}")
        logging.debug(f"Deposit amount: {info.deposit_amount}")
        logging.debug(
            f"Expected received: {info.expected_received_amount} "
            f"{info.destination_asset}"
        )
        logging.debug(f"Expires at: {info.expires_at}")
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
// amount is in the route's source-asset base units (USD-stable parity:
// 1_000_000 = $1 on 6-decimal routes). See the guide for FeeMode,
// destination, and the slippage/overpay overrides.
amount := new(big.Int).SetInt64(1_000_000)
var optionalDestination *breez_sdk_spark.SparkAsset = nil
optionalMaxSlippageBps := uint32(100)
var optionalTargetOverpayBps *uint32 = nil
var optionalFeeMode *breez_sdk_spark.CrossChainFeeMode = nil

request := breez_sdk_spark.ReceivePaymentRequest{
	PaymentMethod: breez_sdk_spark.ReceivePaymentMethodCrossChain{
		Route:             route,
		Amount:            amount,
		Destination:       optionalDestination,
		FeeMode:           optionalFeeMode,
		MaxSlippageBps:    &optionalMaxSlippageBps,
		TargetOverpayBps:  optionalTargetOverpayBps,
	},
}
response, err := sdk.ReceivePayment(request)
if err != nil {
	return nil, err
}

log.Printf("Payment request: %s", response.PaymentRequest)
if info := response.CrossChainInfo; info != nil {
	log.Printf("Deposit address: %s", info.DepositAddress)
	log.Printf("Deposit amount: %v", info.DepositAmount)
	log.Printf(
		"Expected received: %v %s",
		info.ExpectedReceivedAmount, info.DestinationAsset,
	)
	log.Printf("Expires at: %d", info.ExpiresAt)
}
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

#### USDC/USDT

| Event                | Description                                                                                          | UX Suggestion                                    |
| -------------------- | ---------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The sender's deposit was detected and the inbound Spark transfer claim is in progress.               | Show payment as pending.                         |
| **PaymentSucceeded** | The inbound Spark transfer is claimed and the payment is complete.                                   | Show the payment as complete and call `get_info` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/guide/get_info.md). |

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
