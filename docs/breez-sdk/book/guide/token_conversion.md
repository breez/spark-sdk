# Converting tokens

Token conversion enables payments to be made without holding the required asset by converting on-the-fly between Bitcoin and tokens using the Flashnet protocol.

## Fetching conversion limits

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.fetch_conversion_limits

Before performing a conversion, you can fetch the minimum amounts required for the conversion. The limits depend on the conversion direction:

- **Bitcoin to token**: Minimum Bitcoin amount (in satoshis) and minimum token amount to receive (in token base units)
- **Token to Bitcoin**: Minimum token amount (in token base units) and minimum Bitcoin amount to receive (in satoshis)

### Rust

```rust
// Fetch limits for converting Bitcoin to a token
let response = sdk
    .fetch_conversion_limits(FetchConversionLimitsRequest {
        conversion_type: ConversionType::FromBitcoin,
        token_identifier: Some("<token identifier>".to_string()),
    })
    .await?;

if let Some(min_from) = response.min_from_amount {
    info!("Minimum BTC to convert: {} sats", min_from);
}
if let Some(min_to) = response.min_to_amount {
    info!("Minimum tokens to receive: {} base units", min_to);
}

// Fetch limits for converting a token to Bitcoin
let response = sdk
    .fetch_conversion_limits(FetchConversionLimitsRequest {
        conversion_type: ConversionType::ToBitcoin {
            from_token_identifier: "<token identifier>".to_string(),
        },
        token_identifier: None,
    })
    .await?;

if let Some(min_from) = response.min_from_amount {
    info!("Minimum tokens to convert: {} base units", min_from);
}
if let Some(min_to) = response.min_to_amount {
    info!("Minimum BTC to receive: {} sats", min_to);
}
```

### Swift

```swift
// Fetch limits for converting Bitcoin to a token
let fromBitcoinResponse = try await sdk.fetchConversionLimits(
    request: FetchConversionLimitsRequest(
        conversionType: ConversionType.fromBitcoin,
        tokenIdentifier: "<token identifier>"
    ))

if let minFrom = fromBitcoinResponse.minFromAmount {
    print("Minimum BTC to convert: \(minFrom) sats")
}
if let minTo = fromBitcoinResponse.minToAmount {
    print("Minimum tokens to receive: \(minTo) base units")
}

// Fetch limits for converting a token to Bitcoin
let toBitcoinResponse = try await sdk.fetchConversionLimits(
    request: FetchConversionLimitsRequest(
        conversionType: ConversionType.toBitcoin(
            fromTokenIdentifier: "<token identifier>"
        ),
        tokenIdentifier: nil
    ))

if let minFrom = toBitcoinResponse.minFromAmount {
    print("Minimum tokens to convert: \(minFrom) base units")
}
if let minTo = toBitcoinResponse.minToAmount {
    print("Minimum BTC to receive: \(minTo) sats")
}
```

### Kotlin

```kotlin
try {
    // Fetch limits for converting Bitcoin to a token
    val fromBitcoinResponse = sdk.fetchConversionLimits(
        FetchConversionLimitsRequest(
            conversionType = ConversionType.FromBitcoin,
            tokenIdentifier = "<token identifier>"
        )
    )

    if (fromBitcoinResponse.minFromAmount != null) {
        println("Minimum BTC to convert: ${fromBitcoinResponse.minFromAmount} sats")
    }
    if (fromBitcoinResponse.minToAmount != null) {
        println("Minimum tokens to receive: ${fromBitcoinResponse.minToAmount} base units")
    }

    // Fetch limits for converting a token to Bitcoin
    val toBitcoinResponse = sdk.fetchConversionLimits(
        FetchConversionLimitsRequest(
            conversionType = ConversionType.ToBitcoin(
                fromTokenIdentifier = "<token identifier>"
            ),
            tokenIdentifier = null
        )
    )

    if (toBitcoinResponse.minFromAmount != null) {
        println("Minimum tokens to convert: ${toBitcoinResponse.minFromAmount} base units")
    }
    if (toBitcoinResponse.minToAmount != null) {
        println("Minimum BTC to receive: ${toBitcoinResponse.minToAmount} sats")
    }
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
// Fetch limits for converting Bitcoin to a token
var fromBitcoinResponse = await sdk.FetchConversionLimits(
    request: new FetchConversionLimitsRequest(
        conversionType: new ConversionType.FromBitcoin(),
        tokenIdentifier: "<token identifier>"
    )
);

if (fromBitcoinResponse.minFromAmount != null)
{
    Console.WriteLine($"Minimum BTC to convert: {fromBitcoinResponse.minFromAmount} sats");
}
if (fromBitcoinResponse.minToAmount != null)
{
    Console.WriteLine($"Minimum tokens to receive: {fromBitcoinResponse.minToAmount} base units");
}

// Fetch limits for converting a token to Bitcoin
var toBitcoinResponse = await sdk.FetchConversionLimits(
    request: new FetchConversionLimitsRequest(
        conversionType: new ConversionType.ToBitcoin(
            fromTokenIdentifier: "<token identifier>"
        ),
        tokenIdentifier: null
    )
);

if (toBitcoinResponse.minFromAmount != null)
{
    Console.WriteLine($"Minimum tokens to convert: {toBitcoinResponse.minFromAmount} base units");
}
if (toBitcoinResponse.minToAmount != null)
{
    Console.WriteLine($"Minimum BTC to receive: {toBitcoinResponse.minToAmount} sats");
}
```

