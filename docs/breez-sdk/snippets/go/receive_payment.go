package example

import (
	"errors"
	"log"
	"math/big"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func ReceiveLightningBolt11(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.ReceivePaymentResponse, error) {
	// ANCHOR: receive-payment-lightning-bolt11
	description := "<invoice description>"
	// Optionally set the invoice amount you wish the payer to send
	optionalAmountSats := uint64(5_000)
	// Optionally set the expiry duration in seconds
	optionalExpirySecs := uint32(3600)

	request := breez_sdk_spark.ReceivePaymentRequest{
		PaymentMethod: breez_sdk_spark.ReceivePaymentMethodBolt11Invoice{
			Description: description,
			AmountSats:  &optionalAmountSats,
			ExpirySecs:  &optionalExpirySecs,
			PaymentHash: nil,
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
	receiveFeesSat := response.Fee
	log.Printf("Fees: %v sats", receiveFeesSat)
	// ANCHOR_END: receive-payment-lightning-bolt11
	return &response, nil
}

func ReceiveOnchain(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.ReceivePaymentResponse, error) {
	// ANCHOR: receive-payment-onchain
	request := breez_sdk_spark.ReceivePaymentRequest{
		PaymentMethod: breez_sdk_spark.ReceivePaymentMethodBitcoinAddress{},
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
	receiveFeesSat := response.Fee
	log.Printf("Fees: %v sats", receiveFeesSat)
	// ANCHOR_END: receive-payment-onchain
	return &response, nil
}

func ReceiveSparkAddress(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.ReceivePaymentResponse, error) {
	// ANCHOR: receive-payment-spark-address
	request := breez_sdk_spark.ReceivePaymentRequest{
		PaymentMethod: breez_sdk_spark.ReceivePaymentMethodSparkAddress{},
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
	receiveFeesSat := response.Fee
	log.Printf("Fees: %v sats", receiveFeesSat)
	// ANCHOR_END: receive-payment-spark-address
	return &response, nil
}

func ReceiveSparkInvoice(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.ReceivePaymentResponse, error) {
	// ANCHOR: receive-payment-spark-invoice
	optionalDescription := "<invoice description>"
	optionalAmountSats := new(big.Int).SetInt64(5_000)
	// Optionally set the expiry UNIX timestamp in seconds
	optionalExpiryTimeSeconds := uint64(1716691200)
	optionalSenderPublicKey := "<sender public key>"

	request := breez_sdk_spark.ReceivePaymentRequest{
		PaymentMethod: breez_sdk_spark.ReceivePaymentMethodSparkInvoice{
			Description:     &optionalDescription,
			Amount:          &optionalAmountSats,
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
	receiveFeesSat := response.Fee
	log.Printf("Fees: %v sats", receiveFeesSat)
	// ANCHOR_END: receive-payment-spark-invoice
	return &response, nil
}
