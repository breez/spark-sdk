# pylint: disable=duplicate-code
import logging
from breez_sdk_spark import (
    BreezSdk,
    BuyBitcoinRequest,
)


async def buy_bitcoin_basic(sdk: BreezSdk):
    # ANCHOR: buy-bitcoin-basic
    try:
        request = BuyBitcoinRequest(
            address="bc1qexample...",  # Your Bitcoin address
            locked_amount_sat=None,
            max_amount_sat=None,
            redirect_url=None,
        )

        response = await sdk.buy_bitcoin(request=request)
        logging.debug("Open this URL in a browser to complete the purchase:")
        logging.debug(response.url)
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: buy-bitcoin-basic


async def buy_bitcoin_with_amount(sdk: BreezSdk):
    # ANCHOR: buy-bitcoin-with-amount
    try:
        # Lock the purchase to a specific amount (e.g., 0.001 BTC = 100,000 sats)
        request = BuyBitcoinRequest(
            address="bc1qexample...",
            locked_amount_sat=100_000,  # Pre-fill with 100,000 sats
            max_amount_sat=None,
            redirect_url=None,
        )

        response = await sdk.buy_bitcoin(request=request)
        logging.debug("Open this URL in a browser to complete the purchase:")
        logging.debug(response.url)
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: buy-bitcoin-with-amount


async def buy_bitcoin_with_limits(sdk: BreezSdk):
    # ANCHOR: buy-bitcoin-with-limits
    try:
        # Set both a locked amount and maximum amount
        request = BuyBitcoinRequest(
            address="bc1qexample...",
            locked_amount_sat=50_000,   # Pre-fill with 50,000 sats
            max_amount_sat=500_000,     # Limit to 500,000 sats max
            redirect_url=None,
        )

        response = await sdk.buy_bitcoin(request=request)
        logging.debug("Open this URL in a browser to complete the purchase:")
        logging.debug(response.url)
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: buy-bitcoin-with-limits


async def buy_bitcoin_with_redirect(sdk: BreezSdk):
    # ANCHOR: buy-bitcoin-with-redirect
    try:
        # Provide a custom redirect URL for after the purchase
        request = BuyBitcoinRequest(
            address="bc1qexample...",
            locked_amount_sat=100_000,
            max_amount_sat=None,
            redirect_url="https://example.com/purchase-complete",
        )

        response = await sdk.buy_bitcoin(request=request)
        logging.debug("Open this URL in a browser to complete the purchase:")
        logging.debug(response.url)
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: buy-bitcoin-with-redirect
