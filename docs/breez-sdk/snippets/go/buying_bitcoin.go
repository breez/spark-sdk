package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func BuyBitcoin(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: buy-bitcoin
	// Buy Bitcoin with funds deposited directly into the user's wallet.
	// Optionally lock the purchase to a specific amount and provide a redirect URL.
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
	// ANCHOR_END: buy-bitcoin
	return nil
}
