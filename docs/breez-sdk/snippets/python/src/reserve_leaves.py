import logging
from breez_sdk_spark import (
    BreezSdk,
    CancelPrepareSendPaymentRequest,
    PrepareSendPaymentRequest,
)


async def prepare_send_payment_reserve_leaves(sdk: BreezSdk):
    # ANCHOR: prepare-send-payment-reserve-leaves
    payment_request = "<payment request>"
    amount_sats = 50_000
    try:
        prepare_response = await sdk.prepare_send_payment(
            request=PrepareSendPaymentRequest(
                payment_request=payment_request,
                amount=amount_sats,
                token_identifier=None,
                conversion_options=None,
                fee_policy=None,
                reserve_leaves=True,
            )
        )

        # The reservation ID can be used to cancel the reservation if needed
        if prepare_response.reservation_id is not None:
            logging.debug(f"Reservation ID: {prepare_response.reservation_id}")

        # Send payment as usual using the prepare response
        # await sdk.send_payment(
        #     request=SendPaymentRequest(prepare_response=prepare_response)
        # )
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: prepare-send-payment-reserve-leaves


async def cancel_prepare_send_payment(sdk: BreezSdk):
    # ANCHOR: cancel-prepare-send-payment
    reservation_id = "<reservation id from prepare response>"
    try:
        await sdk.cancel_prepare_send_payment(
            request=CancelPrepareSendPaymentRequest(
                reservation_id=reservation_id,
            )
        )
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: cancel-prepare-send-payment
