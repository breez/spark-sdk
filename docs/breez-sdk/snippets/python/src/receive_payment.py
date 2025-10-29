# pylint: disable=duplicate-code
import logging
from breez_sdk_spark import (
    BreezSdk,
    ReceivePaymentMethod,
    ReceivePaymentRequest,
    WaitForPaymentRequest,
    WaitForPaymentIdentifier,
)


async def receive_lightning(sdk: BreezSdk):
    # ANCHOR: receive-payment-lightning-bolt11
    try:
        description = "<invoice description>"
        # Optionally set the invoice amount you wish the payer to send
        optional_amount_sats = 5_000
        payment_method = ReceivePaymentMethod.BOLT11_INVOICE(
            description=description, amount_sats=optional_amount_sats
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
            payment_method=ReceivePaymentMethod.BITCOIN_ADDRESS
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
            payment_method=ReceivePaymentMethod.SPARK_ADDRESS
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


async def wait_for_payment(sdk: BreezSdk):
    # ANCHOR: wait-for-payment
    try:
        # Waiting for a payment given its payment request (Bolt11 or Spark invoice)
        payment_request = "<Bolt11 or Spark invoice>"

        # Wait for a payment to be completed using a payment request
        payment_request_response = await sdk.wait_for_payment(
            request=WaitForPaymentRequest(
                identifier=WaitForPaymentIdentifier.PAYMENT_REQUEST(payment_request)
            )
        )

        logging.debug(f"Payment received with ID: {payment_request_response.payment.id}")

        # Waiting for a payment given its payment id
        payment_id = "<payment id>"

        # Wait for a payment to be completed using a payment id
        payment_id_response = await sdk.wait_for_payment(
            request=WaitForPaymentRequest(
                identifier=WaitForPaymentIdentifier.PAYMENT_ID(payment_id)
            )
        )

        logging.debug(f"Payment received with ID: {payment_id_response.payment.id}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: wait-for-payment
