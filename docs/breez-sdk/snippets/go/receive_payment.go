package example

import (
	"log"
	"math/big"

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

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
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

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
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

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	paymentRequest := response.PaymentRequest
	log.Printf("Payment Request: %v", paymentRequest)
	receiveFeesSat := response.Fee
	log.Printf("Fees: %v sats", receiveFeesSat)
	// ANCHOR_END: receive-payment-spark-invoice
	return &response, nil
}

func WaitForPayment(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: wait-for-payment
	// Waiting for a payment given its payment request (Bolt11 or Spark invoice)
	paymentRequest := "<Bolt11 or Spark invoice>"

	// Wait for a payment to be completed using a payment request
	paymentRequestResponse, err := sdk.WaitForPayment(breez_sdk_spark.WaitForPaymentRequest{
		Identifier: breez_sdk_spark.WaitForPaymentIdentifierPaymentRequest{
			Field0: paymentRequest,
		},
	})

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return err
	}

	log.Printf("Payment received with ID: %v", paymentRequestResponse.Payment.Id)

	// Waiting for a payment given its payment id
	paymentId := "<payment id>"

	// Wait for a payment to be completed using a payment id
	paymentIdResponse, err := sdk.WaitForPayment(breez_sdk_spark.WaitForPaymentRequest{
		Identifier: breez_sdk_spark.WaitForPaymentIdentifierPaymentId{
			Field0: paymentId,
		},
	})

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return err
	}

	log.Printf("Payment received with ID: %v", paymentIdResponse.Payment.Id)
	// ANCHOR_END: wait-for-payment
	return nil
}
