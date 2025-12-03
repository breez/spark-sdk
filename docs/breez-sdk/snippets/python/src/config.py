import logging
from breez_sdk_spark import (
    default_config,
    Network,
    MaxFee
)


async def configure_sdk():
    # ANCHOR: max-deposit-claim-fee
    # Create the default config
    config = default_config(network=Network.MAINNET)
    config.api_key = "<breez api key>"

    # Disable automatic claiming
    config.max_deposit_claim_fee = None

    # Set a maximum feerate of 10 sat/vB
    config.max_deposit_claim_fee = MaxFee.RATE(sat_per_vbyte=10)

    # Set a maximum fee of 1000 sat
    config.max_deposit_claim_fee = MaxFee.FIXED(amount=1000)

    # Set the maximum fee to the fastest network recommended fee at the time of claim
    # with a leeway of 1 sats/vbyte
    config.max_deposit_claim_fee = MaxFee.NETWORK_RECOMMENDED(leeway_sat_per_vbyte=1)
    # ANCHOR_END: max-deposit-claim-fee
    logging.info(f"Config: {config}")

async def configure_private_enabled_default():
    # ANCHOR: private-enabled-default
    # Disable Spark private mode by default
    config = default_config(network=Network.MAINNET)
    config.private_enabled_default = False
    # ANCHOR_END: private-enabled-default
    logging.info(f"Config: {config}")
