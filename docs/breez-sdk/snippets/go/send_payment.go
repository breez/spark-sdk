package example

import (
	"log"
	"math/big"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func PrepareSendPaymentLightningBolt11(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.PrepareSendPaymentResponse, error) {
	// ANCHOR: prepare-send-payment-lightning-bolt11
	paymentRequest := "<bolt11 invoice>"
	// Optionally set the amount you wish the pay the receiver
	optionalAmountSats := new(big.Int).SetInt64(5_000)

	request := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest: paymentRequest,
		Amount:         &optionalAmountSats,
	}
	response, err := sdk.PrepareSendPayment(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	// If the fees are acceptable, continue to create the Send Payment
	switch paymentMethod := response.PaymentMethod.(type) {
	case breez_sdk_spark.SendPaymentMethodBolt11Invoice:
		// Fees to pay via Lightning
		lightningFeeSats := paymentMethod.LightningFeeSats
		// Or fees to pay (if available) via a Spark transfer
		sparkTransferFeeSats := paymentMethod.SparkTransferFeeSats
		log.Printf("Lightning Fees: %v sats", lightningFeeSats)
		log.Printf("Spark Transfer Fees: %v sats", sparkTransferFeeSats)
	}
	// ANCHOR_END: prepare-send-payment-lightning-bolt11
	return &response, nil
}

func PrepareSendPaymentOnchain(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.PrepareSendPaymentResponse, error) {
	// ANCHOR: prepare-send-payment-onchain
	paymentRequest := "<bitcoin address>"
	// Set the amount you wish the pay the receiver
	amountSats := new(big.Int).SetInt64(50_000)

	request := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest: paymentRequest,
		Amount:         &amountSats,
	}
	response, err := sdk.PrepareSendPayment(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	// If the fees are acceptable, continue to create the Send Payment
	switch paymentMethod := response.PaymentMethod.(type) {
	case breez_sdk_spark.SendPaymentMethodBitcoinAddress:
		feeQuote := paymentMethod.FeeQuote
		slowFeeQuote := feeQuote.SpeedSlow.UserFeeSat + feeQuote.SpeedSlow.L1BroadcastFeeSat
		mediumFeeQuote := feeQuote.SpeedMedium.UserFeeSat + feeQuote.SpeedMedium.L1BroadcastFeeSat
		fastFeeQuote := feeQuote.SpeedFast.UserFeeSat + feeQuote.SpeedFast.L1BroadcastFeeSat
		log.Printf("Slow Fees: %v sats", slowFeeQuote)
		log.Printf("Medium Fees: %v sats", mediumFeeQuote)
		log.Printf("Fast Fees: %v sats", fastFeeQuote)
	}
	// ANCHOR_END: prepare-send-payment-onchain
	return &response, nil
}

func PrepareSendPaymentSparkAddress(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.PrepareSendPaymentResponse, error) {
	// ANCHOR: prepare-send-payment-spark-address
	paymentRequest := "<spark address>"
	// Set the amount you wish the pay the receiver
	amountSats := new(big.Int).SetInt64(50_000)

	request := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest: paymentRequest,
		Amount:         &amountSats,
	}
	response, err := sdk.PrepareSendPayment(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	// If the fees are acceptable, continue to create the Send Payment
	switch paymentMethod := response.PaymentMethod.(type) {
	case breez_sdk_spark.SendPaymentMethodSparkAddress:
		feeSats := paymentMethod.Fee
		log.Printf("Fees: %v sats", feeSats)
	}
	// ANCHOR_END: prepare-send-payment-spark-address
	return &response, nil
}

func PrepareSendPaymentSparkInvoice(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.PrepareSendPaymentResponse, error) {
	// ANCHOR: prepare-send-payment-spark-invoice
	paymentRequest := "<spark invoice>"
	// Optionally set the amount you wish the pay the receiver
	optionalAmountSats := new(big.Int).SetInt64(50_000)

	request := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest: paymentRequest,
		Amount:         &optionalAmountSats,
	}
	response, err := sdk.PrepareSendPayment(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	// If the fees are acceptable, continue to create the Send Payment
	switch paymentMethod := response.PaymentMethod.(type) {
	case breez_sdk_spark.SendPaymentMethodSparkInvoice:
		feeSats := paymentMethod.Fee
		log.Printf("Fees: %v sats", feeSats)
	}
	// ANCHOR_END: prepare-send-payment-spark-invoice
	return &response, nil
}

func SendPaymentLightningBolt11(sdk *breez_sdk_spark.BreezSdk, prepareResponse breez_sdk_spark.PrepareSendPaymentResponse) (*breez_sdk_spark.Payment, error) {
	// ANCHOR: send-payment-lightning-bolt11
	var completionTimeoutSecs uint32 = 10
	var options breez_sdk_spark.SendPaymentOptions = breez_sdk_spark.SendPaymentOptionsBolt11Invoice{
		PreferSpark:           false,
		CompletionTimeoutSecs: &completionTimeoutSecs,
	}

	request := breez_sdk_spark.SendPaymentRequest{
		PrepareResponse: prepareResponse,
		Options:         &options,
	}
	response, err := sdk.SendPayment(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	payment := response.Payment
	// ANCHOR_END: send-payment-lightning-bolt11
	return &payment, nil
}

func SendPaymentOnchain(sdk *breez_sdk_spark.BreezSdk, prepareResponse breez_sdk_spark.PrepareSendPaymentResponse) (*breez_sdk_spark.Payment, error) {
	// ANCHOR: send-payment-onchain
	optionalIdempotencyKey := "<idempotency key uuid>"
	var options breez_sdk_spark.SendPaymentOptions = breez_sdk_spark.SendPaymentOptionsBitcoinAddress{
		ConfirmationSpeed: breez_sdk_spark.OnchainConfirmationSpeedMedium,
		IdempotencyKey:    &optionalIdempotencyKey,
	}

	request := breez_sdk_spark.SendPaymentRequest{
		PrepareResponse: prepareResponse,
		Options:         &options,
	}
	response, err := sdk.SendPayment(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	payment := response.Payment
	// ANCHOR_END: send-payment-onchain
	return &payment, nil
}

func SendPaymentSpark(sdk *breez_sdk_spark.BreezSdk, prepareResponse breez_sdk_spark.PrepareSendPaymentResponse) (*breez_sdk_spark.Payment, error) {
	// ANCHOR: send-payment-spark
	var options breez_sdk_spark.SendPaymentOptions = breez_sdk_spark.SendPaymentOptionsSpark{
		IdempotencyKey: "<idempotency key uuid>",
	}

	request := breez_sdk_spark.SendPaymentRequest{
		PrepareResponse: prepareResponse,
		Options:         &options,
	}
	response, err := sdk.SendPayment(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	payment := response.Payment
	// ANCHOR_END: send-payment-spark
	return &payment, nil
}
