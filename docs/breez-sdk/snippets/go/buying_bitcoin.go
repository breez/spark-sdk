package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func BuyBitcoinBasic(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: buy-bitcoin-basic
	// Buy Bitcoin using the SDK's auto-generated deposit address
	request := breez_sdk_spark.BuyBitcoinRequest{}

	response, err := sdk.BuyBitcoin(request)
	if err != nil {
		return err
	}

	log.Printf("Open this URL in a browser to complete the purchase:")
	log.Printf("%v", response.Url)
	// ANCHOR_END: buy-bitcoin-basic
	return nil
}

func BuyBitcoinWithAmount(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: buy-bitcoin-with-amount
	// Lock the purchase to a specific amount (e.g., 0.001 BTC = 100,000 sats)
	lockedAmountSat := uint64(100_000)
	request := breez_sdk_spark.BuyBitcoinRequest{
		LockedAmountSat: &lockedAmountSat,
	}

	response, err := sdk.BuyBitcoin(request)
	if err != nil {
		return err
	}

	log.Printf("Open this URL in a browser to complete the purchase:")
	log.Printf("%v", response.Url)
	// ANCHOR_END: buy-bitcoin-with-amount
	return nil
}

func BuyBitcoinWithRedirect(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: buy-bitcoin-with-redirect
	// Provide a custom redirect URL for after the purchase
	lockedAmountSat := uint64(100_000)
	redirectUrl := "https://example.com/purchase-complete"
	request := breez_sdk_spark.BuyBitcoinRequest{
		LockedAmountSat: &lockedAmountSat,
		RedirectUrl:     &redirectUrl,
	}

	response, err := sdk.BuyBitcoin(request)
	if err != nil {
		return err
	}

	log.Printf("Open this URL in a browser to complete the purchase:")
	log.Printf("%v", response.Url)
	// ANCHOR_END: buy-bitcoin-with-redirect
	return nil
}

