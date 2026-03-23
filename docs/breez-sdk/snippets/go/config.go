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
	feeRateInterface := breez_sdk_spark.MaxFee(breez_sdk_spark.MaxFeeRate{SatPerVbyte: 10})
	config.MaxDepositClaimFee = &feeRateInterface

	// Set a maximum fee of 1000 sat
	feeFixedInterface := breez_sdk_spark.MaxFee(breez_sdk_spark.MaxFeeFixed{Amount: 1000})
	config.MaxDepositClaimFee = &feeFixedInterface

	// Set the maximum fee to the fastest network recommended fee at the time of claim
	// with a leeway of 1 sats/vbyte
	networkRecommendedInterface := breez_sdk_spark.MaxFee(breez_sdk_spark.MaxFeeNetworkRecommended{LeewaySatPerVbyte: 1})
	config.MaxDepositClaimFee = &networkRecommendedInterface
	// ANCHOR_END: max-deposit-claim-fee
	log.Printf("Config: %+v", config)
}

func ConfigurePrivateEnabledDefault() {
	// ANCHOR: private-enabled-default
	// Disable Spark private mode by default
	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
	config.PrivateEnabledDefault = false
	// ANCHOR_END: private-enabled-default
	log.Printf("Config: %+v", config)
}

func ConfigureOptimizationConfiguration() {
	// ANCHOR: optimization-configuration
	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
	config.OptimizationConfig = breez_sdk_spark.OptimizationConfig{AutoEnabled: true, Multiplicity: 1}
	// ANCHOR_END: optimization-configuration
	log.Printf("Config: %+v", config)
}

func ConfigureStableBalance() {
	// ANCHOR: stable-balance-config
	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)

	// Enable stable balance with auto-conversion to a specific token
	thresholdSats := uint64(10_000)
	maxSlippageBps := uint32(100)
	reservedSats := uint64(1_000)
	stableBalanceConfig := breez_sdk_spark.StableBalanceConfig{
		TokenIdentifier: "<token_identifier>",
		ThresholdSats:   &thresholdSats,
		MaxSlippageBps:  &maxSlippageBps,
		ReservedSats:    &reservedSats,
	}
	config.StableBalanceConfig = &stableBalanceConfig
	// ANCHOR_END: stable-balance-config
	log.Printf("Config: %+v", config)
}
