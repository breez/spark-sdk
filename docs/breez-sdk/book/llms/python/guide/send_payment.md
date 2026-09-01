# Sending payments

Once the SDK is initialized, you can directly begin sending payments. The send process takes two steps:

1. [Preparing the Payment](send_payment.md#preparing-payments)
2. [Sending the Payment](send_payment.md#sending-payments)

For sending payments via LNURL, see [LNURL-Pay](lnurl_pay.md).

## Preparing Payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_payment

During the prepare step, the SDK ensures that the inputs are valid with respect to the payment request type,
and also returns the fees related to the payment so they can be confirmed.

The payment request field supports Lightning invoices, Bitcoin addresses, Spark addresses and Spark invoices.

**Developer note**

Payments can be sent without holding Bitcoin by converting on-the-fly as a step before sending a payment. See <a href="./token_conversion.md">Converting tokens</a> for more information.

### Lightning

#### BOLT11 invoice

For BOLT11 invoices the amount can be optionally set. It is only required if the invoice doesn't specify an amount. If the invoice specifies an amount, providing a different amount is not supported.

If the invoice also contains a Spark address, the payment can be sent directly via a Spark transfer instead. When this is the case, the prepare response includes the Spark transfer fee. Note that only one fee is paid: either the Lightning fee or the Spark transfer fee, depending on which payment method is ultimately used. See [Lightning](send_payment.md#lightning-1) for how to select the payment method.

```python
payment_request = "<bolt11 invoice>"
optional_amount_sats = 5_000
try:
    request = PrepareSendPaymentRequest(
        payment_request=PaymentRequest.INPUT(input=payment_request),
        amount=optional_amount_sats,
        token_identifier=None,
        conversion_options=None,
        fee_policy=None,
    )
    prepare_response = await sdk.prepare_send_payment(request=request)

    # If the fees are acceptable, continue to create the Send Payment
    if isinstance(
        prepare_response.payment_method, SendPaymentMethod.BOLT11_INVOICE
    ):
        # Fees to pay via Lightning
        lightning_fee_sats = prepare_response.payment_method.lightning_fee_sats
        # Or fees to pay (if available) via a Spark transfer
        spark_transfer_fee_sats = (
            prepare_response.payment_method.spark_transfer_fee_sats
        )
        logging.debug(f"Lightning Fees: {lightning_fee_sats} sats")
        logging.debug(f"Spark Transfer Fees: {spark_transfer_fee_sats} sats")
except Exception as error:
    logging.error(error)
    raise
```



### Bitcoin

For Bitcoin addresses, the amount must be set in the request. The prepare response includes fee quotes for three payment speeds: Slow, Medium, and Fast.

```python
payment_request = "<bitcoin address>"
amount_sats = 50_000
try:
    request = PrepareSendPaymentRequest(
        payment_request=PaymentRequest.INPUT(input=payment_request),
        amount=amount_sats,
        token_identifier=None,
        conversion_options=None,
        fee_policy=None,
    )
    prepare_response = await sdk.prepare_send_payment(request=request)

    # Review the fee quote for each confirmation speed
    if isinstance(
        prepare_response.payment_method, SendPaymentMethod.BITCOIN_ADDRESS
    ):
        fee_quote = prepare_response.payment_method.fee_quote
        slow_fee_sats = (
            fee_quote.speed_slow.user_fee_sat
            + fee_quote.speed_slow.l1_broadcast_fee_sat
        )
        medium_fee_sats = (
            fee_quote.speed_medium.user_fee_sat
            + fee_quote.speed_medium.l1_broadcast_fee_sat
        )
        fast_fee_sats = (
            fee_quote.speed_fast.user_fee_sat
            + fee_quote.speed_fast.l1_broadcast_fee_sat
        )
        logging.debug(f"Slow fee: {slow_fee_sats} sats")
        logging.debug(f"Medium fee: {medium_fee_sats} sats")
        logging.debug(f"Fast fee: {fast_fee_sats} sats")
except Exception as error:
    logging.error(error)
    raise
```



### Spark

#### Spark address

For Spark addresses, the amount must be set in the request. Sending to a Spark address uses a direct Spark transfer.

```python
payment_request = "<spark address>"
amount_sats = 50_000
try:
    request = PrepareSendPaymentRequest(
        payment_request=PaymentRequest.INPUT(input=payment_request),
        amount=amount_sats,
        token_identifier=None,
        conversion_options=None,
        fee_policy=None,
    )
    prepare_response = await sdk.prepare_send_payment(request=request)

    # If the fees are acceptable, continue to create the Send Payment
    if isinstance(prepare_response.payment_method, SendPaymentMethod.SPARK_ADDRESS):
        fee = prepare_response.payment_method.fee
        logging.debug(f"Fees: {fee} sats")
except Exception as error:
    logging.error(error)
    raise
```



#### Spark invoice

For Spark invoices, the amount can be optionally set. It is only required if the invoice doesn't specify an amount. If the invoice specifies an amount, providing a different amount is not supported.

**Developer note**

Spark invoices may require a token (non-Bitcoin) as the payment asset. To determine the requirements of a Spark invoice and any restrictions it may impose, see the <a href="./parse.md">Parsing inputs</a> page. To learn more about tokens, see the <a href="./tokens.md">Handling tokens</a> page.

```python
payment_request = "<spark invoice>"
optional_amount_sats = 50_000
try:
    request = PrepareSendPaymentRequest(
        payment_request=PaymentRequest.INPUT(input=payment_request),
        amount=optional_amount_sats,
        token_identifier=None,
        conversion_options=None,
        fee_policy=None,
    )
    prepare_response = await sdk.prepare_send_payment(request=request)

    # If the fees are acceptable, continue to create the Send Payment
    if isinstance(prepare_response.payment_method, SendPaymentMethod.SPARK_INVOICE):
        fee = prepare_response.payment_method.fee
        logging.debug(f"Fees: {fee} sats")
except Exception as error:
    logging.error(error)
    raise
```



### USDC/USDT

Send USDC or USDT from a Spark wallet to a recipient on one of several supported chains: Ethereum-family chains (Arbitrum, Base, and similar EVM networks), Solana, and Tron. The source on the Spark side is BTC sats or USDB. This feature must be enabled in [the SDK configuration](./config.md#send-usdc-usdt) before using. See [Send USDC/USDT](./cross_chain.md) for provider details and the status lifecycle.

After [parsing](./parse.md) the recipient address into `InputType.CROSS_CHAIN_ADDRESS`, call `get_cross_chain_routes` with `CrossChainRouteFilter.SEND` carrying the parsed `CrossChainAddressDetails`. The returned `CrossChainRoutePair`s name the provider, destination chain and asset, decimals, optional token contract address, and which source assets (BTC sats or USDB) each route accepts.

```python
input_str = "<recipient address>"
try:
    parsed = await sdk.parse(input=input_str)
    if not isinstance(parsed, InputType.CROSS_CHAIN_ADDRESS):
        raise ValueError("Not a cross-chain address")
    address_details = parsed[0]

    routes = await sdk.get_cross_chain_routes(
        filter=CrossChainRouteFilter.SEND(address_details=address_details)
    )

    for route in routes:
        logging.debug(
            f"Route via {route.provider}: {route.chain}/{route.asset}"
        )
except Exception as error:
    logging.error(error)
    raise
```



Build `PaymentRequest.CROSS_CHAIN` with the recipient address, the chosen route, and an optional `max_slippage_bps` (10 to 500 basis points). The amount on the prepare request is denominated in the source asset's base units: sats for a BTC source, USDB base units for a USDB source.

The prepare response carries a quote `expires_at` timestamp. Re-prepare and pick a fresh route if it lapses before send.

```python
# Optionally set the maximum slippage in basis points (10 to 500)
optional_max_slippage_bps = 100
try:
    request = PrepareSendPaymentRequest(
        payment_request=PaymentRequest.CROSS_CHAIN(
            address=address_details.address,
            route=route,
            max_slippage_bps=optional_max_slippage_bps,
            target_overpay_bps=None,
        ),
        amount=50_000,
        token_identifier=None,
        conversion_options=None,
        fee_policy=None,
    )
    prepare_response = await sdk.prepare_send_payment(request=request)

    if isinstance(
        prepare_response.payment_method, SendPaymentMethod.CROSS_CHAIN_ADDRESS
    ):
        method = prepare_response.payment_method
        logging.debug(f"Amount in: {method.amount_in}")
        logging.debug(f"Estimated out: {method.estimated_out}")
        logging.debug(f"Provider fee: {method.fee_amount}")
        logging.debug(f"Quote expires at: {method.expires_at}")
except Exception as error:
    logging.error(error)
    raise
```



## Fee Policy

By default, fees are added on top of the amount (`FeePolicy.FEES_EXCLUDED`). Use `FeePolicy.FEES_INCLUDED` to deduct fees from the amount instead—the receiver gets the amount minus fees.

This is particularly useful when you want to spend your entire balance in a single payment—simply provide your full balance as the amount. Note: `FeePolicy.FEES_INCLUDED` is not compatible with payment requests that specify an amount (e.g., BOLT11 invoices and Spark invoices with amount).

```python
# By default (FeePolicy.FEES_EXCLUDED), fees are added on top of the amount.
# Use FeePolicy.FEES_INCLUDED to deduct fees from the amount instead.
# The receiver gets amount minus fees.
payment_request = "<payment request>"
amount_sats = 50_000
try:
    request = PrepareSendPaymentRequest(
        payment_request=PaymentRequest.INPUT(input=payment_request),
        amount=amount_sats,
        token_identifier=None,
        conversion_options=None,
        fee_policy=FeePolicy.FEES_INCLUDED,
    )
    prepare_response = await sdk.prepare_send_payment(request=request)

    # The response shows the fee policy used
    logging.debug(f"Fee policy: {prepare_response.fee_policy}")
    logging.debug(f"Amount: {prepare_response.amount}")
    # The receiver gets amount - fees (fees are available in prepare_response.payment_method)
except Exception as error:
    logging.error(error)
    raise
```



When [stable balance](./stable_balance.md) is active, you can send your entire wallet balance — both the token balance and any remaining sats — by combining `FeePolicy.FEES_INCLUDED` with `ConversionType.TO_BITCOIN` conversion options. See [Sending entire balance](./stable_balance.md#sending-entire-balance) for details.

```python
payment_request = "<payment request>"
token_identifier = "<token identifier>"
try:
    info = await sdk.get_info(request=GetInfoRequest(ensure_synced=False))
    token_balance = info.token_balances.get(token_identifier)
    if token_balance is None:
        raise ValueError("Token balance not found")

    conversion_options = ConversionOptions(
        conversion_type=ConversionType.TO_BITCOIN(
            from_token_identifier=token_identifier
        ),
    )

    request = PrepareSendPaymentRequest(
        payment_request=PaymentRequest.INPUT(input=payment_request),
        amount=token_balance.balance,
        token_identifier=token_identifier,
        conversion_options=conversion_options,
        fee_policy=FeePolicy.FEES_INCLUDED,
    )
    prepare_response = await sdk.prepare_send_payment(request=request)

    # The response amount is the estimated total sats available
    # (converted sats + existing sat balance)
    logging.debug(f"Total sats available: {prepare_response.amount}")

    if prepare_response.conversion_estimate is not None:
        conversion_estimate = prepare_response.conversion_estimate
        logging.debug(
            f"Converting {conversion_estimate.amount_in}"
            f" token units → ~{conversion_estimate.amount_out} sats"
        )
        logging.debug(
            f"Conversion fee: {conversion_estimate.fee} token units"
        )
except Exception as error:
    logging.error(error)
    raise
```



## Sending Payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.send_payment

Once the payment has been prepared and the fees are accepted, the payment can be sent by passing:

- **Prepare Response** - The response from the [Preparing the Payment](send_payment.md#preparing-payments) step.
- **Options** - Any payment method specific options for the payment (see below).
- **Idempotency Key** - An optional UUID that identifies the payment. If set, providing the same idempotency key for multiple requests will ensure that only one payment is made.

### Lightning

In the optional send payment options for BOLT11 invoices, you can set:

- **Prefer Spark** - Set the preference to use Spark to transfer the payment if the invoice contains a Spark address. By default, using Spark transfers are disabled.
- **Completion Timeout** - By default, this function returns immediately. You can override this behavior by specifying a completion timeout in seconds. If the timeout is reached, a pending payment object is returned. If the payment completes within the timeout, the completed payment object is returned.

```python
try:
    options = SendPaymentOptions.BOLT11_INVOICE(
        prefer_spark=False, completion_timeout_secs=10
    )
    optional_idempotency_key = "<idempotency key uuid>"
    request = SendPaymentRequest(
        prepare_response=prepare_response,
        options=options,
        idempotency_key=optional_idempotency_key,
    )
    send_response = await sdk.send_payment(request=request)
    payment = send_response.payment
except Exception as error:
    logging.error(error)
    raise
```



### Bitcoin

In the optional send payment options for Bitcoin addresses, you can set:

- **Confirmation Speed** - The priority that the Bitcoin transaction confirms, that also effects the fee paid. By default, it is set to Fast.

```python
try:
    # Select the confirmation speed for the on-chain transaction
    options = SendPaymentOptions.BITCOIN_ADDRESS(
        confirmation_speed=OnchainConfirmationSpeed.MEDIUM
    )
    optional_idempotency_key = "<idempotency key uuid>"
    request = SendPaymentRequest(
        prepare_response=prepare_response,
        options=options,
        idempotency_key=optional_idempotency_key,
    )
    send_response = await sdk.send_payment(request=request)
    payment = send_response.payment
except Exception as error:
    logging.error(error)
    raise
```



### Spark

In the optional send payment options for Spark addresses, you can set:

- **HTLC Options** - Enables Spark HTLC payments, which are an advanced feature that allows for conditional payments. See the [Spark HTLC Payments](htlcs.md) page for more details and example usage.

```python
try:
    optional_idempotency_key = "<idempotency key uuid>"
    request = SendPaymentRequest(
        prepare_response=prepare_response, idempotency_key=optional_idempotency_key
    )
    send_response = await sdk.send_payment(request=request)
    payment = send_response.payment
except Exception as error:
    logging.error(error)
    raise
```



### USDC/USDT

Send USDC/USDT has no additional send payment options.

```python
# Only valid for sends with no token leg (see Retry safety).
optional_idempotency_key = "<idempotency key uuid>"
try:
    request = SendPaymentRequest(
        prepare_response=prepare_response,
        options=None,
        idempotency_key=optional_idempotency_key,
    )
    send_response = await sdk.send_payment(request=request)
    payment = send_response.payment
    logging.debug(f"Payment: {payment}")
except Exception as error:
    logging.error(error)
    raise
```



## Event Flows

Once a send payment is initiated, you can follow and react to the different payment events using the guide below for each payment method. See [listening to events](/guide/events.html) for how to subscribe to events. 

The `SdkEvent.SYNCED` event is also emitted as the SDK syncs in the background. See [fetching the balance](/llms/python/guide/get_info.md) for the recommended pattern for refreshing the balance and payments list.

#### Lightning

| Event                | Description                                                                       | UX Suggestion                                    |
| -------------------- | --------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer has been started. Awaiting Lightning payment completion.       | Show payment as pending.                         |
| **PaymentSucceeded** | The Lightning invoice has been paid either over Lightning or via a Spark transfer | Show the payment as complete and call `get_info` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/python/guide/get_info.md). |
| **PaymentFailed**    | The attempt to pay the Lightning invoice failed.                                  |                                                  |

#### Bitcoin

| Event                | Description                                                                   | UX Suggestion                                    |
| -------------------- | ----------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer has been started. Awaiting on-chain withdrawal completion. | Show payment as pending.                         |
| **PaymentSucceeded** | The payment amount was successfully withdrawn on-chain.                       | Show the payment as complete and call `get_info` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/python/guide/get_info.md). |

#### Spark

| Event                | Description                     | UX Suggestion                                    |
| -------------------- | ------------------------------- | ------------------------------------------------ |
| **PaymentSucceeded** | The Spark transfer is complete. | Show the payment as complete and call `get_info` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/python/guide/get_info.md). |

#### USDC/USDT

| Event                | Description                                                                                              | UX Suggestion                                    |
| -------------------- | -------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The deposit transfer has been submitted to the provider. The cross-chain leg is awaiting settlement.     | Show payment as pending; the bridge leg may take several minutes depending on the provider and destination chain. |
| **PaymentSucceeded** | The provider reports the cross-chain order terminal. The amount actually delivered to the recipient is carried on the conversion info. | Show the payment as complete and call `get_info` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/python/guide/get_info.md). |
