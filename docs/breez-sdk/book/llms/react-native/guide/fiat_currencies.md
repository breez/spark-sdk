# Supporting fiat currencies

## List fiat currencies

You can get the full details of supported fiat currencies, such as symbols and localized names:

```typescript
const response = await sdk.listFiatCurrencies()
```



## Fetch fiat rates

To get the current BTC rate in the various supported fiat currencies:

```typescript
const response = await sdk.listFiatRates()
```