### Javascript (Wasm)

```typescript
// Fetch limits for converting Bitcoin to a token
const fromBitcoinResponse = await sdk.fetchConversionLimits({
  conversionType: { type: 'fromBitcoin' },
  tokenIdentifier: '<token identifier>'
})

if (fromBitcoinResponse.minFromAmount !== undefined) {
  console.log(`Minimum BTC to convert: ${fromBitcoinResponse.minFromAmount} sats`)
}
if (fromBitcoinResponse.minToAmount !== undefined) {
  console.log(`Minimum tokens to receive: ${fromBitcoinResponse.minToAmount} base units`)
}

// Fetch limits for converting a token to Bitcoin
const toBitcoinResponse = await sdk.fetchConversionLimits({
  conversionType: {
    type: 'toBitcoin',
    fromTokenIdentifier: '<token identifier>'
  },
  tokenIdentifier: undefined
})

if (toBitcoinResponse.minFromAmount !== undefined) {
  console.log(`Minimum tokens to convert: ${toBitcoinResponse.minFromAmount} base units`)
}
if (toBitcoinResponse.minToAmount !== undefined) {
  console.log(`Minimum BTC to receive: ${toBitcoinResponse.minToAmount} sats`)
}
```

### React Native

```typescript
// Fetch limits for converting Bitcoin to a token
const fromBitcoinResponse = await sdk.fetchConversionLimits({
  conversionType: new ConversionType.FromBitcoin(),
  tokenIdentifier: '<token identifier>'
})

if (fromBitcoinResponse.minFromAmount !== undefined) {
  console.log(`Minimum BTC to convert: ${fromBitcoinResponse.minFromAmount} sats`)
}
if (fromBitcoinResponse.minToAmount !== undefined) {
  console.log(`Minimum tokens to receive: ${fromBitcoinResponse.minToAmount} base units`)
}

// Fetch limits for converting a token to Bitcoin
const toBitcoinResponse = await sdk.fetchConversionLimits({
  conversionType: new ConversionType.ToBitcoin({
    fromTokenIdentifier: '<token identifier>'
  }),
  tokenIdentifier: undefined
})

if (toBitcoinResponse.minFromAmount !== undefined) {
  console.log(`Minimum tokens to convert: ${toBitcoinResponse.minFromAmount} base units`)
}
if (toBitcoinResponse.minToAmount !== undefined) {
  console.log(`Minimum BTC to receive: ${toBitcoinResponse.minToAmount} sats`)
}
```

### Flutter

```dart
// Fetch limits for converting Bitcoin to a token
final fromBitcoinResponse = await sdk.fetchConversionLimits(
  request: FetchConversionLimitsRequest(
    conversionType: ConversionType.fromBitcoin(),
    tokenIdentifier: '<token identifier>',
  ),
);

if (fromBitcoinResponse.minFromAmount != null) {
  print('Minimum BTC to convert: ${fromBitcoinResponse.minFromAmount} sats');
}
if (fromBitcoinResponse.minToAmount != null) {
  print('Minimum tokens to receive: ${fromBitcoinResponse.minToAmount} base units');
}

// Fetch limits for converting a token to Bitcoin
final toBitcoinResponse = await sdk.fetchConversionLimits(
  request: FetchConversionLimitsRequest(
    conversionType: ConversionType.toBitcoin(
      fromTokenIdentifier: '<token identifier>',
    ),
    tokenIdentifier: null,
  ),
);

if (toBitcoinResponse.minFromAmount != null) {
  print('Minimum tokens to convert: ${toBitcoinResponse.minFromAmount} base units');
}
if (toBitcoinResponse.minToAmount != null) {
  print('Minimum BTC to receive: ${toBitcoinResponse.minToAmount} sats');
}
```

