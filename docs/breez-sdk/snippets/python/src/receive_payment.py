# pylint: disable=duplicate-code
import logging
from breez_sdk_spark import (
    BreezSdk,
    ReceivePaymentMethod,
    ReceivePaymentRequest,
)


async def receive_lightning(sdk: BreezSdk):
    # ANCHOR: receive-payment-lightning-bolt11
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
    # ANCHOR_END: receive-payment-lightning-bolt11


async def receive_onchain(sdk: BreezSdk):
    # ANCHOR: receive-payment-onchain
    try:
        request = ReceivePaymentRequest(
            payment_method=ReceivePaymentMethod.BITCOIN_ADDRESS()
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
    # ANCHOR_END: receive-payment-onchain


async def receive_spark_address(sdk: BreezSdk):
    # ANCHOR: receive-payment-spark-address
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
    # ANCHOR_END: receive-payment-spark-address


async def receive_spark_invoice(sdk: BreezSdk):
    # ANCHOR: receive-payment-spark-invoice
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
    # ANCHOR_END: receive-payment-spark-invoice
