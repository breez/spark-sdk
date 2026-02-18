# pylint: disable=duplicate-code
import hashlib
import logging
from typing import cast
from breez_sdk_spark import (
    BreezSdk,
    ClaimHtlcPaymentRequest,
    ListPaymentsRequest,
    PaymentDetails,
    PaymentDetailsFilter,
    PaymentStatus,
    PaymentType,
    PrepareSendPaymentRequest,
    ReceivePaymentMethod,
    ReceivePaymentRequest,
    SendPaymentRequest,
    SendPaymentOptions,
    SparkHtlcOptions,
    SparkHtlcStatus,
)


async def send_htlc_payment(sdk: BreezSdk):
    # ANCHOR: send-htlc-payment
    payment_request = "<spark address>"
    amount_sats = 50_000
    prepare_request = PrepareSendPaymentRequest(
        payment_request=payment_request,
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
    # ANCHOR_END: send-htlc-payment


async def receive_hodl_invoice_payment(sdk: BreezSdk):
    # ANCHOR: receive-hodl-invoice-payment
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
            )
        )
    )

    invoice = response.payment_request
    logging.debug(f"HODL invoice: {invoice}")
    # ANCHOR_END: receive-hodl-invoice-payment


async def list_claimable_htlc_payments(sdk: BreezSdk):
    # ANCHOR: list-claimable-htlc-payments
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
    # ANCHOR_END: list-claimable-htlc-payments


async def claim_htlc_payment(sdk: BreezSdk):
    # ANCHOR: claim-htlc-payment
    preimage = "<preimage hex>"
    response = await sdk.claim_htlc_payment(
        request=ClaimHtlcPaymentRequest(preimage=preimage)
    )
    payment = response.payment
    # ANCHOR_END: claim-htlc-payment
