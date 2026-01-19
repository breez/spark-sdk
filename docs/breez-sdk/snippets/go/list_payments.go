package example

import (
	"errors"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func GetPayment(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.Payment, error) {
	// ANCHOR: get-payment
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
	// ANCHOR_END: get-payment
	return &payment, nil
}

func ListPayments(sdk *breez_sdk_spark.BreezSdk) (*[]breez_sdk_spark.Payment, error) {
	// ANCHOR: list-payments
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
	// ANCHOR_END: list-payments
	return &payments, nil
}

func ListPaymentsFiltered(sdk *breez_sdk_spark.BreezSdk) (*[]breez_sdk_spark.Payment, error) {
	// ANCHOR: list-payments-filtered
	// Filter by asset (Bitcoin or Token)
	tokenIdentifier := "token_identifier_here"
	var assetFilter breez_sdk_spark.AssetFilter = breez_sdk_spark.AssetFilterToken{TokenIdentifier: &tokenIdentifier}
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
	// ANCHOR_END: list-payments-filtered
	return &payments, nil
}
