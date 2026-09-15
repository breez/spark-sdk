# Conditional Payments

Conditional payments use Hash Time-Locked Contracts (HTLCs) to lock funds with a cryptographic hash of a secret preimage and an expiration time. The payment can only be claimed by revealing the preimage before expiration. If not claimed in time, the funds are automatically returned to the sender. This enables use cases like atomic cross-chain swaps.

The SDK supports both sending conditional payments via Spark HTLCs and receiving them via HODL invoices.

**Developer note**

Preimages are required to be unique and are not managed by the SDK. It is your responsibility as a developer to manage them, including how to generate them, store them, and provide them when claiming payments.

## Sending Spark HTLC payments

HTLC payments use the standard payment API described in [Sending payments](send_payment.md). To create an HTLC payment, prepare the payment normally, then provide the Spark HTLC options when [sending](send_payment.md#spark). These options include the payment hash (SHA-256 hash of the preimage) and the expiry duration.

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

You can receive using HODL invoices — Lightning invoices where the payment is held until you claim it by revealing the preimage. To create one, provide a `PaymentHash` when calling `ReceivePayment` with the `ReceivePaymentMethodBolt11Invoice` payment method.

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

Once detected, claimable HTLC payments are immediately listed as pending in the [list of payments](/llms/go/guide/list_payments.md). Additionally, a `SdkEventPaymentPending` event is emitted to notify your application. See [Listening to events](/llms/go/guide/events.md) for more details.

To list only claimable HTLC payments, you can filter by HTLC status. This works for both Spark HTLC payments and HODL invoices.

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
