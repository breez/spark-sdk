package example

import (
	"errors"
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func HandleErrors(
	sdk *breez_sdk_spark.BreezSdk,
	request breez_sdk_spark.PrepareSendPaymentRequest,
) error {
	// ANCHOR: handle-errors
	prepareResponse, err := sdk.PrepareSendPayment(request)

	var insufficient *breez_sdk_spark.SdkErrorInsufficientFunds
	var disabled *breez_sdk_spark.SdkErrorCrossChainDisabled
	switch {
	case err == nil:
		log.Printf("Payment prepared: %v", prepareResponse.PaymentMethod)
	case errors.As(err, &insufficient):
		log.Printf("Not enough funds for this payment")
	case errors.As(err, &disabled):
		log.Printf("Cross-chain payments are not enabled, see %v", disabled.DocsUrl)
	default:
		log.Printf("Failed to prepare the payment: %v", err)
	}
	// ANCHOR_END: handle-errors
	return nil
}
