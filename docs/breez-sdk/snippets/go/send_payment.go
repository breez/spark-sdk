package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func PrepareSendPaymentLightningBolt11(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.PrepareSendPaymentResponse, error) {
	// ANCHOR: prepare-send-payment-lightning-bolt11
	paymentRequest := "<bolt11 invoice>"
	// Optionally set the amount you wish the pay the receiver
	var optionalPayAmount breez_sdk_spark.PayAmount = breez_sdk_spark.PayAmountBitcoin{
		AmountSats: 5_000,
	}

	request := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest: paymentRequest,
		PayAmount:      &optionalPayAmount,
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
	var payAmount breez_sdk_spark.PayAmount = breez_sdk_spark.PayAmountBitcoin{
		AmountSats: 50_000,
	}
	// Select the confirmation speed (required for Bitcoin addresses)
	onchainSpeed := breez_sdk_spark.OnchainConfirmationSpeedMedium

	request := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest: paymentRequest,
		PayAmount:      &payAmount,
		OnchainSpeed:   &onchainSpeed,
	}
	response, err := sdk.PrepareSendPayment(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	// If the fees are acceptable, continue to create the Send Payment
	switch paymentMethod := response.PaymentMethod.(type) {
	case breez_sdk_spark.SendPaymentMethodBitcoinAddress:
		feeSats := paymentMethod.FeeSats
		selectedSpeed := paymentMethod.SelectedSpeed
		log.Printf("Fee for %v speed: %v sats", selectedSpeed, feeSats)
	}
	// ANCHOR_END: prepare-send-payment-onchain
	return &response, nil
}

func PrepareSendPaymentSparkAddress(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.PrepareSendPaymentResponse, error) {
	// ANCHOR: prepare-send-payment-spark-address
	paymentRequest := "<spark address>"
	// Set the amount you wish the pay the receiver
	var payAmount breez_sdk_spark.PayAmount = breez_sdk_spark.PayAmountBitcoin{
		AmountSats: 50_000,
	}

	request := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest: paymentRequest,
		PayAmount:      &payAmount,
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
	var optionalPayAmount breez_sdk_spark.PayAmount = breez_sdk_spark.PayAmountBitcoin{
		AmountSats: 50_000,
	}

	request := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest: paymentRequest,
		PayAmount:      &optionalPayAmount,
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

func PrepareSendPaymentTokenConversion(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.PrepareSendPaymentResponse, error) {
	// ANCHOR: prepare-send-payment-with-conversion
	paymentRequest := "<payment request>"
	// Set to use token funds to pay via conversion
	optionalMaxSlippageBps := uint32(50)
	optionalCompletionTimeoutSecs := uint32(30)
	conversionOptions := breez_sdk_spark.ConversionOptions{
		ConversionType: breez_sdk_spark.ConversionTypeToBitcoin{
			FromTokenIdentifier: "<token identifier>",
		},
		MaxSlippageBps:        &optionalMaxSlippageBps,
		CompletionTimeoutSecs: &optionalCompletionTimeoutSecs,
	}

	request := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest:    paymentRequest,
		ConversionOptions: &conversionOptions,
	}
	response, err := sdk.PrepareSendPayment(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	// If the fees are acceptable, continue to create the Send Payment
	if response.ConversionEstimate != nil {
		log.Printf("Estimated conversion amount: %v token base units", response.ConversionEstimate.Amount)
		log.Printf("Estimated conversion fee: %v token base units", response.ConversionEstimate.Fee)
	}
	// ANCHOR_END: prepare-send-payment-with-conversion
	return &response, nil
}

func SendPaymentLightningBolt11(sdk *breez_sdk_spark.BreezSdk, prepareResponse breez_sdk_spark.PrepareSendPaymentResponse) (*breez_sdk_spark.Payment, error) {
	// ANCHOR: send-payment-lightning-bolt11
	var completionTimeoutSecs uint32 = 10
	var options breez_sdk_spark.SendPaymentOptions = breez_sdk_spark.SendPaymentOptionsBolt11Invoice{
		PreferSpark:           false,
		CompletionTimeoutSecs: &completionTimeoutSecs,
	}

	optionalIdempotencyKey := "<idempotency key uuid>"
	request := breez_sdk_spark.SendPaymentRequest{
		PrepareResponse: prepareResponse,
		Options:         &options,
		IdempotencyKey:  &optionalIdempotencyKey,
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
	request := breez_sdk_spark.SendPaymentRequest{
		PrepareResponse: prepareResponse,
		IdempotencyKey:  &optionalIdempotencyKey,
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
	optionalIdempotencyKey := "<idempotency key uuid>"
	request := breez_sdk_spark.SendPaymentRequest{
		PrepareResponse: prepareResponse,
		IdempotencyKey:  &optionalIdempotencyKey,
	}
	response, err := sdk.SendPayment(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	payment := response.Payment
	// ANCHOR_END: send-payment-spark
	return &payment, nil
}

func EstimateOnchainSendFeeQuotes(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.EstimateOnchainSendFeeQuotesResponse, error) {
	// ANCHOR: estimate-onchain-send-fee-quotes
	address := "<bitcoin address>"
	// Optionally set the amount, omit for drain
	optionalAmountSats := uint64(50_000)

	request := breez_sdk_spark.EstimateOnchainSendFeeQuotesRequest{
		Address:    address,
		AmountSats: &optionalAmountSats,
	}
	response, err := sdk.EstimateOnchainSendFeeQuotes(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	feeQuote := response.FeeQuote
	slowFeeQuote := feeQuote.SpeedSlow.UserFeeSat + feeQuote.SpeedSlow.L1BroadcastFeeSat
	mediumFeeQuote := feeQuote.SpeedMedium.UserFeeSat + feeQuote.SpeedMedium.L1BroadcastFeeSat
	fastFeeQuote := feeQuote.SpeedFast.UserFeeSat + feeQuote.SpeedFast.L1BroadcastFeeSat
	log.Printf("Slow Fees: %v sats", slowFeeQuote)
	log.Printf("Medium Fees: %v sats", mediumFeeQuote)
	log.Printf("Fast Fees: %v sats", fastFeeQuote)
	// ANCHOR_END: estimate-onchain-send-fee-quotes
	return &response, nil
}

func PrepareSendPaymentDrainOnchain(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.PrepareSendPaymentResponse, error) {
	// ANCHOR: prepare-send-payment-drain-onchain
	paymentRequest := "<bitcoin address>"
	// Select the confirmation speed (required for Bitcoin addresses)
	onchainSpeed := breez_sdk_spark.OnchainConfirmationSpeedMedium
	// Use Drain to send all available funds
	var payAmount breez_sdk_spark.PayAmount = breez_sdk_spark.PayAmountDrain{}

	request := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest: paymentRequest,
		PayAmount:      &payAmount,
		OnchainSpeed:   &onchainSpeed,
	}
	response, err := sdk.PrepareSendPayment(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	// The amount is calculated as balance minus the fee for the selected speed
	switch paymentMethod := response.PaymentMethod.(type) {
	case breez_sdk_spark.SendPaymentMethodBitcoinAddress:
		drainAmount := response.Amount
		feeSats := paymentMethod.FeeSats
		selectedSpeed := paymentMethod.SelectedSpeed
		log.Printf("Drain amount: %v sats", drainAmount)
		log.Printf("Fee for %v speed: %v sats", selectedSpeed, feeSats)
	}
	// ANCHOR_END: prepare-send-payment-drain-onchain
	return &response, nil
}
