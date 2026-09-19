# Converting tokens

Token conversion enables payments to be made without holding the required asset by converting on-the-fly between Bitcoin and tokens using the Flashnet protocol.

## Fetching conversion limits

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.fetch_conversion_limits

Before performing a conversion, you can fetch the minimum amounts required for the conversion. The limits depend on the conversion direction:

- **Bitcoin to token**: Minimum Bitcoin amount (in satoshis) and minimum token amount to receive (in token base units)
- **Token to Bitcoin**: Minimum token amount (in token base units) and minimum Bitcoin amount to receive (in satoshis)

```python
try:
    # Fetch limits for converting Bitcoin to a token
    from_bitcoin_response = await sdk.fetch_conversion_limits(
        request=FetchConversionLimitsRequest(
            conversion_type=ConversionType.FROM_BITCOIN(),
            token_identifier="<token identifier>",
        )
    )

    if from_bitcoin_response.min_from_amount is not None:
        print(f"Minimum BTC to convert: {from_bitcoin_response.min_from_amount} sats")
    if from_bitcoin_response.min_to_amount is not None:
        print(f"Minimum tokens to receive: {from_bitcoin_response.min_to_amount} base units")

    # Fetch limits for converting a token to Bitcoin
    to_bitcoin_response = await sdk.fetch_conversion_limits(
        request=FetchConversionLimitsRequest(
            conversion_type=ConversionType.TO_BITCOIN(
                from_token_identifier="<token identifier>"
            ),
            token_identifier=None,
        )
    )

    if to_bitcoin_response.min_from_amount is not None:
        print(f"Minimum tokens to convert: {to_bitcoin_response.min_from_amount} base units")
    if to_bitcoin_response.min_to_amount is not None:
        print(f"Minimum BTC to receive: {to_bitcoin_response.min_to_amount} sats")
except Exception as error:
    logging.error(error)
    raise
```



**Developer note**

Amounts are denominated in satoshis for Bitcoin (1 BTC = 100,000,000 sats) and in token base units for tokens. Token base units depend on the token's decimal specification.

## Converting Bitcoin to tokens

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_payment

Token conversion enables payments of tokens like <a href="https://sparkscan.io/token/3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca?network=mainnet" target="_blank">USDB</a> to be made without holding the token, but instead using Bitcoin.

To do so, when preparing to send a payment, set the conversion options. The conversion will first calculate the Bitcoin amount needed to be converted into the token, convert Bitcoin into that token amount, and then finally complete the payment.

```python
try:
    payment_request = "<spark address or invoice>"
    token_identifier = "<token identifier>"
    amount = 1_000
    # Set to use Bitcoin funds to pay via conversion
    optional_max_slippage_bps = 50
    optional_completion_timeout_secs = 30
    conversion_options = ConversionOptions(
        conversion_type=ConversionType.FROM_BITCOIN(),
        max_slippage_bps=optional_max_slippage_bps,
        completion_timeout_secs=optional_completion_timeout_secs,
    )

    prepare_response = await sdk.prepare_send_payment(
        request=PrepareSendPaymentRequest(
            payment_request=PaymentRequest.INPUT(input=payment_request),
            amount=amount,
            token_identifier=token_identifier,
            conversion_options=conversion_options,
            fee_policy=None,
        )
    )

    # If the fees are acceptable, continue to send the token payment
    if prepare_response.conversion_estimate is not None:
        conversion_estimate = prepare_response.conversion_estimate
        logging.debug(
            f"Estimated conversion: {conversion_estimate.amount_in}"
            f" token units → {conversion_estimate.amount_out} sats"
        )
        logging.debug(
            f"Estimated conversion fee: {conversion_estimate.fee} token units"
        )
except Exception as error:
    logging.error(error)
    raise
```



**Developer note**

When a conversion fails due to exceeding the maximum slippage, the conversion will be refunded automatically.

**Developer note**

The conversion may result in some token balance remaining in the wallet after the payment is sent. This remaining balance is to account for slippage in the conversion.

## Converting tokens to Bitcoin

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_payment

Token conversion also enables Bitcoin payments to be made without holding the required Bitcoin, but instead using a supported token asset like <a href="https://sparkscan.io/token/3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca?network=mainnet" target="_blank">USDB</a>.

To do so, when preparing to send a payment, set the conversion options. The conversion will first calculate the amount needed to be converted into Bitcoin, convert the token into that Bitcoin amount, and then finally complete the payment.

```python
payment_request = "<payment request>"
# Set to use token funds to pay via conversion
optional_max_slippage_bps = 50
optional_completion_timeout_secs = 30
conversion_options = ConversionOptions(
    conversion_type=ConversionType.TO_BITCOIN(
        from_token_identifier="<token identifier>"
    ),
    max_slippage_bps=optional_max_slippage_bps,
    completion_timeout_secs=optional_completion_timeout_secs,
)
try:
    request = PrepareSendPaymentRequest(
        payment_request=PaymentRequest.INPUT(input=payment_request),
        amount=None,
        token_identifier=None,
        conversion_options=conversion_options,
        fee_policy=None,
    )
    prepare_response = await sdk.prepare_send_payment(request=request)

    # If the fees are acceptable, continue to create the Send Payment
    if prepare_response.conversion_estimate is not None:
        conversion_estimate = prepare_response.conversion_estimate
        logging.debug(
            f"Estimated conversion: {conversion_estimate.amount_in}"
            f" token units → {conversion_estimate.amount_out} sats"
        )
        logging.debug(
            f"Estimated conversion fee: {conversion_estimate.fee} token units"
        )
except Exception as error:
    logging.error(error)
    raise
```



**Developer note**

When a conversion fails due to exceeding the maximum slippage, the conversion will be refunded automatically.

**Developer note**

The conversion may result in some Bitcoin remaining in the wallet after the payment is sent. This remaining Bitcoin is to account for slippage in the conversion.
