# Sending and receiving tokens

Spark supports tokens using the [BTKN protocol](https://docs.spark.money/learn/tokens/hello-btkn). The Breez SDK enables you to send and receive these tokens using the standard payments API.

## Fetching token balances

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_info

Token balances for all tokens currently held in the wallet can be retrieved along with general wallet information. Each token balance includes both the balance amount and the token metadata (identifier, name, ticker, issuer public key, etc.).

```python
try:
    # ensure_synced: True will ensure the SDK is synced with the Spark network
    # before returning the balance
    info = await sdk.get_info(request=GetInfoRequest(ensure_synced=False))

    # Token balances are a map of token identifier to balance
    token_balances = info.token_balances
    for token_id, token_balance in token_balances.items():
        print(f"Token ID: {token_id}")
        print(f"Balance: {token_balance.balance}")
        print(f"Name: {token_balance.token_metadata.name}")
        print(f"Ticker: {token_balance.token_metadata.ticker}")
        print(f"Decimals: {token_balance.token_metadata.decimals}")
except Exception as error:
    logging.error(error)
    raise
```



**Developer note**

Token balances are cached for fast responses. For details on ensuring up-to-date balances, see the <a href="./get_info.md#fetching-the-balance">Fetching the balance</a> section.

## Fetching token metadata

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_tokens_metadata

Token metadata can be fetched for specific tokens by providing their identifiers. This is especially useful for retrieving metadata for tokens that are not currently held in the wallet. The metadata is cached locally after the first fetch for faster subsequent lookups.

```python
try:
    response = await sdk.get_tokens_metadata(
        request=GetTokensMetadataRequest(
            token_identifiers=["<token identifier 1>", "<token identifier 2>"]
            )
        )

    tokens_metadata = response.tokens_metadata
    for token_metadata in tokens_metadata:
        print(f"Token ID: {token_metadata.identifier}")
        print(f"Name: {token_metadata.name}")
        print(f"Ticker: {token_metadata.ticker}")
        print(f"Decimals: {token_metadata.decimals}")
        print(f"Max Supply: {token_metadata.max_supply}")
        print(f"Is Freezable: {token_metadata.is_freezable}")
except Exception as error:
    logging.error(error)
    raise
```



## Receiving a token payment

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

Token payments can be received using either a Spark address or invoice. Using an invoice is useful to impose restrictions on the payment, such as the token to receive, amount, expiry, and who can pay it.

### Spark address

Token payments use the same Spark address as Bitcoin payments - no separate address is required. Your application can retrieve the Spark address as described in the [Receiving a payment](./receive_payment.md#spark) guide. The payer will use this address to send tokens to the wallet.

### Spark invoice

Spark token invoices can be created using the same API as Bitcoin Spark invoices. The only difference is that a token identifier is provided.

```python
try:
    token_identifier = "<token identifier>"
    optional_description = "<invoice description>"
    optional_amount = 5_000
    # Optionally set the expiry UNIX timestamp in seconds
    optional_expiry_time_seconds = 1716691200
    optional_sender_public_key = "<sender public key>"

    request = ReceivePaymentRequest(
        payment_method=ReceivePaymentMethod.SPARK_INVOICE(
            token_identifier=token_identifier,
            description=optional_description,
            amount=optional_amount,
            expiry_time=optional_expiry_time_seconds,
            sender_public_key=optional_sender_public_key,
        )
    )
    response = await sdk.receive_payment(request=request)

    payment_request = response.payment_request
    print(f"Payment request: {payment_request}")
    receive_fee = response.fee
    print(f"Fees: {receive_fee} token base units")
    return response
except Exception as error:
    logging.error(error)
    raise
```



## Sending a token payment

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_payment

To send tokens, provide a Spark address as the payment request. The token identifier must be specified in one of two ways:

1. **Using a Spark invoice**: If the payee provides a Spark address with an embedded token identifier and amount (a Spark invoice), the SDK automatically extracts and uses those values.
2. **Manual specification**: For a plain Spark address without embedded payment details, your application must provide both the token identifier and amount parameters when preparing the payment.

Your application can use the [parse](./parse.md) functionality to determine if a Spark address contains embedded token payment details before preparing the payment.

The code example below demonstrates manual specification. Follow the standard prepare/send payment flow as described in the [Sending a payment](./send_payment.md) guide.

**Developer note**

Payments can be sent without holding an asset by converting on-the-fly as a step before sending a payment. See <a href="./token_conversion.md">Converting tokens</a> for more information.

```python
try:
    payment_request = "<spark address or invoice>"
    token_identifier = "<token identifier>"
    amount = 1_000

    prepare_response = await sdk.prepare_send_payment(
        request=PrepareSendPaymentRequest(
            payment_request=PaymentRequest.INPUT(input=payment_request),
            amount=amount,
            token_identifier=token_identifier,
            conversion_options=None,
            fee_policy=None,
        )
    )

    # If the fees are acceptable, continue to send the token payment
    if isinstance(prepare_response.payment_method, SendPaymentMethod.SPARK_ADDRESS):
        print(f"Token ID: {prepare_response.payment_method.token_identifier}")
        print(f"Fees: {prepare_response.payment_method.fee} token base units")
    if isinstance(prepare_response.payment_method, SendPaymentMethod.SPARK_INVOICE):
        print(f"Token ID: {prepare_response.payment_method.token_identifier}")
        print(f"Fees: {prepare_response.payment_method.fee} token base units")

    # Send the token payment
    send_response = await sdk.send_payment(
        request=SendPaymentRequest(
            prepare_response=prepare_response,
            options=None,
        )
    )
    payment = send_response.payment
    print(f"Payment: {payment}")
except Exception as error:
    logging.error(error)
    raise
```



To pay several recipients at once, see [Sending to multiple recipients](./batch_send.md): one transaction can pay multiple payees, across several tokens, mixing Spark addresses and invoices. A batch that pays an invoice stays on one token.

## Listing token payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

Token payments are included in the regular payment history alongside Bitcoin payments. Your application can retrieve and distinguish token payments from other payment types using the standard payment listing functionality. See the [Listing payments](./list_payments.md) guide for more details.
