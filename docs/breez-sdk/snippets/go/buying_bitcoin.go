package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func BuyBitcoin(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: buy-bitcoin
	// Optionally, lock the purchase to a specific amount
	optionalLockedAmountSat := uint64(100_000)
	// Optionally, set a redirect URL for after the purchase is completed
	optionalRedirectUrl := "https://example.com/purchase-complete"

	request := breez_sdk_spark.BuyBitcoinRequest{
		LockedAmountSat: &optionalLockedAmountSat,
		RedirectUrl:     &optionalRedirectUrl,
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