### Python

```python
try:
    # Fetch limits for converting Bitcoin to a token
    from_bitcoin_response = await sdk.fetch_conversion_limits(
        request=FetchConversionLimitsRequest(
            conversion_type=ConversionType.FROM_BITCOIN(),
            token_identifier="<token identifier>",
        )
    )

    if from_bitcoin_response.min_from_amount is not None:
        print(f"Minimum BTC to convert: {from_bitcoin_response.min_from_amount} sats")
    if from_bitcoin_response.min_to_amount is not None:
        print(f"Minimum tokens to receive: {from_bitcoin_response.min_to_amount} base units")

    # Fetch limits for converting a token to Bitcoin
    to_bitcoin_response = await sdk.fetch_conversion_limits(
        request=FetchConversionLimitsRequest(
            conversion_type=ConversionType.TO_BITCOIN(
                from_token_identifier="<token identifier>"
            ),
            token_identifier=None,
        )
    )

    if to_bitcoin_response.min_from_amount is not None:
        print(f"Minimum tokens to convert: {to_bitcoin_response.min_from_amount} base units")
    if to_bitcoin_response.min_to_amount is not None:
        print(f"Minimum BTC to receive: {to_bitcoin_response.min_to_amount} sats")
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
// Fetch limits for converting Bitcoin to a token
tokenIdentifier := "<token identifier>"
fromBitcoinResponse, err := sdk.FetchConversionLimits(breez_sdk_spark.FetchConversionLimitsRequest{
	ConversionType:  breez_sdk_spark.ConversionTypeFromBitcoin{},
	TokenIdentifier: &tokenIdentifier,
})

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}

if fromBitcoinResponse.MinFromAmount != nil {
	log.Printf("Minimum BTC to convert: %v sats", *fromBitcoinResponse.MinFromAmount)
}
if fromBitcoinResponse.MinToAmount != nil {
	log.Printf("Minimum tokens to receive: %v base units", *fromBitcoinResponse.MinToAmount)
}

// Fetch limits for converting a token to Bitcoin
fromTokenIdentifier := "<token identifier>"
toBitcoinResponse, err := sdk.FetchConversionLimits(breez_sdk_spark.FetchConversionLimitsRequest{
	ConversionType: breez_sdk_spark.ConversionTypeToBitcoin{
		FromTokenIdentifier: fromTokenIdentifier,
	},
	TokenIdentifier: nil,
})

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}

if toBitcoinResponse.MinFromAmount != nil {
	log.Printf("Minimum tokens to convert: %v base units", *toBitcoinResponse.MinFromAmount)
}
if toBitcoinResponse.MinToAmount != nil {
	log.Printf("Minimum BTC to receive: %v sats", *toBitcoinResponse.MinToAmount)
}
```



**Developer note**

Amounts are denominated in satoshis for Bitcoin (1 BTC = 100,000,000 sats) and in token base units for tokens. Token base units depend on the token's decimal specification.

## Converting Bitcoin to tokens

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_payment

Token conversion enables payments of tokens like <a href="https://sparkscan.io/token/3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca?network=mainnet" target="_blank">USDB</a> to be made without holding the token, but instead using Bitcoin.

To do so, when preparing to send a payment, set the conversion options. The conversion will first calculate the Bitcoin amount needed to be converted into the token, convert Bitcoin into that token amount, and then finally complete the payment.

### Rust

