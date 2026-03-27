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
	defaultActiveLabel := "USDB"
	stableBalanceConfig := breez_sdk_spark.StableBalanceConfig{
		Tokens: []breez_sdk_spark.StableBalanceToken{
			{Label: "USDB", TokenIdentifier: "<token_identifier>"},
		},
		DefaultActiveLabel: &defaultActiveLabel,
		ThresholdSats:       &thresholdSats,
		MaxSlippageBps:      &maxSlippageBps,
	}
	config.StableBalanceConfig = &stableBalanceConfig
	// ANCHOR_END: stable-balance-config
	log.Printf("Config: %+v", config)
}

func ConfigureSparkConfig() {
	// ANCHOR: spark-config
	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)

	// Connect to a custom Spark environment
	schemaEndpoint := "graphql/spark/rc"
	sparkConfig := breez_sdk_spark.SparkConfig{
		CoordinatorIdentifier: "0000000000000000000000000000000000000000000000000000000000000001",
		Threshold:             2,
		SigningOperators: []breez_sdk_spark.SparkSigningOperator{
			{
				Id:                0,
				Identifier:        "0000000000000000000000000000000000000000000000000000000000000001",
				Address:           "https://0.spark.example.com",
				IdentityPublicKey: "03acd9a5a88db102730ff83dee69d69088cc4c9d93bbee893e90fd5051b7da9651",
			},
			{
				Id:                1,
				Identifier:        "0000000000000000000000000000000000000000000000000000000000000002",
				Address:           "https://1.spark.example.com",
				IdentityPublicKey: "02d2d103cacb1d6355efeab27637c74484e2a7459e49110c3fe885210369782e23",
			},
			{
				Id:                2,
				Identifier:        "0000000000000000000000000000000000000000000000000000000000000003",
				Address:           "https://2.spark.example.com",
				IdentityPublicKey: "0350f07ffc21bfd59d31e0a7a600e2995273938444447cb9bc4c75b8a895dbb853",
			},
		},
		SspConfig: breez_sdk_spark.SparkSspConfig{
			BaseUrl:           "https://api.example.com",
			IdentityPublicKey: "02e0b8d42c5d3b5fe4c5beb6ea796ab3bc8aaf28a3d3195407482c67e0b58228a5",
			SchemaEndpoint:    &schemaEndpoint,
		},
		ExpectedWithdrawBondSats:              10_000,
		ExpectedWithdrawRelativeBlockLocktime: 1_000,
	}
	config.SparkConfig = &sparkConfig
	// ANCHOR_END: spark-config
	log.Printf("Config: %+v", config)
}
