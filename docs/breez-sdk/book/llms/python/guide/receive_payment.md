# Receiving payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

Once the SDK is initialized, you can directly begin receiving payments. The SDK currently supports three methods of receiving: Lightning, Bitcoin and Spark.

## Lightning

#### BOLT11 invoice

When receiving via Lightning, we can generate a BOLT11 invoice to be paid. Setting the invoice amount fixes the amount the sender should pay.

**Note:** the payment may fallback to a direct Spark payment (if the payer's client supports this).

```python
try:
    description = "<invoice description>"
    # Optionally set the invoice amount you wish the payer to send
    optional_amount_sats = 5_000
    # Optionally set the expiry duration in seconds
    optional_expiry_secs = 3600
    payment_method = ReceivePaymentMethod.BOLT11_INVOICE(
        description=description,
        amount_sats=optional_amount_sats,
        expiry_secs=optional_expiry_secs,
        payment_hash=None,
    )
    request = ReceivePaymentRequest(payment_method=payment_method)
    response = await sdk.receive_payment(request=request)

    payment_request = response.payment_request
    logging.debug(f"Payment Request: {payment_request}")
    receive_fee_sats = response.fee
    logging.debug(f"Fees: {receive_fee_sats} sats")
    return response
except Exception as error:
    logging.error(error)
    raise
```



#### LNURL-Pay & Lightning address

To receive via LNURL-Pay and/or a Lightning address, follow [these instructions](/llms/python/guide/receive_lnurl_pay.md).

> Note: Lightning payments work in Spark even if the receiver is offline. To understand how it works under the hood, read [this](https://docs.spark.money/learn/lightning).

## Bitcoin

For on-chain payments you can generate a Bitcoin deposit address to receive payments. By default the existing address is returned; you can optionally request a new address to rotate to a fresh one for improved privacy. All previously generated addresses remain monitored.

On-chain deposits go through the following lifecycle:

1. **Detected** — The SDK detects the deposit and emits a `SdkEvent.NEW_DEPOSITS` event. The deposit may or may not have sufficient confirmations to be claimed yet.
2. **Sufficient confirmations** — After **3 on-chain confirmations**, the deposit has sufficient confirmations and the SDK automatically attempts to claim it.
3. **Claimed or unclaimed** — If claiming succeeds, the funds are added to your balance. If it fails (e.g. fees too high), the deposit remains unclaimed and can be [manually claimed or refunded](/llms/python/guide/onchain_claims.md).

```python
try:
    new_address = None  # Set to True to get a new address
    request = ReceivePaymentRequest(
        payment_method=ReceivePaymentMethod.BITCOIN_ADDRESS(
            new_address=new_address)
    )
    response = await sdk.receive_payment(request=request)

    payment_request = response.payment_request
    logging.debug(f"Payment Request: {payment_request}")
    receive_fee_sats = response.fee
    logging.debug(f"Fees: {receive_fee_sats} sats")
    return response
except Exception as error:
    logging.error(error)
    raise
```



To track pending deposits, use `list_unclaimed_deposits` and filter by the `is_mature` field:

```python
try:
    request = ListUnclaimedDepositsRequest()
    response = await sdk.list_unclaimed_deposits(request=request)

    pending_deposits = [d for d in response.deposits if not d.is_mature]

    for deposit in pending_deposits:
        logging.info(f"Pending deposit: {deposit.txid}:{deposit.vout}")
        logging.info(f"Amount: {deposit.amount_sats} sats")
except Exception as error:
    logging.error(error)
    raise
```



## Spark

For payments between Spark users, you can use a Spark address or generate a Spark invoice to receive payments.

#### Spark address

Spark addresses are static.

```python
try:
    request = ReceivePaymentRequest(
        payment_method=ReceivePaymentMethod.SPARK_ADDRESS()
    )
    response = await sdk.receive_payment(request=request)

    payment_request = response.payment_request
    logging.debug(f"Payment Request: {payment_request}")
    receive_fee_sats = response.fee
    logging.debug(f"Fees: {receive_fee_sats} sats")
    return response
except Exception as error:
    logging.error(error)
    raise
```



#### Spark invoice

Spark invoices are single-use and may impose restrictions on the payment, such as amount, expiry, and who is able to pay it.

```python
try:
    optional_description = "<invoice description>"
    optional_amount_sats = 5_000
    # Optionally set the expiry UNIX timestamp in seconds
    optional_expiry_time_seconds = 1716691200
    optional_sender_public_key = "<sender public key>"

    request = ReceivePaymentRequest(
        payment_method=ReceivePaymentMethod.SPARK_INVOICE(
            description=optional_description,
            amount=optional_amount_sats,
            expiry_time=optional_expiry_time_seconds,
            sender_public_key=optional_sender_public_key,
            token_identifier=None,
        )
    )
    response = await sdk.receive_payment(request=request)

    payment_request = response.payment_request
    logging.debug(f"Payment Request: {payment_request}")
    receive_fee_sats = response.fee
    logging.debug(f"Fees: {receive_fee_sats} sats")
    return response
except Exception as error:
    logging.error(error)
    raise
```



## Event Flows

Once a receive payment is initiated, you can follow and react to the different payment events using the guide below for each payment method. See [listening to events](/llms/python/guide/events.md) for how to subscribe to events. 

The `SdkEvent.SYNCED` event is also emitted as the SDK syncs in the background. See [fetching the balance](/llms/python/guide/get_info.md) for the recommended pattern for refreshing the balance and payments list.

#### Lightning

| Event                | Description                                                       | UX Suggestion                                    |
| -------------------- | ----------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer was detected and the claim process will start. | Show payment as pending.                         |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete.        | Show the payment as complete and call `get_info` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/python/guide/get_info.md). |

#### Bitcoin

The following events are emitted in order during the deposit lifecycle. See [Listening to events](/llms/python/guide/events.md) for how to subscribe.

| Event                 | Description                                                                                                                              | UX Suggestion                                                                                               |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| **NewDeposits**       | New deposits were detected. Each deposit includes a `is_mature` field indicating whether it has enough confirmations to be claimed. | Show the deposit to the user. If it does not yet have sufficient confirmations, show it as pending.          |
| **ClaimedDeposits**   | The SDK successfully claimed confirmed deposits.                                                                                         |                                                                                                             |
| **UnclaimedDeposits** | Claiming failed (e.g. fee exceeded the configured maximum or the UTXO could not be found).                                               | Allow the user to manually claim or refund. See [Claiming on-chain deposits](/llms/python/guide/onchain_claims.md). |
| **PaymentPending**    | The Spark transfer was detected and the claim process will start.                                                                        | Show payment as pending.                                                                                    |
| **PaymentSucceeded**  | The Spark transfer is claimed and the payment is complete.                                                                               | Show the payment as complete and call `get_info` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/python/guide/get_info.md).                                                            |

#### Spark

| Event                | Description                                                                                                                                                                                          | UX Suggestion                                    |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer was detected and the claim process will start. For Spark HTLC payments, the claim will only start once the HTLC is claimed. For more details see [Spark HTLC payments](htlcs.md). | Show payment as pending.                         |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete.                                                                                                                                           | Show the payment as complete and call `get_info` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/python/guide/get_info.md). |
