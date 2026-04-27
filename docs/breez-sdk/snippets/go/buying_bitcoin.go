package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func BuyBitcoin(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: buy-bitcoin
	optionalLockedAmountSat := uint64(100_000)
	optionalRedirectUrl := "https://example.com/purchase-complete"

	request := breez_sdk_spark.BuyBitcoinRequestMoonpay{
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

func BuyBitcoinViaCashapp(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: buy-bitcoin-cashapp
	// Cash App requires the amount to be specified up front.
	amountSats := uint64(50_000)

	request := breez_sdk_spark.BuyBitcoinRequestCashApp{
		AmountSats: amountSats,
	}

	response, err := sdk.BuyBitcoin(request)
	if err != nil {
		return err
	}

	log.Printf("Open this URL in Cash App to complete the purchase:")
	log.Printf("%v", response.Url)
	// ANCHOR_END: buy-bitcoin-cashapp
	return nil
}
