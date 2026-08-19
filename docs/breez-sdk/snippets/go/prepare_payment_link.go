package example

import (
	"errors"
	"log"
	"math/big"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func PreparePaymentLinkViaCashapp(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.PreparePaymentLinkResponse, error) {
	// ANCHOR: prepare-payment-link-cashapp
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

	// List the stablecoin destinations you can send to and pick one, e.g. USDC
	// on Base. These routes are funded by an external rail, not the Spark wallet.
	// Restrict to Lightning-fundable routes, since Cash App funds over Lightning.
	filter := breez_sdk_spark.CrossChainRouteFilterPaymentLink{
		AddressDetails: addressDetails,
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
	request := breez_sdk_spark.PreparePaymentLinkRequest{
		Address:        addressDetails.Address,
		Route:          *route,
		Amount:         new(big.Int).SetInt64(10_000_000),
		FeePolicy:      nil,
		MaxSlippageBps: nil,
	}
	response, err := sdk.PreparePaymentLink(request)
	if err != nil {
		return nil, err
	}

	// Open this Cash App URL to pay; the recipient then receives the stablecoin.
	log.Printf("Open this URL in Cash App: %v", response.Url)
	log.Printf("Recipient receives ~%v %s", response.EstimatedOut, response.Asset)
	// ANCHOR_END: prepare-payment-link-cashapp
	return &response, nil
}
