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
	// Set this to create an invoice for another Spark identity
	var optionalReceiverIdentityPublicKey *string
	// Set to FeePolicyFeesIncluded to deduct a partner fee from the amount
	// instead of adding it on top for the payer
	var optionalFeePolicy *breez_sdk_spark.FeePolicy

	request := breez_sdk_spark.ReceivePaymentRequest{
		PaymentMethod: breez_sdk_spark.ReceivePaymentMethodBolt11Invoice{
			Description: description,
			AmountSats:  &optionalAmountSats,
			ExpirySecs:  &optionalExpirySecs,
			PaymentHash: nil,
			ReceiverIdentityPublicKey: optionalReceiverIdentityPublicKey,
			FeePolicy:                 optionalFeePolicy,
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
	switch fee := response.Fee.(type) {
	case breez_sdk_spark.ReceiveFeeFixed:
		log.Printf("Fees: %v sats", fee.Amount)
	// Only on an amountless invoice: a share of what the payer sends
	case breez_sdk_spark.ReceiveFeeRate:
		log.Printf("Fees: %v bps of the amount sent plus %v sats", fee.Bps, fee.FixedSats)
	}
	// ANCHOR_END: receive-payment-lightning-bolt11
	return &response, nil
}

func ReceiveOnchain(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.ReceivePaymentResponse, error) {
	// ANCHOR: receive-payment-onchain
	var newAddress *bool // To get a new address: t := true; newAddress = &t
	request := breez_sdk_spark.ReceivePaymentRequest{
		PaymentMethod: breez_sdk_spark.ReceivePaymentMethodBitcoinAddress{
			NewAddress: newAddress,
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
	if fee, ok := response.Fee.(breez_sdk_spark.ReceiveFeeFixed); ok {
		log.Printf("Fees: %v sats", fee.Amount)
	}
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
	if fee, ok := response.Fee.(breez_sdk_spark.ReceiveFeeFixed); ok {
		log.Printf("Fees: %v sats", fee.Amount)
	}
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
	if fee, ok := response.Fee.(breez_sdk_spark.ReceiveFeeFixed); ok {
		log.Printf("Fees: %v sats", fee.Amount)
	}
	// ANCHOR_END: receive-payment-spark-invoice
	return &response, nil
}
