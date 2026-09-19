# Conditional Payments

Conditional payments use Hash Time-Locked Contracts (HTLCs) to lock funds with a cryptographic hash of a secret preimage and an expiration time. The payment can only be claimed by revealing the preimage before expiration. If not claimed in time, the funds are automatically returned to the sender. This enables use cases like atomic cross-chain swaps.

The SDK supports both sending conditional payments via Spark HTLCs and receiving them via HODL invoices.

**Developer note**

Preimages are required to be unique and are not managed by the SDK. It is your responsibility as a developer to manage them, including how to generate them, store them, and provide them when claiming payments.

## Sending Spark HTLC payments

HTLC payments use the standard payment API described in [Sending payments](send_payment.md). To create an HTLC payment, prepare the payment normally, then provide the Spark HTLC options when [sending](send_payment.md#spark). These options include the payment hash (SHA-256 hash of the preimage) and the expiry duration.

```python
payment_request = "<spark address>"
amount_sats = 50_000
prepare_request = PrepareSendPaymentRequest(
    payment_request=PaymentRequest.INPUT(input=payment_request),
    amount=amount_sats,
    token_identifier=None,
    conversion_options=None,
    fee_policy=None,
)
prepare_response = await sdk.prepare_send_payment(request=prepare_request)

# If the fees are acceptable, continue to create the HTLC Payment
if hasattr(prepare_response.payment_method, "fee"):
    fee = prepare_response.payment_method.fee
    logging.debug(f"Fees: {fee} sats")

preimage = "<32-byte unique preimage hex>"
preimage_bytes = bytes.fromhex(preimage)
payment_hash_bytes = hashlib.sha256(preimage_bytes).digest()
payment_hash = payment_hash_bytes.hex()

# Set the HTLC options
options = SendPaymentOptions.SPARK_ADDRESS(
    htlc_options=SparkHtlcOptions(
        payment_hash=payment_hash, expiry_duration_secs=1000
    )
)

request = SendPaymentRequest(
    prepare_response=prepare_response, options=options
)
send_response = await sdk.send_payment(request=request)
payment = send_response.payment
```



## Receiving using HODL invoices

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

You can receive using HODL invoices — Lightning invoices where the payment is held until you claim it by revealing the preimage. To create one, provide a `payment_hash` when calling `receive_payment` with the `ReceivePaymentMethod.BOLT11_INVOICE` payment method.

```python
preimage = "<32-byte unique preimage hex>"
preimage_bytes = bytes.fromhex(preimage)
payment_hash_bytes = hashlib.sha256(preimage_bytes).digest()
payment_hash = payment_hash_bytes.hex()

response = await sdk.receive_payment(
    request=ReceivePaymentRequest(
        payment_method=ReceivePaymentMethod.BOLT11_INVOICE(
            description="HODL invoice",
            amount_sats=50_000,
            expiry_secs=None,
            payment_hash=payment_hash,
            receiver_identity_public_key=None,
        )
    )
)

invoice = response.payment_request
logging.debug(f"HODL invoice: {invoice}")
```



## Listing claimable conditional payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

Once detected, claimable HTLC payments are immediately listed as pending in the [list of payments](/llms/python/guide/list_payments.md). Additionally, a `SdkEvent.PAYMENT_PENDING` event is emitted to notify your application. See [Listening to events](/llms/python/guide/events.md) for more details.

To list only claimable HTLC payments, you can filter by HTLC status. This works for both Spark HTLC payments and HODL invoices.

```python
request = ListPaymentsRequest(
    type_filter=[PaymentType.RECEIVE],
    status_filter=[PaymentStatus.PENDING],
    payment_details_filter=[
        cast(PaymentDetailsFilter, PaymentDetailsFilter.SPARK(
            htlc_status=[SparkHtlcStatus.WAITING_FOR_PREIMAGE],
            conversion_refund_needed=None
        )),
        cast(PaymentDetailsFilter, PaymentDetailsFilter.LIGHTNING(
            htlc_status=[SparkHtlcStatus.WAITING_FOR_PREIMAGE],
        )),
    ],
)

response = await sdk.list_payments(request=request)
payments = response.payments

for payment in payments:
    if isinstance(payment.details, PaymentDetails.SPARK):
        if payment.details.htlc_details is not None:
            logging.debug(f"Spark HTLC expiry time: {payment.details.htlc_details.expiry_time}")
    elif isinstance(payment.details, PaymentDetails.LIGHTNING):
        expiry = payment.details.htlc_details.expiry_time
        logging.debug(f"Lightning HTLC expiry time: {expiry}")
```



## Claiming conditional payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.claim_htlc_payment

To claim an HTLC payment, provide the preimage that matches the payment hash. This works for both Spark HTLC payments and HODL invoices.

```python
preimage = "<preimage hex>"
response = await sdk.claim_htlc_payment(
    request=ClaimHtlcPaymentRequest(preimage=preimage)
)
payment = response.payment
```