```rust
let payment_request = "<spark address or invoice>".to_string();
// Token identifier must match the invoice in case it specifies one.
let token_identifier = Some("<token identifier>".to_string());
// Set the amount of tokens you wish to send (in token base units).
let amount = Some(1_000);
// Set to use Bitcoin funds to pay via conversion
let optional_max_slippage_bps = Some(50);
let optional_completion_timeout_secs = Some(30);
let conversion_options = Some(ConversionOptions {
    conversion_type: ConversionType::FromBitcoin,
    max_slippage_bps: optional_max_slippage_bps,
    completion_timeout_secs: optional_completion_timeout_secs,
});

let prepare_response = sdk
    .prepare_send_payment(PrepareSendPaymentRequest {
        payment_request: PaymentRequest::Input {
            input: payment_request,
        },
        amount,
        token_identifier,
        conversion_options,
        fee_policy: None,
    })
    .await?;

// If the fees are acceptable, continue to send the token payment
if let Some(conversion_estimate) = &prepare_response.conversion_estimate {
    info!(
        "Estimated conversion: {} token units → {} sats",
        conversion_estimate.amount_in, conversion_estimate.amount_out
    );
    info!(
        "Estimated conversion fee: {} token units",
        conversion_estimate.fee
    );
}
```

### Swift

```swift
let paymentRequest = "<spark address or invoice>"
// Token identifier must match the invoice in case it specifies one.
let tokenIdentifier: String? = "<token identifier>"
// Set the amount of tokens you wish to send.
let amount: BInt? = BInt(1_000)
// Set to use Bitcoin funds to pay via conversion
let optionalMaxSlippageBps = UInt32(50)
let optionalCompletionTimeoutSecs = UInt32(30)
let conversionOptions = ConversionOptions(
    conversionType: ConversionType.fromBitcoin,
    maxSlippageBps: optionalMaxSlippageBps,
    completionTimeoutSecs: optionalCompletionTimeoutSecs
)

let prepareResponse = try await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
        paymentRequest: .input(input: paymentRequest),
        amount: amount,
        tokenIdentifier: tokenIdentifier,
        conversionOptions: conversionOptions,
        feePolicy: nil
    ))

// If the fees are acceptable, continue to send the token payment
if let conversionEstimate = prepareResponse.conversionEstimate {
    print(
        "Estimated conversion: \(conversionEstimate.amountIn) token units "
            + "→ \(conversionEstimate.amountOut) sats")
    print("Estimated conversion fee: \(conversionEstimate.fee) token units")
}
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
    // Set to use Bitcoin funds to pay via conversion
    val optionalMaxSlippageBps = 50u
    val optionalCompletionTimeoutSecs = 30u
    val conversionOptions = ConversionOptions(
        conversionType = ConversionType.FromBitcoin,
        maxSlippageBps = optionalMaxSlippageBps,
        completionTimeoutSecs = optionalCompletionTimeoutSecs
    )

    val prepareResponse =
        sdk.prepareSendPayment(
            PrepareSendPaymentRequest(
                paymentRequest = PaymentRequest.Input(input = paymentRequest),
                amount = amount,
                tokenIdentifier = tokenIdentifier,
                conversionOptions = conversionOptions,
                feePolicy = null,
            )
        )

    // If the fees are acceptable, continue to send the token payment
    prepareResponse.conversionEstimate?.let { conversionEstimate ->
        println(
            "Estimated conversion: ${conversionEstimate.amountIn} token units " +
                "→ ${conversionEstimate.amountOut} sats"
        )
        println("Estimated conversion fee: ${conversionEstimate.fee} token units")
    }
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
// Optionally set to use Bitcoin funds to pay via conversion
var optionalMaxSlippageBps = 50U;
var optionalCompletionTimeoutSecs = 30U;
var conversionOptions = new ConversionOptions(
    conversionType: new ConversionType.FromBitcoin(),
    maxSlippageBps: optionalMaxSlippageBps,
    completionTimeoutSecs: optionalCompletionTimeoutSecs
);

var prepareResponse = await sdk.PrepareSendPayment(
    request: new PrepareSendPaymentRequest(
        paymentRequest: new PaymentRequest.Input(input: paymentRequest),
        amount: amount,
        tokenIdentifier: tokenIdentifier,
        conversionOptions: conversionOptions,
        feePolicy: null
    )
);

// If the fees are acceptable, continue to send the token payment
if (prepareResponse.conversionEstimate != null)
{
    Console.WriteLine("Estimated conversion: " +
        $"{prepareResponse.conversionEstimate.amountIn} token units " +
        $"→ {prepareResponse.conversionEstimate.amountOut} sats");
    Console.WriteLine("Estimated conversion fee: " +
        $"{prepareResponse.conversionEstimate.fee} token units");
}
```

### Javascript (Wasm)

