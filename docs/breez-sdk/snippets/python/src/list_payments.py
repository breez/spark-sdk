import logging
from breez_sdk_spark import (
    BreezSdk,
    GetPaymentRequest,
    ListPaymentsRequest,
)


async def get_payment(sdk: BreezSdk):
    try:
        # ANCHOR: get-payment
        payment_id = "<payment id>"
        response = await sdk.get_payment(
            request=GetPaymentRequest(payment_id=payment_id)
        )
        payment = response.payment
        # ANCHOR_END: get-payment
    except Exception as error:
        logging.error(error)
        raise


async def list_payments(sdk: BreezSdk):
    try:
        # ANCHOR: list-payments
        response = await sdk.list_payments(request=ListPaymentsRequest())
        payments = response.payments
        # ANCHOR_END: list-payments
    except Exception as error:
        logging.error(error)
        raise


async def list_payments_filtered(sdk: BreezSdk):
    try:
        # ANCHOR: list-payments-filtered
        request = ListPaymentsRequest(offset=0, limit=50)
        response = await sdk.list_payments(request=request)
        payments = response.payments
        # ANCHOR_END: list-payments-filtered
    except Exception as error:
        logging.error(error)
        raise
