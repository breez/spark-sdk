# pylint: disable=duplicate-code
import logging
from breez_sdk_spark import (
    BreezSdk,
    BuyBitcoinRequest,
)


async def buy_bitcoin_basic(sdk: BreezSdk):
    # ANCHOR: buy-bitcoin-basic
    try:
        # Buy Bitcoin using the SDK's auto-generated deposit address
        request = BuyBitcoinRequest()

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
            locked_amount_sat=100_000,
        )

        response = await sdk.buy_bitcoin(request=request)
        logging.debug("Open this URL in a browser to complete the purchase:")
        logging.debug(response.url)
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: buy-bitcoin-with-amount


async def buy_bitcoin_with_redirect(sdk: BreezSdk):
    # ANCHOR: buy-bitcoin-with-redirect
    try:
        # Provide a custom redirect URL for after the purchase
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
    # ANCHOR_END: buy-bitcoin-with-redirect


async def buy_bitcoin_with_address(sdk: BreezSdk):
    # ANCHOR: buy-bitcoin-with-address
    try:
        # Specify a custom Bitcoin address to receive funds
        request = BuyBitcoinRequest(
            address="bc1qexample...",
            locked_amount_sat=100_000,
        )

        response = await sdk.buy_bitcoin(request=request)
        logging.debug("Open this URL in a browser to complete the purchase:")
        logging.debug(response.url)
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: buy-bitcoin-with-address