```typescript
const paymentRequest = '<spark address or invoice>'
// Token identifier must match the invoice in case it specifies one.
const tokenIdentifier = '<token identifier>'
// Set the amount of tokens you wish to send (in token base units).
const amount = BigInt(1_000)
// Set to use Bitcoin funds to pay via conversion
const optionalMaxSlippageBps = 50
const optionalCompletionTimeoutSecs = 30
const conversionOptions: ConversionOptions = {
  conversionType: {
    type: 'fromBitcoin'
  },
  maxSlippageBps: optionalMaxSlippageBps,
  completionTimeoutSecs: optionalCompletionTimeoutSecs
}

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: { type: 'input', input: paymentRequest },
  amount,
  tokenIdentifier,
  conversionOptions,
  feePolicy: undefined
})

// If the fees are acceptable, continue to send the token payment
if (prepareResponse.conversionEstimate !== undefined) {
  const conversionEstimate = prepareResponse.conversionEstimate
  console.log(
    `Estimated conversion: ${conversionEstimate.amountIn} token units → ` +
    `${conversionEstimate.amountOut} sats`
  )
  console.log(`Estimated conversion fee: ${conversionEstimate.fee} token units`)
}
```

### React Native

```typescript
const paymentRequest = '<spark address or invoice>'
// Token identifier must match the invoice in case it specifies one.
const tokenIdentifier = '<token identifier>'
// Set the amount of tokens you wish to send (in token base units).
const amount = BigInt(1_000)
// Set to use Bitcoin funds to pay via conversion
const optionalMaxSlippageBps = 50
const optionalCompletionTimeoutSecs = 30
const conversionOptions = {
  conversionType: new ConversionType.FromBitcoin(),
  maxSlippageBps: optionalMaxSlippageBps,
  completionTimeoutSecs: optionalCompletionTimeoutSecs
}

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: new PaymentRequest.Input({ input: paymentRequest }),
  amount,
  tokenIdentifier,
  conversionOptions,
  feePolicy: undefined
})

// If the fees are acceptable, continue to send the token payment
if (prepareResponse.conversionEstimate !== undefined) {
  const conversionEstimate = prepareResponse.conversionEstimate
  console.debug(
    `Estimated conversion: ${conversionEstimate.amountIn} token units → ` +
    `${conversionEstimate.amountOut} sats`
  )
  console.debug(`Estimated conversion fee: ${conversionEstimate.fee} token units`)
}
```

### Flutter

```dart
final paymentRequest = '<spark address or invoice>';
// Token identifier must match the invoice in case it specifies one.
final tokenIdentifier = '<token identifier>';
// Set the amount of tokens you wish to send (in token base units).
final amount = BigInt.from(1000);
// Set to use Bitcoin funds to pay via conversion
int optionalMaxSlippageBps = 50;
int optionalCompletionTimeoutSecs = 30;
final conversionOptions = ConversionOptions(
  conversionType: ConversionType.fromBitcoin(),
  maxSlippageBps: optionalMaxSlippageBps,
  completionTimeoutSecs: optionalCompletionTimeoutSecs,
);

final prepareResponse = await sdk.prepareSendPayment(
  request: PrepareSendPaymentRequest(
    paymentRequest: PaymentRequest.input(input: paymentRequest),
    amount: amount,
    tokenIdentifier: tokenIdentifier,
    conversionOptions: conversionOptions,
    feePolicy: null,
  ),
);

// If the fees are acceptable, continue to send the token payment
if (prepareResponse.conversionEstimate != null) {
  print(
      "Estimated conversion: ${prepareResponse.conversionEstimate!.amountIn} token units "
      "→ ${prepareResponse.conversionEstimate!.amountOut} sats");
  print(
      "Estimated conversion fee: ${prepareResponse.conversionEstimate!.fee} token units");
}
```

### Python

