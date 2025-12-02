# pylint: disable=duplicate-code
import hashlib
import logging
from breez_sdk_spark import (
    BreezSdk,
    ClaimHtlcPaymentRequest,
    ListPaymentsRequest,
    PaymentDetailsFilter,
    PaymentStatus,
    PaymentType,
    PrepareSendPaymentRequest,
    SendPaymentRequest,
    SendPaymentOptions,
    SparkHtlcOptions,
    SparkHtlcStatus,
)


async def send_htlc_payment(sdk: BreezSdk):
    # ANCHOR: send-htlc-payment
    payment_request = "<spark address>"
    # Set the amount you wish the pay the receiver
    amount_sats = 50_000
    prepare_request = PrepareSendPaymentRequest(
        payment_request=payment_request, amount=amount_sats
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


async def list_claimable_htlc_payments(sdk: BreezSdk):
    # ANCHOR: list-claimable-htlc-payments
    request = ListPaymentsRequest(
        type_filter=[PaymentType.RECEIVE],
        status_filter=[PaymentStatus.PENDING],
        payment_details_filter=PaymentDetailsFilter.SPARK(
            htlc_status=[SparkHtlcStatus.WAITING_FOR_PREIMAGE],
            transfer_refund_needed=None
        ),
    )

    response = await sdk.list_payments(request=request)
    payments = response.payments
    # ANCHOR_END: list-claimable-htlc-payments


async def claim_htlc_payment(sdk: BreezSdk):
    # ANCHOR: claim-htlc-payment
    preimage = "<preimage hex>"
    response = await sdk.claim_htlc_payment(
        request=ClaimHtlcPaymentRequest(preimage=preimage)
    )
    payment = response.payment
    # ANCHOR_END: claim-htlc-payment
