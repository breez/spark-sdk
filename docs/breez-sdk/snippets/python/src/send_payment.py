# pylint: disable=duplicate-code
import logging
from breez_sdk_spark import (
    BreezSdk,
    OnchainConfirmationSpeed,
    PrepareSendPaymentRequest,
    PrepareSendPaymentResponse,
    SendPaymentRequest,
    SendPaymentMethod,
    SendPaymentOptions,
)


async def prepare_send_payment_lightning_bolt11(sdk: BreezSdk):
    # ANCHOR: prepare-send-payment-lightning-bolt11
    payment_request = "<bolt11 invoice>"
    # Optionally set the amount you wish the pay the receiver
    optional_amount_sats = 5_000
    try:
        request = PrepareSendPaymentRequest(
            payment_request=payment_request, amount=optional_amount_sats
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
    # ANCHOR_END: prepare-send-payment-lightning-bolt11


async def prepare_send_payment_onchain(sdk: BreezSdk):
    # ANCHOR: prepare-send-payment-onchain
    payment_request = "<bitcoin address>"
    # Set the amount you wish the pay the receiver
    amount_sats = 50_000
    try:
        request = PrepareSendPaymentRequest(
            payment_request=payment_request, amount=amount_sats
        )
        prepare_response = await sdk.prepare_send_payment(request=request)

        # If the fees are acceptable, continue to create the Send Payment
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
            logging.debug(f"Slow Fees: {slow_fee_sats} sats")
            logging.debug(f"Medium Fees: {medium_fee_sats} sats")
            logging.debug(f"Fast Fees: {fast_fee_sats} sats")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: prepare-send-payment-onchain


async def prepare_send_payment_spark_address(sdk: BreezSdk):
    # ANCHOR: prepare-send-payment-spark-address
    payment_request = "<spark address>"
    # Set the amount you wish the pay the receiver
    amount_sats = 50_000
    try:
        request = PrepareSendPaymentRequest(
            payment_request=payment_request, amount=amount_sats
        )
        prepare_response = await sdk.prepare_send_payment(request=request)

        # If the fees are acceptable, continue to create the Send Payment
        if isinstance(prepare_response.payment_method, SendPaymentMethod.SPARK_ADDRESS):
            fee = prepare_response.payment_method.fee
            logging.debug(f"Fees: {fee} sats")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: prepare-send-payment-spark-address


async def prepare_send_payment_spark_invoice(sdk: BreezSdk):
    # ANCHOR: prepare-send-payment-spark-invoice
    payment_request = "<spark invoice>"
    # Optionally set the amount you wish the pay the receiver
    optional_amount_sats = 50_000
    try:
        request = PrepareSendPaymentRequest(
            payment_request=payment_request, amount=optional_amount_sats
        )
        prepare_response = await sdk.prepare_send_payment(request=request)

        # If the fees are acceptable, continue to create the Send Payment
        if isinstance(prepare_response.payment_method, SendPaymentMethod.SPARK_INVOICE):
            fee = prepare_response.payment_method.fee
            logging.debug(f"Fees: {fee} sats")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: prepare-send-payment-spark-invoice


async def send_payment_lightning_bolt11(
    sdk: BreezSdk, prepare_response: PrepareSendPaymentResponse
):
    # ANCHOR: send-payment-lightning-bolt11
    try:
        options = SendPaymentOptions.BOLT11_INVOICE(
            prefer_spark=False,
            completion_timeout_secs=10
        )
        request = SendPaymentRequest(prepare_response=prepare_response, options=options)
        send_response = await sdk.send_payment(request=request)
        payment = send_response.payment
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: send-payment-lightning-bolt11


async def send_payment_onchain(
    sdk: BreezSdk, prepare_response: PrepareSendPaymentResponse
):
    # ANCHOR: send-payment-onchain
    try:
        options = SendPaymentOptions.BITCOIN_ADDRESS(
            confirmation_speed=OnchainConfirmationSpeed.MEDIUM
        )
        request = SendPaymentRequest(prepare_response=prepare_response, options=options)
        send_response = await sdk.send_payment(request=request)
        payment = send_response.payment
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: send-payment-onchain


async def send_payment_spark(
    sdk: BreezSdk, prepare_response: PrepareSendPaymentResponse
):
    # ANCHOR: send-payment-spark
    try:
        request = SendPaymentRequest(prepare_response=prepare_response)
        send_response = await sdk.send_payment(request=request)
        payment = send_response.payment
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: send-payment-spark
