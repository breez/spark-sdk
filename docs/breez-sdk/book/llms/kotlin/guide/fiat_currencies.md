# Supporting fiat currencies

## List fiat currencies

You can get the full details of supported fiat currencies, such as symbols and localized names:

```kotlin
try {
    val response = sdk.listFiatCurrencies()
} catch (e: Exception) {
    // handle error
}
```



## Fetch fiat rates

To get the current BTC rate in the various supported fiat currencies:

```kotlin
try {
    val response = sdk.listFiatRates()
} catch (e: Exception) {
    // handle error
}
```
