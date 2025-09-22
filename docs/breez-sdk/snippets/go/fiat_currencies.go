package example

import (
	"github.com/breez/breez-sdk-spark-go/breez_sdk_common"
	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func ListFiatCurrencies(sdk *breez_sdk_spark.BreezSdk) (*[]breez_sdk_common.FiatCurrency, error) {
	// ANCHOR: list-fiat-currencies
	response, err := sdk.ListFiatCurrencies()

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}
	// ANCHOR_END: list-fiat-currencies
	return &response.Currencies, nil
}

func ListFiatRates(sdk *breez_sdk_spark.BreezSdk) (*[]breez_sdk_common.Rate, error) {
	// ANCHOR: list-fiat-rates
	response, err := sdk.ListFiatRates()
	
	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}
	// ANCHOR_END: list-fiat-rates
	return &response.Rates, nil
}
