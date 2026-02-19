import logging
from breez_sdk_spark import (
    BreezSdk,
    default_config,
    Network,
    MaxFee,
    MaxDepositClaimFeeUpdate,
    OptimizationConfig,
    StableBalanceConfig,
    StableBalanceConfigUpdate,
    UpdateConfigRequest,
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

async def configure_optimization_configuration():
    # ANCHOR: optimization-configuration
    config = default_config(network=Network.MAINNET)
    config.optimization_config = OptimizationConfig(auto_enabled=True, multiplicity=1)
    # ANCHOR_END: optimization-configuration
    logging.info(f"Config: {config}")

async def configure_stable_balance():
    # ANCHOR: stable-balance-config
    config = default_config(network=Network.MAINNET)

    # Enable stable balance with auto-conversion to a specific token
    config.stable_balance_config = StableBalanceConfig(
        token_identifier="<token_identifier>",
        threshold_sats=10_000,
        max_slippage_bps=100,
        reserved_sats=1_000
    )
    # ANCHOR_END: stable-balance-config
    logging.info(f"Config: {config}")

async def update_config(sdk: BreezSdk):
    # ANCHOR: update-config
    # Update the sync interval and prefer Spark over Lightning
    await sdk.update_config(request=UpdateConfigRequest(
        sync_interval_secs=30,
        prefer_spark_over_lightning=True,
    ))

    # Enable stable balance with auto-conversion
    await sdk.update_config(request=UpdateConfigRequest(
        stable_balance_config=StableBalanceConfigUpdate.SET(
            config=StableBalanceConfig(
                token_identifier="<token_identifier>",
                threshold_sats=10_000,
                max_slippage_bps=100,
                reserved_sats=1_000,
            )
        ),
    ))

    # Disable stable balance and update max deposit claim fee
    await sdk.update_config(request=UpdateConfigRequest(
        max_deposit_claim_fee=MaxDepositClaimFeeUpdate.SET(
            fee=MaxFee.RATE(sat_per_vbyte=5),
        ),
        stable_balance_config=StableBalanceConfigUpdate.UNSET,
    ))
    # ANCHOR_END: update-config
