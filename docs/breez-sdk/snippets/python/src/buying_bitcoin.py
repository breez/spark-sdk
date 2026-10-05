# pylint: disable=duplicate-code
import logging
from breez_sdk_spark import (
    BreezSdk,
    BuyBitcoinRequest,
    MoonpayDelivery,
)


async def buy_bitcoin(sdk: BreezSdk):
    # ANCHOR: buy-bitcoin
    # Optionally, prefill the purchase amount
    optional_amount_sat = 100_000
    # Optionally, set a redirect URL for after the purchase is completed
    optional_redirect_url = "https://example.com/purchase-complete"

    try:
        request = BuyBitcoinRequest.MOONPAY(
            delivery=MoonpayDelivery.BITCOIN(
                amount_sat=optional_amount_sat,
            ),
            redirect_url=optional_redirect_url,
        )

        response = await sdk.buy_bitcoin(request=request)
        logging.debug("Open this URL in a browser to complete the purchase:")
        logging.debug(response.url)
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: buy-bitcoin


async def buy_bitcoin_via_cross_chain(sdk: BreezSdk):
    # ANCHOR: buy-bitcoin-cross-chain
    # USD amount to receive, in 6-decimal base units ($50)
    amount = 50_000_000

    try:
        request = BuyBitcoinRequest.MOONPAY(
            delivery=MoonpayDelivery.CROSS_CHAIN(
                amount=amount,
                fee_mode=None,
            ),
            redirect_url=None,
        )

        response = await sdk.buy_bitcoin(request=request)
        logging.debug("Open this URL in a browser to complete the purchase:")
        logging.debug(response.url)

        info = response.cross_chain_info
        if info is not None:
            logging.debug(f"USDC to buy: {info.deposit_amount}")
            logging.debug(
                f"Expected to receive: {info.expected_received_amount} "
                f"{info.destination_asset}"
            )
            logging.debug(f"Conversion fee: {info.service_fee_amount}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: buy-bitcoin-cross-chain


async def buy_bitcoin_via_cashapp(sdk: BreezSdk):
    # ANCHOR: buy-bitcoin-cashapp
    # Cash App requires the amount to be specified up front.
    amount_sats = 50_000

    try:
        request = BuyBitcoinRequest.CASH_APP(
            amount_sats=amount_sats,
        )

        response = await sdk.buy_bitcoin(request=request)
        logging.debug("Open this URL in Cash App to complete the purchase:")
        logging.debug(response.url)
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: buy-bitcoin-cashapp
