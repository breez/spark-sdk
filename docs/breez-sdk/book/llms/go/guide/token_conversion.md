# Converting tokens

Token conversion enables payments to be made without holding the required asset by converting on-the-fly between Bitcoin and tokens using the Flashnet protocol.

## Fetching conversion limits

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.fetch_conversion_limits

Before performing a conversion, you can fetch the minimum amounts required for the conversion. The limits depend on the conversion direction:

- **Bitcoin to token**: Minimum Bitcoin amount (in satoshis) and minimum token amount to receive (in token base units)
- **Token to Bitcoin**: Minimum token amount (in token base units) and minimum Bitcoin amount to receive (in satoshis)

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