```python
try:
    payment_request = "<spark address or invoice>"
    token_identifier = "<token identifier>"
    amount = 1_000
    # Set to use Bitcoin funds to pay via conversion
    optional_max_slippage_bps = 50
    optional_completion_timeout_secs = 30
    conversion_options = ConversionOptions(
        conversion_type=ConversionType.FROM_BITCOIN(),
        max_slippage_bps=optional_max_slippage_bps,
        completion_timeout_secs=optional_completion_timeout_secs,
    )

    prepare_response = await sdk.prepare_send_payment(
        request=PrepareSendPaymentRequest(
            payment_request=PaymentRequest.INPUT(input=payment_request),
            amount=amount,
            token_identifier=token_identifier,
            conversion_options=conversion_options,
            fee_policy=None,
        )
    )

    # If the fees are acceptable, continue to send the token payment
    if prepare_response.conversion_estimate is not None:
        conversion_estimate = prepare_response.conversion_estimate
        logging.debug(
            f"Estimated conversion: {conversion_estimate.amount_in}"
            f" token units → {conversion_estimate.amount_out} sats"
        )
        logging.debug(
            f"Estimated conversion fee: {conversion_estimate.fee} token units"
        )
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

// Set to use Bitcoin funds to pay via conversion
optionalMaxSlippageBps := uint32(50)
optionalCompletionTimeoutSecs := uint32(30)
conversionOptions := &breez_sdk_spark.ConversionOptions{
	ConversionType:        breez_sdk_spark.ConversionTypeToBitcoin{},
	MaxSlippageBps:        &optionalMaxSlippageBps,
	CompletionTimeoutSecs: &optionalCompletionTimeoutSecs,
}

prepareResponse, err := sdk.PrepareSendPayment(breez_sdk_spark.PrepareSendPaymentRequest{
	PaymentRequest:    breez_sdk_spark.PaymentRequestInput{Input: paymentRequest},
	Amount:            &amount,
	TokenIdentifier:   &tokenIdentifier,
	ConversionOptions: conversionOptions,
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
if prepareResponse.ConversionEstimate != nil {
	log.Printf(
		"Estimated conversion: %v token units → %v sats",
		prepareResponse.ConversionEstimate.AmountIn,
		prepareResponse.ConversionEstimate.AmountOut,
	)
	log.Printf("Estimated conversion fee: %v token units", prepareResponse.ConversionEstimate.Fee)
}
```



**Developer note**

When a conversion fails due to exceeding the maximum slippage, the conversion will be refunded automatically.

**Developer note**

The conversion may result in some token balance remaining in the wallet after the payment is sent. This remaining balance is to account for slippage in the conversion.

## Converting tokens to Bitcoin

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_payment

Token conversion also enables Bitcoin payments to be made without holding the required Bitcoin, but instead using a supported token asset like <a href="https://sparkscan.io/token/3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca?network=mainnet" target="_blank">USDB</a>.

To do so, when preparing to send a payment, set the conversion options. The conversion will first calculate the amount needed to be converted into Bitcoin, convert the token into that Bitcoin amount, and then finally complete the payment.

### Rust

```rust
let payment_request = "<payment request>".to_string();
// Set to use token funds to pay via conversion
let optional_max_slippage_bps = Some(50);
let optional_completion_timeout_secs = Some(30);
let conversion_options = Some(ConversionOptions {
    conversion_type: ConversionType::ToBitcoin {
        from_token_identifier: "<token identifier>".to_string(),
    },
    max_slippage_bps: optional_max_slippage_bps,
    completion_timeout_secs: optional_completion_timeout_secs,
});

let prepare_response = sdk
    .prepare_send_payment(PrepareSendPaymentRequest {
        payment_request: PaymentRequest::Input {
            input: payment_request,
        },
        amount: None,
        token_identifier: None,
        conversion_options,
        fee_policy: None,
    })
    .await?;

// If the fees are acceptable, continue to create the Send Payment
if let Some(conversion_estimate) = &prepare_response.conversion_estimate {
    info!(
        "Estimated conversion: {} token units → {} sats",
        conversion_estimate.amount_in, conversion_estimate.amount_out
    );
    info!(
        "Estimated conversion fee: {} token units",
        conversion_estimate.fee
    );
}
```

### Swift

```swift
let paymentRequest = "<payment request>"
// Set to use token funds to pay via conversion
let optionalMaxSlippageBps = UInt32(50)
let optionalCompletionTimeoutSecs = UInt32(30)
let conversionOptions = ConversionOptions(
    conversionType: ConversionType.toBitcoin(
        fromTokenIdentifier: "<token identifier>"
    ),
    maxSlippageBps: optionalMaxSlippageBps,
    completionTimeoutSecs: optionalCompletionTimeoutSecs
)

let prepareResponse = try await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
        paymentRequest: .input(input: paymentRequest),
        amount: nil,
        tokenIdentifier: nil,
        conversionOptions: conversionOptions,
        feePolicy: nil
    ))

if let conversionEstimate = prepareResponse.conversionEstimate {
    print(
        "Estimated conversion: \(conversionEstimate.amountIn) token units "
            + "→ \(conversionEstimate.amountOut) sats")
    print("Estimated conversion fee: \(conversionEstimate.fee) token units")
}
```

