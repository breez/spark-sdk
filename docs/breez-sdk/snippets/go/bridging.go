package example

import (
	"errors"
	"log"
	"math/big"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func NewClient() (*breez_sdk_spark.BridgingClient, error) {
	// ANCHOR: new-bridging-client
	config := breez_sdk_spark.DefaultBridgingConfig("<your api key>")
	client, err := breez_sdk_spark.NewBridgingClient(config)
	if err != nil {
		return nil, err
	}
	// ANCHOR_END: new-bridging-client
	return client, nil
}

func ClientFromSdk(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.BridgingClient, error) {
	// ANCHOR: get-bridging-client
	client, err := sdk.GetBridgingClient()
	if err != nil {
		return nil, err
	}
	// ANCHOR_END: get-bridging-client
	return client, nil
}

func CreateBridge(client *breez_sdk_spark.BridgingClient) error {
	// ANCHOR: create-bridge
	// Who to pay: a Spark address, a Lightning address or an EVM/Solana/Tron
	// address.
	recipient := "<recipient>"

	// List the routes that pay them and pick what the payer pays in, e.g.
	// USDC on Base.
	routesResponse, err := client.GetRoutes(breez_sdk_spark.GetBridgeRoutesRequest{
		Recipient: recipient,
	})
	if err != nil {
		return err
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
		return errors.New("no route from USDC on Base")
	}

	// Ask for $10. The amount is in the recipient's stablecoin when they are
	// paid in one, the payer's otherwise (6 decimals for USDC). The deposit is
	// refunded to the payer's address if the recipient can't be paid.
	refundAddress := "<payer address on Base>"
	response, err := client.CreateBridge(breez_sdk_spark.CreateBridgeRequest{
		Recipient: recipient,
		Route:     *route,
		Amount:    new(big.Int).SetInt64(10_000_000),
		// Required to pay a Lightning address.
		RefundAddress:  &refundAddress,
		FeePolicy:      nil,
		MaxSlippageBps: nil,
	})
	if err != nil {
		return err
	}

	// Show the payer what to pay, and store the bridge id and status token to
	// look up the status later.
	log.Printf("Pay with: %s", response.PaymentRequest)
	log.Printf("Bridge id: %s", response.BridgeId)
	// ANCHOR_END: create-bridge
	return nil
}

func GetBridgeStatus(
	client *breez_sdk_spark.BridgingClient,
	bridgeId string,
	statusToken *string,
) error {
	// ANCHOR: get-bridge-status
	response, err := client.GetBridgeStatus(breez_sdk_spark.GetBridgeStatusRequest{
		BridgeId:    bridgeId,
		StatusToken: statusToken,
	})
	if err != nil {
		return err
	}
	switch response.Status {
	case breez_sdk_spark.BridgeStatusCompleted:
		if response.AmountOut != nil {
			log.Printf("Delivered %v", *response.AmountOut)
		}
	case breez_sdk_spark.BridgeStatusRefunded,
		breez_sdk_spark.BridgeStatusFailed,
		breez_sdk_spark.BridgeStatusExpired:
		log.Printf("Not delivered: %v", response.Status)
	default:
		log.Printf("Not final yet: %v", response.Status)
	}
	// ANCHOR_END: get-bridge-status
	return nil
}
