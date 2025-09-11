package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func ReceiveLightningBolt11(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.ReceivePaymentResponse, error) {
	// ANCHOR: receive-payment-lightning-bolt11
	description := "<invoice description>"
	// Optionally set the invoice amount you wish the payer to send
	optionalAmountSats := uint64(5_000)

	request := breez_sdk_spark.ReceivePaymentRequest{
		PaymentMethod: breez_sdk_spark.ReceivePaymentMethodBolt11Invoice{
			Description: description,
			AmountSats:  &optionalAmountSats,
		},
	}

	response, err := sdk.ReceivePayment(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	paymentRequest := response.PaymentRequest
	log.Printf("Payment Request: %v", paymentRequest)
	receiveFeesSat := response.FeeSats
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

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	paymentRequest := response.PaymentRequest
	log.Printf("Payment Request: %v", paymentRequest)
	receiveFeesSat := response.FeeSats
	log.Printf("Fees: %v sats", receiveFeesSat)
	// ANCHOR_END: receive-payment-onchain
	return &response, nil
}

func ReceiveSpark(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.ReceivePaymentResponse, error) {
	// ANCHOR: receive-payment-spark
	request := breez_sdk_spark.ReceivePaymentRequest{
		PaymentMethod: breez_sdk_spark.ReceivePaymentMethodSparkAddress{},
	}

	response, err := sdk.ReceivePayment(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	paymentRequest := response.PaymentRequest
	log.Printf("Payment Request: %v", paymentRequest)
	receiveFeesSat := response.FeeSats
	log.Printf("Fees: %v sats", receiveFeesSat)
	// ANCHOR_END: receive-payment-spark
	return &response, nil
}
