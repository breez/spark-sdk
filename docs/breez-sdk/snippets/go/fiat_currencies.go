package example

import (
	"errors"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func ListFiatCurrencies(client *breez_sdk_spark.BreezClient) (*[]breez_sdk_spark.FiatCurrency, error) {
	// ANCHOR: list-fiat-currencies
	response, err := client.ListFiatCurrencies()

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return nil, err
	}
	// ANCHOR_END: list-fiat-currencies
	return &response.Currencies, nil
}

func ListFiatRates(client *breez_sdk_spark.BreezClient) (*[]breez_sdk_spark.Rate, error) {
	// ANCHOR: list-fiat-rates
	response, err := client.ListFiatRates()

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return nil, err
	}
	// ANCHOR_END: list-fiat-rates
	return &response.Rates, nil
}
