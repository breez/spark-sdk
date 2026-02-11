# pylint: disable=duplicate-code
import logging
from breez_sdk_spark import (
    BreezSdk,
    BuyBitcoinRequest,
)


async def buy_bitcoin(sdk: BreezSdk):
    # ANCHOR: buy-bitcoin
    # Optionally, lock the purchase to a specific amount
    optional_locked_amount_sat = 100_000
    # Optionally, set a redirect URL for after the purchase is completed
    optional_redirect_url = "https://example.com/purchase-complete"

    try:
        request = BuyBitcoinRequest(
            locked_amount_sat=optional_locked_amount_sat,
            redirect_url=optional_redirect_url,
        )

        response = await sdk.buy_bitcoin(request=request)
        logging.debug("Open this URL in a browser to complete the purchase:")
        logging.debug(response.url)
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: buy-bitcoin