### Kotlin

```kotlin
val paymentRequest = "<payment request>"
// Set to use token funds to pay via conversion
val optionalMaxSlippageBps = 50u
val optionalCompletionTimeoutSecs = 30u
val conversionOptions = ConversionOptions(
    conversionType = ConversionType.ToBitcoin(
        "<token identifier>"
    ),
    maxSlippageBps = optionalMaxSlippageBps,
    completionTimeoutSecs = optionalCompletionTimeoutSecs
)

try {
    val req = PrepareSendPaymentRequest(
        paymentRequest = PaymentRequest.Input(input = paymentRequest),
        amount = null,
        tokenIdentifier = null,
        conversionOptions = conversionOptions,
        feePolicy = null,
    )
    val prepareResponse = sdk.prepareSendPayment(req)

    // If the fees are acceptable, continue to create the Send Payment
    prepareResponse.conversionEstimate?.let { conversionEstimate ->
        // Log.v("Breez", "Estimated conversion: ${conversionEstimate.amountIn}
        // token units → ${conversionEstimate.amountOut} sats")
        // Log.v("Breez", "Estimated conversion fee: ${conversionEstimate.fee} token units")
    }
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
var paymentRequest = "<payment request>";
// Set to use token funds to pay via conversion
var optionalMaxSlippageBps = 50U;
var optionalCompletionTimeoutSecs = 30U;
var conversionOptions = new ConversionOptions(
    conversionType: new ConversionType.ToBitcoin(
        fromTokenIdentifier: "<token identifier>"
    ),
    maxSlippageBps: optionalMaxSlippageBps,
    completionTimeoutSecs: optionalCompletionTimeoutSecs
);

var request = new PrepareSendPaymentRequest(
    paymentRequest: new PaymentRequest.Input(input: paymentRequest),
    amount: null,
    tokenIdentifier: null,
    conversionOptions: conversionOptions,
    feePolicy: null
);
var prepareResponse = await sdk.PrepareSendPayment(request: request);

// If the fees are acceptable, continue to create the Send Payment
if (prepareResponse.conversionEstimate != null)
{
    Console.WriteLine("Estimated conversion: " +
        $"{prepareResponse.conversionEstimate.amountIn} token units " +
        $"→ {prepareResponse.conversionEstimate.amountOut} sats");
    Console.WriteLine("Estimated conversion fee: " +
        $"{prepareResponse.conversionEstimate.fee} token units");
}
```

### Javascript (Wasm)

```typescript
const paymentRequest = '<payment request>'
// Set to use token funds to pay via conversion
const optionalMaxSlippageBps = 50
const optionalCompletionTimeoutSecs = 30
const conversionOptions: ConversionOptions = {
  conversionType: {
    type: 'toBitcoin',
    fromTokenIdentifier: '<token identifier>'
  },
  maxSlippageBps: optionalMaxSlippageBps,
  completionTimeoutSecs: optionalCompletionTimeoutSecs
}

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: { type: 'input', input: paymentRequest },
  amount: undefined,
  tokenIdentifier: undefined,
  conversionOptions,
  feePolicy: undefined
})

// If the fees are acceptable, continue to create the Send Payment
if (prepareResponse.conversionEstimate !== undefined) {
  const conversionEstimate = prepareResponse.conversionEstimate
  console.debug(
    `Estimated conversion: ${conversionEstimate.amountIn} token units → ` +
    `${conversionEstimate.amountOut} sats`
  )
  console.debug(`Estimated conversion fee: ${conversionEstimate.fee} token units`)
}
```

### React Native

