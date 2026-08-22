# Sending and receiving tokens

Spark supports tokens using the [BTKN protocol](https://docs.spark.money/learn/tokens/hello-btkn). The Breez SDK enables you to send and receive these tokens using the standard payments API.

## Fetching token balances

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_info

Token balances for all tokens currently held in the wallet can be retrieved along with general wallet information. Each token balance includes both the balance amount and the token metadata (identifier, name, ticker, issuer public key, etc.).

### Rust

```rust
let info = sdk
    .get_info(GetInfoRequest {
        // ensure_synced: true will ensure the SDK is synced with the Spark network
        // before returning the balance
        ensure_synced: Some(false),
    })
    .await?;

// Token balances are a map of token identifier to balance
let token_balances = info.token_balances;
for (token_id, token_balance) in token_balances {
    info!("Token ID: {}", token_id);
    info!("Balance: {}", token_balance.balance);
    info!("Name: {}", token_balance.token_metadata.name);
    info!("Ticker: {}", token_balance.token_metadata.ticker);
    info!("Decimals: {}", token_balance.token_metadata.decimals);
}
```

### Swift

```swift
// ensureSynced: true will ensure the SDK is synced with the Spark network
// before returning the balance
let info = try await sdk.getInfo(
    request: GetInfoRequest(
        ensureSynced: false
    ))

// Token balances are a map of token identifier to balance
let tokenBalances = info.tokenBalances
for (tokenId, tokenBalance) in tokenBalances {
    print("Token ID: \(tokenId)")
    print("Balance: \(tokenBalance.balance)")
    print("Name: \(tokenBalance.tokenMetadata.name)")
    print("Ticker: \(tokenBalance.tokenMetadata.ticker)")
    print("Decimals: \(tokenBalance.tokenMetadata.decimals)")
}
```

### Kotlin

```kotlin
try {
    // ensureSynced: true will ensure the SDK is synced with the Spark network
    // before returning the balance
    val info = sdk.getInfo(GetInfoRequest(false))

    // Token balances are a map of token identifier to balance
    val tokenBalances = info.tokenBalances
    for ((tokenId, tokenBalance) in tokenBalances) {
        println("Token ID: $tokenId")
        println("Balance: ${tokenBalance.balance}")
        println("Name: ${tokenBalance.tokenMetadata.name}")
        println("Ticker: ${tokenBalance.tokenMetadata.ticker}")
        println("Decimals: ${tokenBalance.tokenMetadata.decimals}")
    }
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
// ensureSynced: true will ensure the SDK is synced with the Spark network
// before returning the balance
var info = await sdk.GetInfo(request: new GetInfoRequest(ensureSynced: false));

// Token balances are a map of token identifier to balance
var tokenBalances = info.tokenBalances;
foreach (var kvp in tokenBalances)
{
    var tokenId = kvp.Key;
    var tokenBalance = kvp.Value;
    Console.WriteLine($"Token ID: {tokenId}");
    Console.WriteLine($"Balance: {tokenBalance.balance}");
    Console.WriteLine($"Name: {tokenBalance.tokenMetadata.name}");
    Console.WriteLine($"Ticker: {tokenBalance.tokenMetadata.ticker}");
    Console.WriteLine($"Decimals: {tokenBalance.tokenMetadata.decimals}");
}
```

### Javascript (Wasm)

```typescript
const info = await sdk.getInfo({
  // ensureSynced: true will ensure the SDK is synced with the Spark network
  // before returning the balance
  ensureSynced: false
})

// Token balances are a map of token identifier to balance
const tokenBalances = info.tokenBalances
for (const [tokenId, tokenBalance] of Object.entries(tokenBalances)) {
  console.log(`Token ID: ${tokenId}`)
  console.log(`Balance: ${tokenBalance.balance}`)
  console.log(`Name: ${tokenBalance.tokenMetadata.name}`)
  console.log(`Ticker: ${tokenBalance.tokenMetadata.ticker}`)
  console.log(`Decimals: ${tokenBalance.tokenMetadata.decimals}`)
}
```

### React Native

```typescript
const info = await sdk.getInfo({
  // ensureSynced: true will ensure the SDK is synced with the Spark network
  // before returning the balance
  ensureSynced: false
})

// Token balances are a map of token identifier to balance
const tokenBalances = info.tokenBalances
for (const [tokenId, tokenBalance] of Object.entries(tokenBalances)) {
  console.log(`Token ID: ${tokenId}`)
  console.log(`Balance: ${tokenBalance.balance}`)
  console.log(`Name: ${tokenBalance.tokenMetadata.name}`)
  console.log(`Ticker: ${tokenBalance.tokenMetadata.ticker}`)
  console.log(`Decimals: ${tokenBalance.tokenMetadata.decimals}`)
}
```

### Flutter

```dart
// ensureSynced: true will ensure the SDK is synced with the Spark network
// before returning the balance
final info = await sdk.getInfo(request: GetInfoRequest(ensureSynced: false));

// Token balances are a map of token identifier to balance
final tokenBalances = info.tokenBalances;
tokenBalances.forEach((tokenId, tokenBalance) {
  print('Token ID: $tokenId');
  print('Balance: ${tokenBalance.balance}');
  print('Name: ${tokenBalance.tokenMetadata.name}');
  print('Ticker: ${tokenBalance.tokenMetadata.ticker}');
  print('Decimals: ${tokenBalance.tokenMetadata.decimals}');
});
```

### Python

```python
try:
    # ensure_synced: True will ensure the SDK is synced with the Spark network
    # before returning the balance
    info = await sdk.get_info(request=GetInfoRequest(ensure_synced=False))

    # Token balances are a map of token identifier to balance
    token_balances = info.token_balances
    for token_id, token_balance in token_balances.items():
        print(f"Token ID: {token_id}")
        print(f"Balance: {token_balance.balance}")
        print(f"Name: {token_balance.token_metadata.name}")
        print(f"Ticker: {token_balance.token_metadata.ticker}")
        print(f"Decimals: {token_balance.token_metadata.decimals}")
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
ensureSynced := false
info, err := sdk.GetInfo(breez_sdk_spark.GetInfoRequest{
	// EnsureSynced: true will ensure the SDK is synced with the Spark network
	// before returning the balance
	EnsureSynced: &ensureSynced,
})

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}

// Token balances are a map of token identifier to balance
tokenBalances := info.TokenBalances
for tokenId, tokenBalance := range tokenBalances {
	log.Printf("Token ID: %v", tokenId)
	log.Printf("Balance: %v", tokenBalance.Balance)
	log.Printf("Name: %v", tokenBalance.TokenMetadata.Name)
	log.Printf("Ticker: %v", tokenBalance.TokenMetadata.Ticker)
	log.Printf("Decimals: %v", tokenBalance.TokenMetadata.Decimals)
}
```



**Developer note**

Token balances are cached for fast responses. For details on ensuring up-to-date balances, see the <a href="./get_info.md#fetching-the-balance">Fetching the balance</a> section.

## Fetching token metadata

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_tokens_metadata

Token metadata can be fetched for specific tokens by providing their identifiers. This is especially useful for retrieving metadata for tokens that are not currently held in the wallet. The metadata is cached locally after the first fetch for faster subsequent lookups.

### Rust

```rust
let response = sdk
    .get_tokens_metadata(GetTokensMetadataRequest {
        token_identifiers: vec![
            String::from("<token identifier 1>"),
            String::from("<token identifier 2>"),
        ],
    })
    .await?;

let tokens_metadata = response.tokens_metadata;
for token_metadata in tokens_metadata {
    info!("Token ID: {}", token_metadata.identifier);
    info!("Name: {}", token_metadata.name);
    info!("Ticker: {}", token_metadata.ticker);
    info!("Decimals: {}", token_metadata.decimals);
    info!("Max Supply: {}", token_metadata.max_supply);
    info!("Is Freezable: {}", token_metadata.is_freezable);
}
```

### Swift

```swift
let response = try await sdk.getTokensMetadata(
    request: GetTokensMetadataRequest(tokenIdentifiers: [
        "<token identifier 1>", "<token identifier 2>",
    ]))

let tokensMetadata = response.tokensMetadata
for tokenMetadata in tokensMetadata {
    print("Token ID: \(tokenMetadata.identifier)")
    print("Name: \(tokenMetadata.name)")
    print("Ticker: \(tokenMetadata.ticker)")
    print("Decimals: \(tokenMetadata.decimals)")
    print("Max Supply: \(tokenMetadata.maxSupply)")
    print("Is Freezable: \(tokenMetadata.isFreezable)")
}
```

### Kotlin

```kotlin
try {
    val response = 
        sdk.getTokensMetadata(
            GetTokensMetadataRequest(
                tokenIdentifiers = listOf("<token identifier 1>", "<token identifier 2>")
        )
    )   

    val tokensMetadata = response.tokensMetadata
    for (tokenMetadata in tokensMetadata) {
        println("Token ID: ${tokenMetadata.identifier}")
        println("Name: ${tokenMetadata.name}")
        println("Ticker: ${tokenMetadata.ticker}")
        println("Decimals: ${tokenMetadata.decimals}")
        println("Max Supply: ${tokenMetadata.maxSupply}")
        println("Is Freezable: ${tokenMetadata.isFreezable}")
    }
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
var response = await sdk.GetTokensMetadata(
    request: new GetTokensMetadataRequest(
        tokenIdentifiers: new string[] { "<token identifier 1>", "<token identifier 2>" }
    )
);

var tokensMetadata = response.tokensMetadata;
foreach (var tokenMetadata in tokensMetadata)
{
    Console.WriteLine($"Token ID: {tokenMetadata.identifier}");
    Console.WriteLine($"Name: {tokenMetadata.name}");
    Console.WriteLine($"Ticker: {tokenMetadata.ticker}");
    Console.WriteLine($"Decimals: {tokenMetadata.decimals}");
    Console.WriteLine($"Max Supply: {tokenMetadata.maxSupply}");
    Console.WriteLine($"Is Freezable: {tokenMetadata.isFreezable}");
}
```

### Javascript (Wasm)

```typescript
const response = await sdk.getTokensMetadata({
  tokenIdentifiers: ['<token identifier 1>', '<token identifier 2>']
})

const tokensMetadata = response.tokensMetadata
for (const tokenMetadata of tokensMetadata) {
  console.log(`Token ID: ${tokenMetadata.identifier}`)
  console.log(`Name: ${tokenMetadata.name}`)
  console.log(`Ticker: ${tokenMetadata.ticker}`)
  console.log(`Decimals: ${tokenMetadata.decimals}`)
  console.log(`Max Supply: ${tokenMetadata.maxSupply}`)
  console.log(`Is Freezable: ${tokenMetadata.isFreezable}`)
}
```

### React Native

```typescript
const response = await sdk.getTokensMetadata({
  tokenIdentifiers: ['<token identifier 1>', '<token identifier 2>']
})

const tokensMetadata = response.tokensMetadata
for (const tokenMetadata of tokensMetadata) {
  console.log(`Token ID: ${tokenMetadata.identifier}`)
  console.log(`Name: ${tokenMetadata.name}`)
  console.log(`Ticker: ${tokenMetadata.ticker}`)
  console.log(`Decimals: ${tokenMetadata.decimals}`)
  console.log(`Max Supply: ${tokenMetadata.maxSupply}`)
  console.log(`Is Freezable: ${tokenMetadata.isFreezable}`)
}
```

### Flutter

```dart
final response = await sdk.getTokensMetadata(
  request: GetTokensMetadataRequest(
    tokenIdentifiers: ['<token identifier 1>', '<token identifier 2>']
    )
  );

final tokensMetadata = response.tokensMetadata;
for (final tokenMetadata in tokensMetadata) {
  print('Token ID: $tokenMetadata.identifier');
  print('Name: ${tokenMetadata.name}');
  print('Ticker: ${tokenMetadata.ticker}');
  print('Decimals: ${tokenMetadata.decimals}');
  print('Max Supply: ${tokenMetadata.maxSupply}');
  print('Is Freezable: ${tokenMetadata.isFreezable}');
}
```

### Python

```python
try:
    response = await sdk.get_tokens_metadata(
        request=GetTokensMetadataRequest(
            token_identifiers=["<token identifier 1>", "<token identifier 2>"]
            )
        )

    tokens_metadata = response.tokens_metadata
    for token_metadata in tokens_metadata:
        print(f"Token ID: {token_metadata.identifier}")
        print(f"Name: {token_metadata.name}")
        print(f"Ticker: {token_metadata.ticker}")
        print(f"Decimals: {token_metadata.decimals}")
        print(f"Max Supply: {token_metadata.max_supply}")
        print(f"Is Freezable: {token_metadata.is_freezable}")
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
tokenIdentifiers := []string{"<token identifier 1>", "<token identifier 2>"}
response, err := sdk.GetTokensMetadata(breez_sdk_spark.GetTokensMetadataRequest{
	TokenIdentifiers: tokenIdentifiers,
})

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}

tokensMetadata := response.TokensMetadata
for _, tokenMetadata := range tokensMetadata {
	log.Printf("Token ID: %v", tokenMetadata.Identifier)
	log.Printf("Name: %v", tokenMetadata.Name)
	log.Printf("Ticker: %v", tokenMetadata.Ticker)
	log.Printf("Decimals: %v", tokenMetadata.Decimals)
	log.Printf("Max Supply: %v", tokenMetadata.MaxSupply)
	log.Printf("Is Freezable: %v", tokenMetadata.IsFreezable)
}
```



## Receiving a token payment

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

Token payments can be received using either a Spark address or invoice. Using an invoice is useful to impose restrictions on the payment, such as the token to receive, amount, expiry, and who can pay it.

### Spark address

Token payments use the same Spark address as Bitcoin payments - no separate address is required. Your application can retrieve the Spark address as described in the [Receiving a payment](./receive_payment.md#spark) guide. The payer will use this address to send tokens to the wallet.

### Spark invoice

Spark token invoices can be created using the same API as Bitcoin Spark invoices. The only difference is that a token identifier is provided.

#### Rust

```rust
let token_identifier = Some("<token identifier>".to_string());
let optional_description = Some("<invoice description>".to_string());
let optional_amount = Some(5_000);
// Optionally set the expiry UNIX timestamp in seconds
let optional_expiry_time_seconds = Some(1716691200);
let optional_sender_public_key = Some("<sender public key>".to_string());

let response = sdk
    .receive_payment(ReceivePaymentRequest {
        payment_method: ReceivePaymentMethod::SparkInvoice {
            token_identifier,
            description: optional_description,
            amount: optional_amount,
            expiry_time: optional_expiry_time_seconds,
            sender_public_key: optional_sender_public_key,
        },
    })
    .await?;

let payment_request = response.payment_request;
info!("Payment request: {payment_request}");
let receive_fee = response.fee;
info!("Fees: {receive_fee} token base units");
```

#### Swift

```swift
let tokenIdentifier = "<token identifier>"
let optionalDescription = "<invoice description>"
let optionalAmount = BInt(5_000)
// Optionally set the expiry UNIX timestamp in seconds
let optionalExpiryTimeSeconds: UInt64 = 1_716_691_200
let optionalSenderPublicKey = "<sender public key>"

let response =
    try await sdk
    .receivePayment(
        request: ReceivePaymentRequest(
            paymentMethod: ReceivePaymentMethod.sparkInvoice(
                amount: optionalAmount,
                tokenIdentifier: tokenIdentifier,
                expiryTime: optionalExpiryTimeSeconds,
                description: optionalDescription,
                senderPublicKey: optionalSenderPublicKey
            )
        ))

let paymentRequest = response.paymentRequest
print("Payment request: \(paymentRequest)")
let receiveFeeSats = response.fee
print("Fees: \(receiveFeeSats) token base units")
```

#### Kotlin

```kotlin
try {
    val tokenIdentifier = "<token identifier>"
    val optionalDescription = "<invoice description>"
    // Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer, which is included in
    // package)
    val optionalAmount = BigInteger.fromLong(5_000L)
    // Android (BigInteger from java.math)
    // val optionalAmount = BigInteger.valueOf(5_000L)
    // Optionally set the expiry UNIX timestamp in seconds
    val optionalExpiryTimeSeconds = 1716691200.toULong()
    val optionalSenderPublicKey = "<sender public key>"

    val request = ReceivePaymentRequest(
        ReceivePaymentMethod.SparkInvoice(
            tokenIdentifier = tokenIdentifier,
            description = optionalDescription,
            amount = optionalAmount,
            expiryTime = optionalExpiryTimeSeconds,
            senderPublicKey = optionalSenderPublicKey
        )
    )
    val response = sdk.receivePayment(request)

    val paymentRequest = response.paymentRequest
    println("Payment request: $paymentRequest")
    val receiveFee = response.fee
    println("Fees: $receiveFee token base units")
} catch (e: Exception) {
    // handle error
}
```

#### C#

```csharp
var tokenIdentifier = "<token identifier>";
var optionalDescription = "<invoice description>";
var optionalAmount = new BigInteger(5000);
// Optionally set the expiry UNIX timestamp in seconds
var optionalExpiryTimeSeconds = 1716691200UL;
var optionalSenderPublicKey = "<sender public key>";

var request = new ReceivePaymentRequest(
    paymentMethod: new ReceivePaymentMethod.SparkInvoice(
        tokenIdentifier: tokenIdentifier,
        description: optionalDescription,
        amount: optionalAmount,
        expiryTime: optionalExpiryTimeSeconds,
        senderPublicKey: optionalSenderPublicKey
    )
);
var response = await sdk.ReceivePayment(request: request);

var paymentRequest = response.paymentRequest;
Console.WriteLine($"Payment request: {paymentRequest}");
var receiveFee = response.fee;
Console.WriteLine($"Fees: {receiveFee} token base units");
```

#### Javascript (Wasm)

```typescript
const tokenIdentifier = '<token identifier>'
const optionalDescription = '<invoice description>'
const optionalAmount = '5000'
// Optionally set the expiry UNIX timestamp in seconds
const optionalExpiryTimeSeconds = 1716691200
const optionalSenderPublicKey = '<sender public key>'

const response = await sdk.receivePayment({
  paymentMethod: {
    type: 'sparkInvoice',
    tokenIdentifier,
    description: optionalDescription,
    amount: optionalAmount,
    expiryTime: optionalExpiryTimeSeconds,
    senderPublicKey: optionalSenderPublicKey
  }
})

const paymentRequest = response.paymentRequest
console.log(`Payment request: ${paymentRequest}`)
const receiveFeeSats = response.fee
console.log(`Fees: ${receiveFeeSats} token base units`)
```

#### React Native

```typescript
const tokenIdentifier = '<token identifier>'
const optionalDescription = '<invoice description>'
const optionalAmount = BigInt(5_000)
// Optionally set the expiry UNIX timestamp in seconds
const optionalExpiryTimeSeconds = BigInt(1716691200)
const optionalSenderPublicKey = '<sender public key>'

const response = await sdk.receivePayment({
  paymentMethod: new ReceivePaymentMethod.SparkInvoice({
    tokenIdentifier,
    description: optionalDescription,
    amount: optionalAmount,
    expiryTime: optionalExpiryTimeSeconds,
    senderPublicKey: optionalSenderPublicKey
  })
})

const paymentRequest = response.paymentRequest
console.log(`Payment request: ${paymentRequest}`)
const receiveFee = response.fee
console.log(`Fees: ${receiveFee} token base units`)
```

#### Flutter

```dart
String tokenIdentifier = '<token identifier>';
String optionalDescription = "<invoice description>";
BigInt optionalAmount = BigInt.from(5000);
// Optionally set the expiry UNIX timestamp in seconds
BigInt optionalExpiryTimeSeconds = BigInt.from(1716691200);
String optionalSenderPublicKey = "<sender public key>"; 

ReceivePaymentRequest request =
    ReceivePaymentRequest(paymentMethod: ReceivePaymentMethod.sparkInvoice(
      tokenIdentifier: tokenIdentifier,
      description: optionalDescription,
      amount: optionalAmount,
      expiryTime: optionalExpiryTimeSeconds,
      senderPublicKey: optionalSenderPublicKey,
    ));
ReceivePaymentResponse response = await sdk.receivePayment(
  request: request,
);

String paymentRequest = response.paymentRequest;
print("Payment request: $paymentRequest");
BigInt receiveFee = response.fee;
print("Fees: $receiveFee token base units");
```

#### Python

```python
try:
    token_identifier = "<token identifier>"
    optional_description = "<invoice description>"
    optional_amount = 5_000
    # Optionally set the expiry UNIX timestamp in seconds
    optional_expiry_time_seconds = 1716691200
    optional_sender_public_key = "<sender public key>"

    request = ReceivePaymentRequest(
        payment_method=ReceivePaymentMethod.SPARK_INVOICE(
            token_identifier=token_identifier,
            description=optional_description,
            amount=optional_amount,
            expiry_time=optional_expiry_time_seconds,
            sender_public_key=optional_sender_public_key,
        )
    )
    response = await sdk.receive_payment(request=request)

    payment_request = response.payment_request
    print(f"Payment request: {payment_request}")
    receive_fee = response.fee
    print(f"Fees: {receive_fee} token base units")
    return response
except Exception as error:
    logging.error(error)
    raise
```

#### Go

```go
tokenIdentifier := "<token identifier>"
optionalDescription := "<invoice description>"
optionalAmount := new(big.Int).SetInt64(5_000)
// Optionally set the expiry UNIX timestamp in seconds
optionalExpiryTimeSeconds := uint64(1716691200)
optionalSenderPublicKey := "<sender public key>"

request := breez_sdk_spark.ReceivePaymentRequest{
	PaymentMethod: breez_sdk_spark.ReceivePaymentMethodSparkInvoice{
		TokenIdentifier: &tokenIdentifier,
		Description:     &optionalDescription,
		Amount:          &optionalAmount,
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
receiveFees := response.Fee
log.Printf("Fees: %v token base units", receiveFees)
```



## Sending a token payment

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_payment

To send tokens, provide a Spark address as the payment request. The token identifier must be specified in one of two ways:

1. **Using a Spark invoice**: If the payee provides a Spark address with an embedded token identifier and amount (a Spark invoice), the SDK automatically extracts and uses those values.
2. **Manual specification**: For a plain Spark address without embedded payment details, your application must provide both the token identifier and amount parameters when preparing the payment.

Your application can use the [parse](./parse.md) functionality to determine if a Spark address contains embedded token payment details before preparing the payment.

The code example below demonstrates manual specification. Follow the standard prepare/send payment flow as described in the [Sending a payment](./send_payment.md) guide.

**Developer note**

Payments can be sent without holding an asset by converting on-the-fly as a step before sending a payment. See <a href="./token_conversion.md">Converting tokens</a> for more information.

### Rust

```rust
let payment_request = "<spark address or invoice>".to_string();
// Token identifier must match the invoice in case it specifies one.
let token_identifier = Some("<token identifier>".to_string());
// Set the amount of tokens you wish to send (in token base units).
let amount = Some(1_000);

let prepare_response = sdk
    .prepare_send_payment(PrepareSendPaymentRequest {
        payment_request: PaymentRequest::Input {
            input: payment_request,
        },
        amount,
        token_identifier,
        conversion_options: None,
        fee_policy: None,
    })
    .await?;

// If the fees are acceptable, continue to send the token payment
match &prepare_response.payment_method {
    SendPaymentMethod::SparkAddress {
        fee,
        token_identifier: token_id,
        ..
    } => {
        info!("Token ID: {:?}", token_id);
        info!("Fees: {} token base units", fee);
    }
    SendPaymentMethod::SparkInvoice {
        fee,
        token_identifier: token_id,
        ..
    } => {
        info!("Token ID: {:?}", token_id);
        info!("Fees: {} token base units", fee);
    }
    _ => {}
}

// Send the token payment
let send_response = sdk
    .send_payment(SendPaymentRequest {
        prepare_response,
        options: None,
        idempotency_key: None,
    })
    .await?;
let payment = send_response.payment;
info!("Payment: {payment:?}");
```

### Swift

```swift
let paymentRequest = "<spark address or invoice>"
// Token identifier must match the invoice in case it specifies one.
let tokenIdentifier: String? = "<token identifier>"
// Set the amount of tokens you wish to send.
let amount: BInt? = BInt(1_000)

let prepareResponse = try await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
        paymentRequest: .input(input: paymentRequest),
        amount: amount,
        tokenIdentifier: tokenIdentifier,
        conversionOptions: nil,
        feePolicy: nil
    ))

// If the fees are acceptable, continue to send the token payment
if case let .sparkAddress(address, fee, tokenId) = prepareResponse.paymentMethod {
    print("Token ID: \(String(describing: tokenId))")
    print("Fees: \(fee) token base units")
}
if case let .sparkInvoice(invoice, fee, tokenId) = prepareResponse.paymentMethod {
    print("Token ID: \(String(describing: tokenId))")
    print("Fees: \(fee) token base units")
}

// Send the token payment
let sendResponse = try await sdk.sendPayment(
    request: SendPaymentRequest(
        prepareResponse: prepareResponse,
        options: nil
    ))
let payment = sendResponse.payment
print("Payment: \(payment)")
```

### Kotlin

```kotlin
try {
    val paymentRequest = "<spark address or invoice>"
    // Token identifier must match the invoice in case it specifies one.
    val tokenIdentifier = "<token identifier>"
    // Set the amount of tokens you wish to send (in token base units).
    // Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer)
    val amount = BigInteger.fromLong(1_000L)
    // Android (BigInteger from java.math)
    // val amount = BigInteger.valueOf(1_000L)

    val prepareResponse =
        sdk.prepareSendPayment(
            PrepareSendPaymentRequest(
                paymentRequest = PaymentRequest.Input(input = paymentRequest),
                amount = amount,
                tokenIdentifier = tokenIdentifier,
                conversionOptions = null,
                feePolicy = null,
            )
        )

    // If the fees are acceptable, continue to send the token payment
    when (val method = prepareResponse.paymentMethod) {
        is SendPaymentMethod.SparkAddress -> {
            println("Token ID: ${method.tokenIdentifier}")
            println("Fees: ${method.fee} token base units")
        }
        is SendPaymentMethod.SparkInvoice -> {
            println("Token ID: ${method.tokenIdentifier}")
            println("Fees: ${method.fee} token base units")
        }
        else -> {}
    }

    // Send the token payment
    val sendResponse =
        sdk.sendPayment(
            SendPaymentRequest(prepareResponse = prepareResponse, options = null)
        )
    val payment = sendResponse.payment
    println("Payment: $payment")
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
var paymentRequest = "<spark address or invoice>";
// Token identifier must match the invoice in case it specifies one.
var tokenIdentifier = "<token identifier>";
// Set the amount of tokens you wish to send.
ulong? amount = 1_000UL;

var prepareResponse = await sdk.PrepareSendPayment(
    request: new PrepareSendPaymentRequest(
        paymentRequest: new PaymentRequest.Input(input: paymentRequest),
        amount: amount,
        tokenIdentifier: tokenIdentifier,
        conversionOptions: null,
        feePolicy: null
    )
);

// If the fees are acceptable, continue to send the token payment
if (prepareResponse.paymentMethod is SendPaymentMethod.SparkAddress sparkAddress)
{
    Console.WriteLine($"Token ID: {sparkAddress.tokenIdentifier}");
    Console.WriteLine($"Fees: {sparkAddress.fee} token base units");
}
if (prepareResponse.paymentMethod is SendPaymentMethod.SparkInvoice sparkInvoice)
{
    Console.WriteLine($"Token ID: {sparkInvoice.tokenIdentifier}");
    Console.WriteLine($"Fees: {sparkInvoice.fee} token base units");
}

// Send the token payment
var sendResponse = await sdk.SendPayment(
    request: new SendPaymentRequest(
        prepareResponse: prepareResponse,
        options: null
    )
);
var payment = sendResponse.payment;
Console.WriteLine($"Payment: {payment}");
```

### Javascript (Wasm)

```typescript
const paymentRequest = '<spark address or invoice>'
// Token identifier must match the invoice in case it specifies one.
const tokenIdentifier = '<token identifier>'
// Set the amount of tokens you wish to send (in token base units).
const amount = BigInt(1_000)

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: { type: 'input', input: paymentRequest },
  amount,
  tokenIdentifier,
  conversionOptions: undefined,
  feePolicy: undefined
})

// If the fees are acceptable, continue to send the token payment
if (prepareResponse.paymentMethod.type === 'sparkAddress') {
  console.log(`Token ID: ${prepareResponse.paymentMethod.tokenIdentifier}`)
  console.log(`Fees: ${prepareResponse.paymentMethod.fee} token base units`)
}
if (prepareResponse.paymentMethod.type === 'sparkInvoice') {
  console.log(`Token ID: ${prepareResponse.paymentMethod.tokenIdentifier}`)
  console.log(`Fees: ${prepareResponse.paymentMethod.fee} token base units`)
}

// Send the token payment
const sendResponse = await sdk.sendPayment({
  prepareResponse,
  options: undefined
})
const payment = sendResponse.payment
console.log(`Payment: ${JSON.stringify(payment)}`)
```

### React Native

```typescript
const paymentRequest = '<spark address or invoice>'
// Token identifier must match the invoice in case it specifies one.
const tokenIdentifier = '<token identifier>'
// Set the amount of tokens you wish to send (in token base units).
const amount = BigInt(1_000)

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: new PaymentRequest.Input({ input: paymentRequest }),
  amount,
  tokenIdentifier,
  conversionOptions: undefined,
  feePolicy: undefined
})

// If the fees are acceptable, continue to send the token payment
if (prepareResponse.paymentMethod?.tag === SendPaymentMethod_Tags.SparkAddress) {
  console.log(`Token ID: ${prepareResponse.paymentMethod.inner.tokenIdentifier}`)
  console.log(`Fees: ${prepareResponse.paymentMethod.inner.fee} token base units`)
}
if (prepareResponse.paymentMethod?.tag === SendPaymentMethod_Tags.SparkInvoice) {
  console.log(`Token ID: ${prepareResponse.paymentMethod.inner.tokenIdentifier}`)
  console.log(`Fees: ${prepareResponse.paymentMethod.inner.fee} token base units`)
}

// Send the token payment
const sendResponse = await sdk.sendPayment({
  prepareResponse,
  options: undefined,
  idempotencyKey: undefined
})
const payment = sendResponse.payment
console.log(`Payment: ${JSON.stringify(payment)}`)
```

### Flutter

```dart
final paymentRequest = '<spark address or invoice>';
// Token identifier must match the invoice in case it specifies one.
final tokenIdentifier = '<token identifier>';
// Set the amount of tokens you wish to send (in token base units).
final amount = BigInt.from(1000);

final prepareResponse = await sdk.prepareSendPayment(
  request: PrepareSendPaymentRequest(
    paymentRequest: PaymentRequest.input(input: paymentRequest),
    amount: amount,
    tokenIdentifier: tokenIdentifier,
    conversionOptions: null,
    feePolicy: null,
  ),
);

// If the fees are acceptable, continue to send the token payment
if (prepareResponse.paymentMethod is SendPaymentMethod_SparkAddress) {
  final method = prepareResponse.paymentMethod as SendPaymentMethod_SparkAddress;
  print('Token ID: ${method.tokenIdentifier}');
  print('Fees: ${method.fee} token base units');
}
if (prepareResponse.paymentMethod is SendPaymentMethod_SparkInvoice) {
  final method = prepareResponse.paymentMethod as SendPaymentMethod_SparkInvoice;
  print('Token ID: ${method.tokenIdentifier}');
  print('Fees: ${method.fee} token base units');
}

// Send the token payment
final sendResponse = await sdk.sendPayment(
  request: SendPaymentRequest(
    prepareResponse: prepareResponse,
    options: null,
  ),
);
final payment = sendResponse.payment;
print('Payment: $payment');
```

### Python

```python
try:
    payment_request = "<spark address or invoice>"
    token_identifier = "<token identifier>"
    amount = 1_000

    prepare_response = await sdk.prepare_send_payment(
        request=PrepareSendPaymentRequest(
            payment_request=PaymentRequest.INPUT(input=payment_request),
            amount=amount,
            token_identifier=token_identifier,
            conversion_options=None,
            fee_policy=None,
        )
    )

    # If the fees are acceptable, continue to send the token payment
    if isinstance(prepare_response.payment_method, SendPaymentMethod.SPARK_ADDRESS):
        print(f"Token ID: {prepare_response.payment_method.token_identifier}")
        print(f"Fees: {prepare_response.payment_method.fee} token base units")
    if isinstance(prepare_response.payment_method, SendPaymentMethod.SPARK_INVOICE):
        print(f"Token ID: {prepare_response.payment_method.token_identifier}")
        print(f"Fees: {prepare_response.payment_method.fee} token base units")

    # Send the token payment
    send_response = await sdk.send_payment(
        request=SendPaymentRequest(
            prepare_response=prepare_response,
            options=None,
        )
    )
    payment = send_response.payment
    print(f"Payment: {payment}")
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
paymentRequest := "<spark address or invoice>"
// Token identifier must match the invoice in case it specifies one.
tokenIdentifier := "<token identifier>"
// Set the amount of tokens you wish to send.
amount := new(big.Int).SetInt64(1_000)

prepareResponse, err := sdk.PrepareSendPayment(breez_sdk_spark.PrepareSendPaymentRequest{
	PaymentRequest:    breez_sdk_spark.PaymentRequestInput{Input: paymentRequest},
	Amount:            &amount,
	TokenIdentifier:   &tokenIdentifier,
	ConversionOptions: nil,
	FeePolicy:         nil,
})

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}

// If the fees are acceptable, continue to send the token payment
switch method := prepareResponse.PaymentMethod.(type) {
case breez_sdk_spark.SendPaymentMethodSparkAddress:
	log.Printf("Token ID: %v", method.TokenIdentifier)
	log.Printf("Fees: %v token base units", method.Fee)
case breez_sdk_spark.SendPaymentMethodSparkInvoice:
	log.Printf("Token ID: %v", method.TokenIdentifier)
	log.Printf("Fees: %v token base units", method.Fee)
}

// Send the token payment
sendResponse, err := sdk.SendPayment(breez_sdk_spark.SendPaymentRequest{
	PrepareResponse: prepareResponse,
	Options:         nil,
})

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}

payment := sendResponse.Payment
log.Printf("Payment: %#v", payment)
```



To pay several recipients at once, see [Sending to multiple recipients](./batch_send.md): one transaction can pay multiple payees, across several tokens, mixing Spark addresses and invoices. A batch that pays an invoice stays on one token.

## Listing token payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

Token payments are included in the regular payment history alongside Bitcoin payments. Your application can retrieve and distinguish token payments from other payment types using the standard payment listing functionality. See the [Listing payments](./list_payments.md) guide for more details.