package example

import (
	"log"
	"math/big"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func BuyBitcoin(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: buy-bitcoin
	// Optionally, prefill the purchase amount
	optionalAmountSat := uint64(100_000)
	// Optionally, set a redirect URL for after the purchase is completed
	optionalRedirectUrl := "https://example.com/purchase-complete"

	var delivery breez_sdk_spark.MoonpayDelivery = breez_sdk_spark.MoonpayDeliveryBitcoin{
		AmountSat: &optionalAmountSat,
	}
	request := breez_sdk_spark.BuyBitcoinRequestMoonpay{
		Delivery:    &delivery,
		RedirectUrl: &optionalRedirectUrl,
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

func BuyBitcoinViaCrossChain(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: buy-bitcoin-cross-chain
	// USD amount to receive, in 6-decimal base units ($50)
	amount := new(big.Int).SetInt64(50_000_000)

	var delivery breez_sdk_spark.MoonpayDelivery = breez_sdk_spark.MoonpayDeliveryCrossChain{
		Amount:  amount,
		FeeMode: nil,
	}
	request := breez_sdk_spark.BuyBitcoinRequestMoonpay{
		Delivery:    &delivery,
		RedirectUrl: nil,
	}

	response, err := sdk.BuyBitcoin(request)
	if err != nil {
		return err
	}

	log.Printf("Open this URL in a browser to complete the purchase:")
	log.Printf("%v", response.Url)

	if info := response.CrossChainInfo; info != nil {
		log.Printf("USDC to buy: %v", info.DepositAmount)
		log.Printf(
			"Expected to receive: %v %s",
			info.ExpectedReceivedAmount, info.DestinationAsset,
		)
		log.Printf("Conversion fee: %v", info.ServiceFeeAmount)
	}
	// ANCHOR_END: buy-bitcoin-cross-chain
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
