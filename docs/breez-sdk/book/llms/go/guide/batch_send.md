# Sending to multiple recipients

A single transaction can pay multiple recipients at once. The batch API is separate from the regular send flow because a batch has no single amount to report: it may span several tokens, so prepare reports a total per asset instead.

A batch currently pays tokens only. Sending sats to several recipients at once is not supported yet, so every recipient must resolve to a token.

## Preparing the batch

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_batch

Each recipient is identified by a `PaymentRequest`, which is either a Spark address or a Spark invoice, and the two may be mixed freely in one batch:

- **Spark address**: the token identifier and the amount must be set, exactly as for a single-recipient send. Leaving the token identifier unset would name sats, which a batch cannot send yet.
- **Spark invoice**: the token and amount are taken from the invoice. The amount is only required if the invoice doesn't specify one. If the invoice specifies an amount, providing a different amount is not supported.

The same invoice may only appear once in a batch. Repeating a plain Spark address is allowed: that is simply two outputs to the same payee.

A batch that pays a Spark invoice is limited to a single token, and that includes its address recipients: the Spark operators reject a transaction that carries an invoice and pays more than one token. Prepare rejects such a batch, so split it into one batch per token. A batch of address recipients only may span as many tokens as you like.

The response resolves every recipient into the concrete `Destination`, asset and amount it will be paid, and reports `Totals`: what the batch debits, one entry per distinct asset. You may show these to the user before sending.

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

Each payment also raises its own `SdkEventPaymentSucceeded` event, the same as a single-recipient send.

**Developer note**

A token send carries no idempotency key. If a batch fails in a way that leaves the outcome unknown, look for the transaction first (filtering the payment list by transaction hash) instead of sending it again, which would pay every recipient twice.

## External signing

A batch can also be signed outside the SDK. The flow matches the single-recipient one described in [Client signing](./client_signing.md), with two differences: the unsigned package is built from the batch prepare response, and publishing it returns every payment rather than one. The package carries the same per-asset totals, so the signer can show the user what they are approving.
