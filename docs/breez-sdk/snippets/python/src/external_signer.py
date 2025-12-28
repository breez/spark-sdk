from breez_sdk_spark import (
    default_config,
    default_external_signer,
    connect_with_signer,
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

    signer = default_external_signer(
        mnemonic=mnemonic,
        passphrase=None,
        network=network,
        key_set_config=key_set_config,
    )

    return signer
# ANCHOR_END: default-external-signer

# ANCHOR: connect-with-signer
async def example_connect_with_signer() -> BreezSdk:
    # Create the signer
    key_set_config = KeySetConfig(
        key_set_type=KeySetType.DEFAULT,
        use_address_index=False,
        account_number=None,
    )

    signer = default_external_signer(
        mnemonic="<mnemonic words>",
        passphrase=None,
        network=Network.MAINNET,
        key_set_config=key_set_config,
    )

    # Create the config
    config = default_config(Network.MAINNET)
    config.api_key = "<breez api key>"

    # Connect using the external signer
    sdk = await connect_with_signer(ConnectWithSignerRequest(
        config=config,
        signer=signer,
        storage_dir="./.data"
    ))

    return sdk
# ANCHOR_END: connect-with-signer
