package example

import (
	"errors"
	"log"
	"math/big"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func BridgeFromCashApp(
	client *breez_sdk_spark.BridgingClient,
) (*breez_sdk_spark.CreateBridgeResponse, error) {
	// ANCHOR: bridge-from-cash-app
	// List the routes to the recipient's external-chain address
	// (EVM/Solana/Tron) and pick a stablecoin Cash App can fund over
	// Lightning, e.g. USDC on Base.
	recipient := "<recipient address>"
	routesResponse, err := client.GetRoutes(breez_sdk_spark.GetBridgeRoutesRequest{
		Recipient: recipient,
	})
	if err != nil {
		return nil, err
	}
	routes := routesResponse.Routes
	var route *breez_sdk_spark.BridgeRoutePair
	for i := range routes {
		if routes[i].Source.Chain == "lightning" &&
			routes[i].Destination.Asset == "USDC" &&
			routes[i].Destination.Chain == "base" {
			route = &routes[i]
			break
		}
	}
	if route == nil {
		return nil, errors.New("no USDC route on Base")
	}

	// Send $10 of USDC, funded by Cash App over Lightning. The amount is in
	// USDC base units (6 decimals), so 10_000_000 = 10 USDC, about $10.
	response, err := client.BridgeFromCashApp(breez_sdk_spark.CreateBridgeRequest{
		Recipient:      recipient,
		Route:          *route,
		Amount:         new(big.Int).SetInt64(10_000_000),
		RefundAddress:  nil,
		FeePolicy:      nil,
		MaxSlippageBps: nil,
	})
	if err != nil {
		return nil, err
	}

	// Open this Cash App link to pay. The recipient then receives the stablecoin.
	info := response.Info
	log.Printf("Open this URL in Cash App: %s", response.PaymentRequest)
	log.Printf("Recipient receives ~%v %s", info.ExpectedReceivedAmount, info.DestinationAsset)
	// ANCHOR_END: bridge-from-cash-app
	return &response, nil
}

func BridgeToCashApp(
	client *breez_sdk_spark.BridgingClient,
) (*breez_sdk_spark.CreateBridgeResponse, error) {
	// ANCHOR: bridge-to-cash-app
	// List the routes to the Cash App user and pick what the payer pays in,
	// e.g. USDC on Base.
	recipient := "$alice"
	routesResponse, err := client.GetRoutes(breez_sdk_spark.GetBridgeRoutesRequest{
		Recipient: recipient,
	})
	if err != nil {
		return nil, err
	}
	routes := routesResponse.Routes
	var route *breez_sdk_spark.BridgeRoutePair
	for i := range routes {
		if routes[i].Source.Asset == "USDC" && routes[i].Source.Chain == "base" {
			route = &routes[i]
			break
		}
	}
	if route == nil {
		return nil, errors.New("no route from USDC on Base")
	}

	// Pay $10 of USDC to the Cash App user $alice. The amount is in USDC base
	// units (6 decimals), so 10_000_000 = 10 USDC, about $10. The deposit is
	// refunded to the payer's address if delivery fails.
	refundAddress := "<payer address>"
	response, err := client.BridgeToCashApp(breez_sdk_spark.CreateBridgeRequest{
		Recipient:      recipient,
		Route:          *route,
		Amount:         new(big.Int).SetInt64(10_000_000),
		RefundAddress:  &refundAddress,
		FeePolicy:      nil,
		MaxSlippageBps: nil,
	})
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
