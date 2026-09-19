# Listing payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

To view your payment history, you can list all the payments that have been sent and received.

```go
response, err := sdk.ListPayments(breez_sdk_spark.ListPaymentsRequest{})

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

payments := response.Payments
```



## Filtering Payments

When listing payments you can also filter and page the results.

```go
// Filter by asset (Bitcoin or Token)
tokenIdentifier := "token_identifier_here"
var assetFilter breez_sdk_spark.AssetFilter = breez_sdk_spark.AssetFilterToken{
	TokenIdentifier: &tokenIdentifier,
}
// To filter by Bitcoin instead:
// var assetFilter breez_sdk_spark.AssetFilter = breez_sdk_spark.AssetFilterBitcoin

// Filter options
typeFilter := []breez_sdk_spark.PaymentType{
	breez_sdk_spark.PaymentTypeSend,
	breez_sdk_spark.PaymentTypeReceive,
}
statusFilter := []breez_sdk_spark.PaymentStatus{
	breez_sdk_spark.PaymentStatusCompleted,
}
fromTimestamp := uint64(1704067200) // Unix timestamp
toTimestamp := uint64(1735689600)   // Unix timestamp
offset := uint32(0)
limit := uint32(50)
sortAscending := false

request := breez_sdk_spark.ListPaymentsRequest{
	TypeFilter:    &typeFilter,    // Filter by payment type
	StatusFilter:  &statusFilter,  // Filter by status
	AssetFilter:   &assetFilter,   // Filter by asset (Bitcoin or Token)
	FromTimestamp: &fromTimestamp, // Time range filters
	ToTimestamp:   &toTimestamp,   // Time range filters
	Offset:        &offset,        // Pagination
	Limit:         &limit,         // Pagination
	SortAscending: &sortAscending, // Sort order (true = oldest first, false = newest first)
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
```



## Get Payment

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_payment

You can also retrieve a single payment using the payment id:

```go
paymentId := "<payment id>"
request := breez_sdk_spark.GetPaymentRequest{
	PaymentId: paymentId,
}
response, err := sdk.GetPayment(request)

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
