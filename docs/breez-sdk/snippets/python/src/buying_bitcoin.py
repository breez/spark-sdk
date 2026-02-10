# pylint: disable=duplicate-code
import logging
from breez_sdk_spark import (
    BreezSdk,
    BuyBitcoinRequest,
)


async def buy_bitcoin(sdk: BreezSdk):
    # ANCHOR: buy-bitcoin
    try:
        # Buy Bitcoin with funds deposited directly into the user's wallet.
        # Optionally lock the purchase to a specific amount and provide a redirect URL.
        request = BuyBitcoinRequest(
            locked_amount_sat=100_000,
            redirect_url="https://example.com/purchase-complete",
        )

        response = await sdk.buy_bitcoin(request=request)
        logging.debug("Open this URL in a browser to complete the purchase:")
        logging.debug(response.url)
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: buy-bitcoin
