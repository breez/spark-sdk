package example

import (
	"crypto/sha256"
	"encoding/hex"
	"log"
	"math/big"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func SendHtlcPayment(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.Payment, error) {
	// ANCHOR: send-htlc-payment
	paymentRequest := "<spark address>"
	// Set the amount you wish the pay the receiver
	amountSats := new(big.Int).SetInt64(50_000)
	prepareRequest := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest: paymentRequest,
		Amount:         &amountSats,
	}
	prepareResponse, err := sdk.PrepareSendPayment(prepareRequest)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
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

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	payment := sendResponse.Payment
	// ANCHOR_END: send-htlc-payment
	return &payment, nil
}

func ListClaimableHtlcPayments(sdk *breez_sdk_spark.BreezSdk) (*[]breez_sdk_spark.Payment, error) {
	// ANCHOR: list-claimable-htlc-payments
	typeFilter := []breez_sdk_spark.PaymentType{
		breez_sdk_spark.PaymentTypeReceive,
	}
	statusFilter := []breez_sdk_spark.PaymentStatus{
		breez_sdk_spark.PaymentStatusPending,
	}
	var paymentDetailsFilter breez_sdk_spark.PaymentDetailsFilter = breez_sdk_spark.PaymentDetailsFilterSpark{
		HtlcStatus: &[]breez_sdk_spark.SparkHtlcStatus{
			breez_sdk_spark.SparkHtlcStatusWaitingForPreimage,
		},
	}

	request := breez_sdk_spark.ListPaymentsRequest{
		TypeFilter:            &typeFilter,
		StatusFilter:          &statusFilter,
		PaymentDetailsFilter:  &paymentDetailsFilter,
	}

	response, err := sdk.ListPayments(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	payments := response.Payments
	// ANCHOR_END: list-claimable-htlc-payments
	return &payments, nil
}

func ClaimHtlcPayment(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.Payment, error) {
	// ANCHOR: claim-htlc-payment
	preimage := "<preimage hex>"
	request := breez_sdk_spark.ClaimHtlcPaymentRequest{
		Preimage: preimage,
	}
	response, err := sdk.ClaimHtlcPayment(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	payment := response.Payment
	// ANCHOR_END: claim-htlc-payment
	return &payment, nil
}
