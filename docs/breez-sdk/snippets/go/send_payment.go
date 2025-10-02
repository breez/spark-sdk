package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func PrepareSendPaymentLightningBolt11(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.PrepareSendPaymentResponse, error) {
	// ANCHOR: prepare-send-payment-lightning-bolt11
	paymentRequest := "<bolt11 invoice>"
	// Optionally set the amount you wish the pay the receiver
	optionalAmountSats := uint64(5_000)

	request := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest: paymentRequest,
		AmountSats:     &optionalAmountSats,
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
	amountSats := uint64(50_000)

	request := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest: paymentRequest,
		AmountSats:     &amountSats,
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

func PrepareSendPaymentSpark(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.PrepareSendPaymentResponse, error) {
	// ANCHOR: prepare-send-payment-spark
	paymentRequest := "<spark address>"
	// Set the amount you wish the pay the receiver
	amountSats := uint64(50_000)

	request := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest: paymentRequest,
		AmountSats:     &amountSats,
	}
	response, err := sdk.PrepareSendPayment(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	// If the fees are acceptable, continue to create the Send Payment
	switch paymentMethod := response.PaymentMethod.(type) {
	case breez_sdk_spark.SendPaymentMethodSparkAddress:
		feeSats := paymentMethod.FeeSats
		log.Printf("Fees: %v sats", feeSats)
	}
	// ANCHOR_END: prepare-send-payment-spark
	return &response, nil
}

func SendPaymentLightningBolt11(sdk *breez_sdk_spark.BreezSdk, prepareResponse breez_sdk_spark.PrepareSendPaymentResponse) (*breez_sdk_spark.Payment, error) {
	// ANCHOR: send-payment-lightning-bolt11
	var returnPendingAfterSecs uint32 = 0
	var options breez_sdk_spark.SendPaymentOptions = breez_sdk_spark.SendPaymentOptionsBolt11Invoice{
		PreferSpark:            true,
		ReturnPendingAfterSecs: &returnPendingAfterSecs,
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
	var options breez_sdk_spark.SendPaymentOptions = breez_sdk_spark.SendPaymentOptionsBitcoinAddress{
		ConfirmationSpeed: breez_sdk_spark.OnchainConfirmationSpeedMedium,
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
	request := breez_sdk_spark.SendPaymentRequest{
		PrepareResponse: prepareResponse,
	}
	response, err := sdk.SendPayment(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	payment := response.Payment
	// ANCHOR_END: send-payment-spark
	return &payment, nil
}
