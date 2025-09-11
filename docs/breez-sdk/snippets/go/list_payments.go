package example

import (
	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func GetPayment(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.Payment, error) {
	// ANCHOR: get-payment
	paymentId := "<payment id>"
	request := breez_sdk_spark.GetPaymentRequest{
		PaymentId: paymentId,
	}
	response, err := sdk.GetPayment(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	payment := response.Payment
	// ANCHOR_END: get-payment
	return &payment, nil
}

func ListPayments(sdk *breez_sdk_spark.BreezSdk) (*[]breez_sdk_spark.Payment, error) {
	// ANCHOR: list-payments
	response, err := sdk.ListPayments(breez_sdk_spark.ListPaymentsRequest{})

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	payments := response.Payments
	// ANCHOR_END: list-payments
	return &payments, nil
}

func ListPaymentsFiltered(sdk *breez_sdk_spark.BreezSdk) (*[]breez_sdk_spark.Payment, error) {
	// ANCHOR: list-payments-filtered
	limit := uint32(50)
	offset := uint32(0)
	request := breez_sdk_spark.ListPaymentsRequest{
		Offset: &offset,
		Limit:  &limit,
	}
	response, err := sdk.ListPayments(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	payments := response.Payments
	// ANCHOR_END: list-payments-filtered
	return &payments, nil
}
