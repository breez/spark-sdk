# Listing payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

To view your payment history, you can list all the payments that have been sent and received.

```python
response = await sdk.list_payments(request=ListPaymentsRequest())
payments = response.payments
```



## Filtering Payments

When listing payments you can also filter and page the results.

```python
# Filter by asset (Bitcoin or Token)
asset_filter = AssetFilter.TOKEN(token_identifier="token_identifier_here")
# To filter by Bitcoin instead:
# asset_filter = AssetFilter.BITCOIN

request = ListPaymentsRequest(
    # Filter by payment type
    type_filter=[PaymentType.SEND, PaymentType.RECEIVE],
    # Filter by status
    status_filter=[PaymentStatus.COMPLETED],
    asset_filter=asset_filter,
    # Time range filters
    from_timestamp=1704067200,  # Unix timestamp
    to_timestamp=1735689600,    # Unix timestamp
    # Pagination
    offset=0,
    limit=50,
    # Sort order (true = oldest first, false = newest first)
    sort_ascending=False
)
response = await sdk.list_payments(request=request)
payments = response.payments
```



## Get Payment

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_payment

You can also retrieve a single payment using the payment id:

```python
payment_id = "<payment id>"
response = await sdk.get_payment(
    request=GetPaymentRequest(payment_id=payment_id)
)
payment = response.payment
```
