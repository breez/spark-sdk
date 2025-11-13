package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func ConfigureSdk() {
	// ANCHOR: max-deposit-claim-fee
	// Create the default config
	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
	apiKey := "<breez api key>"
	config.ApiKey = &apiKey

	// Disable automatic claiming
	config.MaxDepositClaimFee = nil

	// Set a maximum feerate of 10 sat/vB
	feeRateInterface := breez_sdk_spark.Fee(breez_sdk_spark.FeeRate{SatPerVbyte: 10})
	config.MaxDepositClaimFee = &feeRateInterface

	// Set a maximum fee of 1000 sat
	feeFixedInterface := breez_sdk_spark.Fee(breez_sdk_spark.FeeFixed{Amount: 1000})
	config.MaxDepositClaimFee = &feeFixedInterface
	// ANCHOR_END: max-deposit-claim-fee
	log.Printf("Config: %+v", config)
}
