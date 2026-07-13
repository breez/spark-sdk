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
	// Only valid for sends with no token leg (see Retry safety).
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

func GetCrossChainReceiveRoutes(sdk *breez_sdk_spark.BreezSdk) ([]breez_sdk_spark.CrossChainRoutePair, error) {
	// ANCHOR: cross-chain-get-receive-routes
	filter := breez_sdk_spark.CrossChainRouteFilterReceive{ContractAddress: nil}
	routes, err := sdk.GetCrossChainRoutes(filter)
	if err != nil {
		return nil, err
	}

	for _, route := range routes {
		log.Printf(
			"Route via %v: %s/%s -> Spark",
			route.Provider, route.Chain, route.Asset,
		)
	}
	// ANCHOR_END: cross-chain-get-receive-routes
	return routes, nil
}

func ReceivePaymentCrossChain(
	sdk *breez_sdk_spark.BreezSdk,
	route breez_sdk_spark.CrossChainRoutePair,
) (*breez_sdk_spark.ReceivePaymentResponse, error) {
	// ANCHOR: cross-chain-receive
	// With the default FeesExcluded mode, amount is the receiver's net target
	// on Spark in destination-asset base units (sats for BTC, token base units
	// for USDB). The SDK pads the sender's deposit to cover fees + overpay.
	// With FeesIncluded, amount is the sender's deposit in source-asset units.
	amount := new(big.Int).SetInt64(1_000)
	// Optionally set the destination Spark-side asset. nil = auto: active
	// stable-balance token if the route supports it, otherwise BTC.
	var optionalDestination *breez_sdk_spark.SparkAsset = nil
	// Optionally set the maximum slippage in basis points (10 to 500)
	optionalMaxSlippageBps := uint32(100)
	// Optionally override the overpay buffer (0 to 500 bps). Defaults to 15.
	var optionalTargetOverpayBps *uint32 = nil
	// Optionally override the fee mode. Defaults to FeesExcluded.
	var optionalFeeMode *breez_sdk_spark.CrossChainFeeMode = nil

	request := breez_sdk_spark.ReceivePaymentRequest{
		PaymentMethod: breez_sdk_spark.ReceivePaymentMethodCrossChain{
			Route:             route,
			Amount:            amount,
			Destination:       optionalDestination,
			FeeMode:           optionalFeeMode,
			MaxSlippageBps:    &optionalMaxSlippageBps,
			TargetOverpayBps:  optionalTargetOverpayBps,
		},
	}
	response, err := sdk.ReceivePayment(request)
	if err != nil {
		return nil, err
	}

	log.Printf("Payment request: %s", response.PaymentRequest)
	if info := response.CrossChainInfo; info != nil {
		log.Printf("Deposit address: %s", info.DepositAddress)
		log.Printf("Deposit amount: %v", info.DepositAmount)
		denom := "BTC"
		if info.TokenIdentifier != nil {
			denom = "USDB"
		}
		log.Printf(
			"Expected received: %v %s",
			info.ExpectedReceivedAmount, denom,
		)
		log.Printf("Expires at: %d", info.ExpiresAt)
	}
	// ANCHOR_END: cross-chain-receive
	return &response, nil
}
