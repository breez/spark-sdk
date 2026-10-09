package example

import (
	"errors"
	"log"
	"math/big"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func BridgeFromCashApp(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.BridgeFromCashAppResponse, error) {
	// ANCHOR: bridge-from-cash-app
	// Parse the recipient's external-chain address (EVM/Solana/Tron).
	inputStr := "<recipient address>"
	input, err := sdk.Parse(inputStr)
	if err != nil {
		return nil, err
	}
	addressInput, ok := input.(breez_sdk_spark.InputTypeCrossChainAddress)
	if !ok {
		return nil, errors.New("not a cross-chain address")
	}
	addressDetails := addressInput.Field0

	// List the stablecoin destinations Cash App can fund over Lightning and
	// pick one, e.g. USDC on Base.
	deliveryMethod := breez_sdk_spark.DeliveryMethodLightning
	filter := breez_sdk_spark.CrossChainRouteFilterSend{
		AddressDetails: addressDetails,
		DeliveryMethod: &deliveryMethod,
	}
	routes, err := sdk.GetCrossChainRoutes(filter)
	if err != nil {
		return nil, err
	}
	var route *breez_sdk_spark.CrossChainRoutePair
	for i := range routes {
		if routes[i].Asset == "USDC" && routes[i].Chain == "base" {
			route = &routes[i]
			break
		}
	}
	if route == nil {
		return nil, errors.New("no USDC route on Base")
	}

	// Send $10 of USDC, funded by Cash App over Lightning. The amount is in the
	// route asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC,
	// about $10.
	request := breez_sdk_spark.BridgeFromCashAppRequest{
		Address:        addressDetails.Address,
		Route:          *route,
		Amount:         new(big.Int).SetInt64(10_000_000),
		FeePolicy:      nil,
		MaxSlippageBps: nil,
	}
	response, err := sdk.BridgeFromCashApp(request)
	if err != nil {
		return nil, err
	}

	// Open this Cash App URL to pay. The recipient then receives the stablecoin.
	log.Printf("Open this URL in Cash App: %v", response.Url)
	log.Printf("Recipient receives ~%v %s", response.EstimatedOut, response.Asset)
	// ANCHOR_END: bridge-from-cash-app
	return &response, nil
}

func BridgeToCashApp(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.BridgeToCashAppResponse, error) {
	// ANCHOR: bridge-to-cash-app
	// List the stablecoin sources that can pay a Cash App user over Lightning
	// and pick one, e.g. USDC on Base.
	deliveryMethod := breez_sdk_spark.DeliveryMethodLightning
	filter := breez_sdk_spark.CrossChainRouteFilterReceive{
		ContractAddress: nil,
		DeliveryMethod:  &deliveryMethod,
	}
	routes, err := sdk.GetCrossChainRoutes(filter)
	if err != nil {
		return nil, err
	}
	var route *breez_sdk_spark.CrossChainRoutePair
	for i := range routes {
		if routes[i].Asset == "USDC" && routes[i].Chain == "base" {
			route = &routes[i]
			break
		}
	}
	if route == nil {
		return nil, errors.New("no USDC route on Base")
	}

	// Pay $10 of USDC to the Cash App user $alice. The amount is in the route
	// asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC, about $10.
	// The deposit is refunded to the payer's address if delivery fails.
	request := breez_sdk_spark.BridgeToCashAppRequest{
		Recipient:      "$alice",
		Route:          *route,
		Amount:         new(big.Int).SetInt64(10_000_000),
		FeePolicy:      nil,
		RefundAddress:  "<payer address>",
		MaxSlippageBps: nil,
	}
	response, err := sdk.BridgeToCashApp(request)
	if err != nil {
		return nil, err
	}

	// Show the payer what to pay. The recipient then receives Bitcoin.
	info := response.Info
	log.Printf("Pay with: %s", response.PaymentRequest)
	log.Printf(
		"Deposit %v to %s, recipient receives ~%v sats",
		info.DepositAmount, info.DepositAddress, info.ExpectedReceivedAmount,
	)
	// ANCHOR_END: bridge-to-cash-app
	return &response, nil
}
