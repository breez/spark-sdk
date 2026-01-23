package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func BuyBitcoinBasic(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: buy-bitcoin-basic
	request := breez_sdk_spark.BuyBitcoinRequest{
		Address:          "bc1qexample...", // Your Bitcoin address
		LockedAmountSat:  nil,
		MaxAmountSat:     nil,
		RedirectUrl:      nil,
	}

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
	lockedAmount := uint64(100_000) // Pre-fill with 100,000 sats
	request := breez_sdk_spark.BuyBitcoinRequest{
		Address:          "bc1qexample...",
		LockedAmountSat:  &lockedAmount,
		MaxAmountSat:     nil,
		RedirectUrl:      nil,
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

func BuyBitcoinWithLimits(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: buy-bitcoin-with-limits
	// Set both a locked amount and maximum amount
	lockedAmount := uint64(50_000)  // Pre-fill with 50,000 sats
	maxAmount := uint64(500_000)    // Limit to 500,000 sats max
	request := breez_sdk_spark.BuyBitcoinRequest{
		Address:          "bc1qexample...",
		LockedAmountSat:  &lockedAmount,
		MaxAmountSat:     &maxAmount,
		RedirectUrl:      nil,
	}

	response, err := sdk.BuyBitcoin(request)
	if err != nil {
		return err
	}

	log.Printf("Open this URL in a browser to complete the purchase:")
	log.Printf("%v", response.Url)
	// ANCHOR_END: buy-bitcoin-with-limits
	return nil
}

func BuyBitcoinWithRedirect(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: buy-bitcoin-with-redirect
	// Provide a custom redirect URL for after the purchase
	lockedAmount := uint64(100_000)
	redirectUrl := "https://example.com/purchase-complete"
	request := breez_sdk_spark.BuyBitcoinRequest{
		Address:          "bc1qexample...",
		LockedAmountSat:  &lockedAmount,
		MaxAmountSat:     nil,
		RedirectUrl:      &redirectUrl,
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
