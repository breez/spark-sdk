import logging
from breez_sdk_spark import (
    default_config,
    Network,
    MaxFee,
    OptimizationConfig,
    SparkConfig,
    SparkSigningOperator,
    SparkSspConfig,
    StableBalanceConfig,
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

async def configure_spark_config():
    # ANCHOR: spark-config
    config = default_config(network=Network.MAINNET)

    # Connect to a custom Spark environment
    config.spark_config = SparkConfig(
        coordinator_identifier="0000000000000000000000000000000000000000000000000000000000000001",
        threshold=2,
        signing_operators=[
            SparkSigningOperator(
                id=0,
                identifier="0000000000000000000000000000000000000000000000000000000000000001",
                address="https://0.spark.example.com",
                identity_public_key=(
                    "03acd9a5a88db102730ff83dee69d69088cc4c9d93bbee893e90fd5051b7da9651"
                ),
            ),
            SparkSigningOperator(
                id=1,
                identifier="0000000000000000000000000000000000000000000000000000000000000002",
                address="https://1.spark.example.com",
                identity_public_key=(
                    "02d2d103cacb1d6355efeab27637c74484e2a7459e49110c3fe885210369782e23"
                ),
            ),
            SparkSigningOperator(
                id=2,
                identifier="0000000000000000000000000000000000000000000000000000000000000003",
                address="https://2.spark.example.com",
                identity_public_key=(
                    "0350f07ffc21bfd59d31e0a7a600e2995273938444447cb9bc4c75b8a895dbb853"
                ),
            ),
        ],
        ssp_config=SparkSspConfig(
            base_url="https://api.example.com",
            identity_public_key=(
                "02e0b8d42c5d3b5fe4c5beb6ea796ab3bc8aaf28a3d3195407482c67e0b58228a5"
            ),
        ),
        expected_withdraw_bond_sats=10_000,
        expected_withdraw_relative_block_locktime=1_000,
    )
    # ANCHOR_END: spark-config
    logging.info(f"Config: {config}")
