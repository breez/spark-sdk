package example

import (
	"errors"
	"log"
	"math/big"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func GetCrossChainRoutes(sdk *breez_sdk_spark.BreezSdk) ([]breez_sdk_spark.CrossChainRoutePair, error) {
	// ANCHOR: cross-chain-get-routes
	inputStr := "<recipient address>"
	input, err := sdk.Parse(inputStr)
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError
		}
		return nil, err
	}

	addressInput, ok := input.(breez_sdk_spark.InputTypeCrossChainAddress)
	if !ok {
		return nil, errors.New("not a cross-chain address")
	}
	addressDetails := addressInput.Field0

	filter := breez_sdk_spark.CrossChainRouteFilterSend{AddressDetails: addressDetails}
	routes, err := sdk.GetCrossChainRoutes(filter)
	if err != nil {
		return nil, err
	}

	for _, route := range routes {
		log.Printf("Route via %v: %s/%s", route.Provider, route.Chain, route.Asset)
	}
	// ANCHOR_END: cross-chain-get-routes
	return routes, nil
}

func PrepareSendPaymentCrossChain(
	sdk *breez_sdk_spark.BreezSdk,
	addressDetails breez_sdk_spark.CrossChainAddressDetails,
	route breez_sdk_spark.CrossChainRoutePair,
) (*breez_sdk_spark.PrepareSendPaymentResponse, error) {
	// ANCHOR: cross-chain-prepare
	// Optionally set the maximum slippage in basis points (10 to 500)
	optionalMaxSlippageBps := uint32(100)
	amount := new(big.Int).SetInt64(50_000)

	request := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest: breez_sdk_spark.PaymentRequestCrossChain{
			Address:           addressDetails.Address,
			Route:             route,
			MaxSlippageBps:    &optionalMaxSlippageBps,
			TargetOverpayBps:  nil,
		},
		Amount:            &amount,
		TokenIdentifier:   nil,
		ConversionOptions: nil,
		FeePolicy:         nil,
	}
	response, err := sdk.PrepareSendPayment(request)
	if err != nil {
		return nil, err
	}

	switch paymentMethod := response.PaymentMethod.(type) {
	case breez_sdk_spark.SendPaymentMethodCrossChainAddress:
		log.Printf("Amount in: %v", paymentMethod.AmountIn)
		log.Printf("Estimated out: %v", paymentMethod.EstimatedOut)
		log.Printf("Provider fee: %v", paymentMethod.FeeAmount)
		log.Printf("Quote expires at: %s", paymentMethod.ExpiresAt)
	}
	// ANCHOR_END: cross-chain-prepare
	return &response, nil
}

func SendPaymentCrossChain(
	sdk *breez_sdk_spark.BreezSdk,
	prepareResponse breez_sdk_spark.PrepareSendPaymentResponse,
) (*breez_sdk_spark.SendPaymentResponse, error) {
	// ANCHOR: cross-chain-send
	optionalIdempotencyKey := "<idempotency key uuid>"
	request := breez_sdk_spark.SendPaymentRequest{
		PrepareResponse: prepareResponse,
		Options:         nil,
		IdempotencyKey:  &optionalIdempotencyKey,
	}
	response, err := sdk.SendPayment(request)
	if err != nil {
		return nil, err
	}
	log.Printf("Payment: %v", response.Payment)
	// ANCHOR_END: cross-chain-send
	return &response, nil
}
