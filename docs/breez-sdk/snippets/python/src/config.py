import logging
from breez_sdk_spark import (
    default_config,
    Network,
    Fee
)


async def configure_sdk():
    # ANCHOR: max-deposit-claim-fee
    # Create the default config
    config = default_config(network=Network.MAINNET)
    config.api_key = "<breez api key>"

    # Disable automatic claiming
    config.max_deposit_claim_fee = None

    # Set a maximum feerate of 10 sat/vB
    config.max_deposit_claim_fee = Fee.RATE(sat_per_vbyte=10)

    # Set a maximum fee of 1000 sat
    config.max_deposit_claim_fee = Fee.FIXED(amount=1000)
    # ANCHOR_END: max-deposit-claim-fee
    logging.info(f"Config: {config}")
