import logging
from breez_sdk_spark import (
    BreezSdk,
    GetPaymentRequest,
    ListPaymentsRequest,
    PaymentType,
    PaymentStatus,
    AssetFilter,
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
        # Filter by asset (Bitcoin or Token)
        asset_filter = AssetFilter.TOKEN(token_identifier="token_identifier_here")
        # To filter by Bitcoin instead:
        # asset_filter = AssetFilter.BITCOIN

        request = ListPaymentsRequest(
            # Filter by payment type
            type_filter=[PaymentType.SEND, PaymentType.RECEIVE],
            # Filter by status
            status_filter=[PaymentStatus.COMPLETED],
            asset_filter=asset_filter,
            # Time range filters
            from_timestamp=1704067200,  # Unix timestamp
            to_timestamp=1735689600,    # Unix timestamp
            # Pagination
            offset=0,
            limit=50,
            # Sort order (true = oldest first, false = newest first)
            sort_ascending=False
        )
        response = await sdk.list_payments(request=request)
        payments = response.payments
        # ANCHOR_END: list-payments-filtered
    except Exception as error:
        logging.error(error)
        raise
