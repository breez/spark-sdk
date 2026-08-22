# Supporting fiat currencies

## List fiat currencies

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_fiat_currencies

You can get the full details of supported fiat currencies, such as symbols and localized names:

```swift
let response = try await sdk.listFiatCurrencies()
```



## Fetch fiat rates

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_fiat_rates

To get the current BTC rate in the various supported fiat currencies:

```swift
let response = try await sdk.listFiatRates()
```
