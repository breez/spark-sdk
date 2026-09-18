# Supporting fiat currencies

## List fiat currencies

You can get the full details of supported fiat currencies, such as symbols and localized names:

```csharp
var response = await sdk.ListFiatCurrencies();
```



## Fetch fiat rates

To get the current BTC rate in the various supported fiat currencies:

```csharp
var response = await sdk.ListFiatRates();
```
