# pylint: disable=duplicate-code
import logging
from breez_sdk_spark import (
    BreezSdk,
    OnchainConfirmationSpeed,
    PayAmount,
    PrepareSendPaymentRequest,
    PrepareSendPaymentResponse,
    SendPaymentRequest,
    SendPaymentMethod,
    SendPaymentOptions,
    ConversionOptions,
    ConversionType,
)


async def prepare_send_payment_lightning_bolt11(sdk: BreezSdk):
    # ANCHOR: prepare-send-payment-lightning-bolt11
    payment_request = "<bolt11 invoice>"
    # Optionally set the amount you wish to pay the receiver
    optional_pay_amount = PayAmount.BITCOIN(amount_sats=5_000)
    try:
        request = PrepareSendPaymentRequest(
            payment_request=payment_request,
            pay_amount=optional_pay_amount,
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
    # Set the amount you wish to pay the receiver
    pay_amount = PayAmount.BITCOIN(amount_sats=50_000)
    try:
        request = PrepareSendPaymentRequest(
            payment_request=payment_request,
            pay_amount=pay_amount,
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
    # ANCHOR_END: prepare-send-payment-onchain


async def prepare_send_payment_spark_address(sdk: BreezSdk):
    # ANCHOR: prepare-send-payment-spark-address
    payment_request = "<spark address>"
    # Set the amount you wish to pay the receiver
    pay_amount = PayAmount.BITCOIN(amount_sats=50_000)
    try:
        request = PrepareSendPaymentRequest(
            payment_request=payment_request,
            pay_amount=pay_amount,
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
    # Optionally set the amount you wish to pay the receiver
    optional_pay_amount = PayAmount.BITCOIN(amount_sats=50_000)
    try:
        request = PrepareSendPaymentRequest(
            payment_request=payment_request,
            pay_amount=optional_pay_amount,
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


async def prepare_send_payment_token_conversion(sdk: BreezSdk):
    # ANCHOR: prepare-send-payment-with-conversion
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
            payment_request=payment_request,
            conversion_options=conversion_options,
        )
        prepare_response = await sdk.prepare_send_payment(request=request)

        # If the fees are acceptable, continue to create the Send Payment
        if prepare_response.conversion_estimate is not None:
            conversion_estimate = prepare_response.conversion_estimate
            logging.debug(
                f"Estimated conversion amount: {conversion_estimate.amount} token base units"
            )
            logging.debug(
                f"Estimated conversion fee: {conversion_estimate.fee} token base units"
            )
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: prepare-send-payment-with-conversion


async def send_payment_lightning_bolt11(
    sdk: BreezSdk, prepare_response: PrepareSendPaymentResponse
):
    # ANCHOR: send-payment-lightning-bolt11
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
    # ANCHOR_END: send-payment-lightning-bolt11


async def send_payment_onchain(
    sdk: BreezSdk, prepare_response: PrepareSendPaymentResponse
):
    # ANCHOR: send-payment-onchain
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
    # ANCHOR_END: send-payment-onchain


async def send_payment_spark(
    sdk: BreezSdk, prepare_response: PrepareSendPaymentResponse
):
    # ANCHOR: send-payment-spark
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
    # ANCHOR_END: send-payment-spark


async def prepare_send_payment_drain(sdk: BreezSdk):
    # ANCHOR: prepare-send-payment-drain
    # Use PayAmount.DRAIN to send all available funds
    payment_request = "<payment request>"
    pay_amount = PayAmount.DRAIN()
    try:
        request = PrepareSendPaymentRequest(
            payment_request=payment_request,
            pay_amount=pay_amount,
        )
        prepare_response = await sdk.prepare_send_payment(request=request)

        # The response contains PayAmount.DRAIN to indicate this is a drain operation
        logging.debug(f"Pay amount: {prepare_response.pay_amount}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: prepare-send-payment-drain
