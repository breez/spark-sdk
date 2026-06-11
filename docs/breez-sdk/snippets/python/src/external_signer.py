from breez_sdk_spark import (
    default_config,
    default_external_signers,
    connect_with_signer,
    BreezSdk,
    ConnectWithSignerRequest,
    DefaultExternalSigners,
    KeySetConfig,
    Network,
)

# ANCHOR: default-external-signer
def create_signers() -> DefaultExternalSigners:
    mnemonic = "<mnemonic words>"
    network = Network.MAINNET
    account_number = 0

    key_set_config = KeySetConfig(
        account_number=account_number,
    )

    signers = default_external_signers(
        mnemonic=mnemonic,
        passphrase=None,
        network=network,
        key_set_config=key_set_config,
    )

    return signers
# ANCHOR_END: default-external-signer

# ANCHOR: connect-with-signer
async def example_connect_with_signer(signers: DefaultExternalSigners) -> BreezSdk:
    # Create the config
    config = default_config(Network.MAINNET)
    config.api_key = "<breez api key>"

    # Connect using the external signers
    sdk = await connect_with_signer(ConnectWithSignerRequest(
        config=config,
        breez_signer=signers.breez_signer,
        spark_signer=signers.spark_signer,
        storage_dir="./.data"
    ))

    return sdk
# ANCHOR_END: connect-with-signer
