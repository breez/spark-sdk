package example

import (
	"errors"
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func PrepareSendPaymentLightningBolt11(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.PrepareSendPaymentResponse, error) {
	// ANCHOR: prepare-send-payment-lightning-bolt11
	paymentRequest := "<bolt11 invoice>"
	// Optionally set the amount you wish to pay the receiver
	var optionalPayAmount breez_sdk_spark.PayAmount = breez_sdk_spark.PayAmountBitcoin{
		AmountSats: 5_000,
	}

	request := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest: paymentRequest,
		PayAmount:      &optionalPayAmount,
	}
	response, err := sdk.PrepareSendPayment(request)

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
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
	// Set the amount you wish to pay the receiver
	var payAmount breez_sdk_spark.PayAmount = breez_sdk_spark.PayAmountBitcoin{
		AmountSats: 50_000,
	}

	request := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest: paymentRequest,
		PayAmount:      &payAmount,
	}
	response, err := sdk.PrepareSendPayment(request)

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return nil, err
	}

	// Review the fee quote for each confirmation speed
	switch paymentMethod := response.PaymentMethod.(type) {
	case breez_sdk_spark.SendPaymentMethodBitcoinAddress:
		feeQuote := paymentMethod.FeeQuote
		slowFeeSats := feeQuote.SpeedSlow.UserFeeSat + feeQuote.SpeedSlow.L1BroadcastFeeSat
		mediumFeeSats := feeQuote.SpeedMedium.UserFeeSat + feeQuote.SpeedMedium.L1BroadcastFeeSat
		fastFeeSats := feeQuote.SpeedFast.UserFeeSat + feeQuote.SpeedFast.L1BroadcastFeeSat
		log.Printf("Slow fee: %v sats", slowFeeSats)
		log.Printf("Medium fee: %v sats", mediumFeeSats)
		log.Printf("Fast fee: %v sats", fastFeeSats)
	}
	// ANCHOR_END: prepare-send-payment-onchain
	return &response, nil
}

func PrepareSendPaymentSparkAddress(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.PrepareSendPaymentResponse, error) {
	// ANCHOR: prepare-send-payment-spark-address
	paymentRequest := "<spark address>"
	// Set the amount you wish to pay the receiver
	var payAmount breez_sdk_spark.PayAmount = breez_sdk_spark.PayAmountBitcoin{
		AmountSats: 50_000,
	}

	request := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest: paymentRequest,
		PayAmount:      &payAmount,
	}
	response, err := sdk.PrepareSendPayment(request)

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
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
	// Optionally set the amount you wish to pay the receiver
	var optionalPayAmount breez_sdk_spark.PayAmount = breez_sdk_spark.PayAmountBitcoin{
		AmountSats: 50_000,
	}

	request := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest: paymentRequest,
		PayAmount:      &optionalPayAmount,
	}
	response, err := sdk.PrepareSendPayment(request)

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
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

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
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

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return nil, err
	}

	payment := response.Payment
	// ANCHOR_END: send-payment-lightning-bolt11
	return &payment, nil
}

func SendPaymentOnchain(sdk *breez_sdk_spark.BreezSdk, prepareResponse breez_sdk_spark.PrepareSendPaymentResponse) (*breez_sdk_spark.Payment, error) {
	// ANCHOR: send-payment-onchain
	// Select the confirmation speed for the on-chain transaction
	var options breez_sdk_spark.SendPaymentOptions = breez_sdk_spark.SendPaymentOptionsBitcoinAddress{
		ConfirmationSpeed: breez_sdk_spark.OnchainConfirmationSpeedMedium,
	}
	optionalIdempotencyKey := "<idempotency key uuid>"
	request := breez_sdk_spark.SendPaymentRequest{
		PrepareResponse: prepareResponse,
		Options:         &options,
		IdempotencyKey:  &optionalIdempotencyKey,
	}
	response, err := sdk.SendPayment(request)

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
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

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return nil, err
	}

	payment := response.Payment
	// ANCHOR_END: send-payment-spark
	return &payment, nil
}

func PrepareSendPaymentDrain(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.PrepareSendPaymentResponse, error) {
	// ANCHOR: prepare-send-payment-drain
	// Use PayAmountDrain to send all available funds
	paymentRequest := "<payment request>"
	var payAmount breez_sdk_spark.PayAmount = breez_sdk_spark.PayAmountDrain{}

	request := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest: paymentRequest,
		PayAmount:      &payAmount,
	}
	response, err := sdk.PrepareSendPayment(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	// The response contains PayAmountDrain to indicate this is a drain operation
	log.Printf("Pay amount: %v", response.PayAmount)
	// ANCHOR_END: prepare-send-payment-drain
	return &response, nil
}
