# Supporting fiat currencies

## List fiat currencies

You can get the full details of supported fiat currencies, such as symbols and localized names:

```python
try:
    response = await sdk.list_fiat_currencies()
except Exception as error:
    print(error)
    raise
```



## Fetch fiat rates

To get the current BTC rate in the various supported fiat currencies:

```python
try:
    response = await sdk.list_fiat_rates()
except Exception as error:
    print(error)
    raise
```
