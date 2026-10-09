import logging
from breez_sdk_spark import SdkError


async def handle_errors(sdk, request):
    # ANCHOR: handle-errors
    try:
        prepare_response = await sdk.prepare_send_payment(request=request)
    except SdkError.InsufficientFunds:
        logging.debug("Not enough funds for this payment")
    except SdkError.CrossChainDisabled as error:
        logging.debug(f"Cross-chain payments are not enabled, see {error.docs_url}")
    except Exception as error:
        logging.error(f"Failed to prepare the payment: {error}")
        raise
    else:
        logging.debug(f"Payment prepared: {prepare_response.payment_method}")
    # ANCHOR_END: handle-errors
