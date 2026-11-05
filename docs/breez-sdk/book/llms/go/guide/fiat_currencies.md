# Supporting fiat currencies

## List fiat currencies

You can get the full details of supported fiat currencies, such as symbols and localized names:

```go
response, err := sdk.ListFiatCurrencies()

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}
```



## Fetch fiat rates

To get the current BTC rate in the various supported fiat currencies:

```go
response, err := sdk.ListFiatRates()

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}
```