```typescript
const paymentRequest = '<payment request>'
// Set to use token funds to pay via conversion
const optionalMaxSlippageBps = 50
const optionalCompletionTimeoutSecs = 30
const conversionOptions = {
  conversionType: new ConversionType.ToBitcoin({
    fromTokenIdentifier: '<token identifier>'
  }),
  maxSlippageBps: optionalMaxSlippageBps,
  completionTimeoutSecs: optionalCompletionTimeoutSecs
}

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: new PaymentRequest.Input({ input: paymentRequest }),
  amount: undefined,
  tokenIdentifier: undefined,
  conversionOptions,
  feePolicy: undefined
})

// If the fees are acceptable, continue to create the Send Payment
if (prepareResponse.conversionEstimate !== undefined) {
  const conversionEstimate = prepareResponse.conversionEstimate
  console.debug(
    `Estimated conversion: ${conversionEstimate.amountIn} token units → ` +
    `${conversionEstimate.amountOut} sats`
  )
  console.debug(`Estimated conversion fee: ${conversionEstimate.fee} token units`)
}
```

### Flutter

```dart
String paymentRequest = "<payment request>";
// Set to use token funds to pay via conversion
int optionalMaxSlippageBps = 50;
int optionalCompletionTimeoutSecs = 30;
final conversionOptions = ConversionOptions(
  conversionType: ConversionType.toBitcoin(
    fromTokenIdentifier: "<token identifier>",
  ),
  maxSlippageBps: optionalMaxSlippageBps,
  completionTimeoutSecs: optionalCompletionTimeoutSecs,
);

final request = PrepareSendPaymentRequest(
    paymentRequest: PaymentRequest.input(input: paymentRequest),
    amount: null,
    tokenIdentifier: null,
    conversionOptions: conversionOptions,
    feePolicy: null);
final response = await sdk.prepareSendPayment(request: request);

// If the fees are acceptable, continue to create the Send Payment
if (response.conversionEstimate != null) {
  print(
      "Estimated conversion: ${response.conversionEstimate!.amountIn} token units "
      "→ ${response.conversionEstimate!.amountOut} sats");
  print(
      "Estimated conversion fee: ${response.conversionEstimate!.fee} token units");
}
```

### Python

```python
payment_request = "<payment request>"
# Set to use token funds to pay via conversion
optional_max_slippage_bps = 50
optional_completion_timeout_secs = 30
conversion_options = ConversionOptions(
    conversion_type=ConversionType.TO_BITCOIN(
        from_token_identifier="<token identifier>"
    ),
    max_slippage_bps=optional_max_slippage_bps,
    completion_timeout_secs=optional_completion_timeout_secs,
)
try:
    request = PrepareSendPaymentRequest(
        payment_request=PaymentRequest.INPUT(input=payment_request),
        amount=None,
        token_identifier=None,
        conversion_options=conversion_options,
        fee_policy=None,
    )
    prepare_response = await sdk.prepare_send_payment(request=request)

    # If the fees are acceptable, continue to create the Send Payment
    if prepare_response.conversion_estimate is not None:
        conversion_estimate = prepare_response.conversion_estimate
        logging.debug(
            f"Estimated conversion: {conversion_estimate.amount_in}"
            f" token units → {conversion_estimate.amount_out} sats"
        )
        logging.debug(
            f"Estimated conversion fee: {conversion_estimate.fee} token units"
        )
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
paymentRequest := "<payment request>"
// Set to use token funds to pay via conversion
optionalMaxSlippageBps := uint32(50)
optionalCompletionTimeoutSecs := uint32(30)
conversionOptions := breez_sdk_spark.ConversionOptions{
	ConversionType: breez_sdk_spark.ConversionTypeToBitcoin{
		FromTokenIdentifier: "<token identifier>",
	},
	MaxSlippageBps:        &optionalMaxSlippageBps,
	CompletionTimeoutSecs: &optionalCompletionTimeoutSecs,
}

request := breez_sdk_spark.PrepareSendPaymentRequest{
	PaymentRequest:    breez_sdk_spark.PaymentRequestInput{Input: paymentRequest},
	Amount:            nil,
	TokenIdentifier:   nil,
	ConversionOptions: &conversionOptions,
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
if response.ConversionEstimate != nil {
	log.Printf(
		"Estimated conversion: %v token units → %v sats",
		response.ConversionEstimate.AmountIn,
		response.ConversionEstimate.AmountOut,
	)
	log.Printf("Estimated conversion fee: %v token units", response.ConversionEstimate.Fee)
}
```



**Developer note**

When a conversion fails due to exceeding the maximum slippage, the conversion will be refunded automatically.

**Developer note**

The conversion may result in some Bitcoin remaining in the wallet after the payment is sent. This remaining Bitcoin is to account for slippage in the conversion.