package example

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"log"
	"math/big"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func SendHtlcPayment(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.Payment, error) {
	// ANCHOR: send-htlc-payment
	paymentRequest := "<spark address>"
	// Set the amount you wish to pay the receiver
	amountSats := new(big.Int).SetInt64(50_000)
	prepareRequest := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest:    paymentRequest,
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
	// ANCHOR_END: send-htlc-payment
	return &payment, nil
}

func ReceiveHodlInvoicePayment(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: receive-hodl-invoice-payment
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
	// ANCHOR_END: receive-hodl-invoice-payment
	return nil
}

func ListClaimableHtlcPayments(sdk *breez_sdk_spark.BreezSdk) (*[]breez_sdk_spark.Payment, error) {
	// ANCHOR: list-claimable-htlc-payments
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

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return nil, err
	}

	payment := response.Payment
	// ANCHOR_END: claim-htlc-payment
	return &payment, nil
}
