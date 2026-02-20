from breez_sdk_spark import (
    BreezSdkSpark,
    BreezSdk,
    ConnectWithSignerRequest,
    ExternalSigner,
    KeySetConfig,
    KeySetType,
    Network,
)

# ANCHOR: default-external-signer
def create_signer() -> ExternalSigner:
    mnemonic = "<mnemonic words>"
    network = Network.MAINNET
    key_set_type = KeySetType.DEFAULT
    use_address_index = False
    account_number = 0

    key_set_config = KeySetConfig(
        key_set_type=key_set_type,
        use_address_index=use_address_index,
        account_number=account_number,
    )

    signer = BreezSdkSpark.default_external_signer(
        mnemonic=mnemonic,
        passphrase=None,
        network=network,
        key_set_config=key_set_config,
    )

    return signer
# ANCHOR_END: default-external-signer

# ANCHOR: connect-with-signer
async def example_connect_with_signer(signer: ExternalSigner) -> BreezSdk:
    # Create the config
    config = BreezSdkSpark.default_config(Network.MAINNET)
    config.api_key = "<breez api key>"

    # Connect using the external signer
    sdk = await BreezSdkSpark.connect_with_signer(ConnectWithSignerRequest(
        config=config,
        signer=signer,
        storage_dir="./.data"
    ))

    return sdk
# ANCHOR_END: connect-with-signer
