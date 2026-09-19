# Sending and receiving tokens

Spark supports tokens using the [BTKN protocol](https://docs.spark.money/learn/tokens/hello-btkn). The Breez SDK enables you to send and receive these tokens using the standard payments API.

## Fetching token balances

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_info

Token balances for all tokens currently held in the wallet can be retrieved along with general wallet information. Each token balance includes both the balance amount and the token metadata (identifier, name, ticker, issuer public key, etc.).

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
