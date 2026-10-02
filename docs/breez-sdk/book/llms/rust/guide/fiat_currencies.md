# Supporting fiat currencies

## List fiat currencies

You can get the full details of supported fiat currencies, such as symbols and localized names:

```rust
let response = sdk.list_fiat_currencies().await?;
```



## Fetch fiat rates

To get the current BTC rate in the various supported fiat currencies:

```rust
let response = sdk.list_fiat_rates().await?;
```
