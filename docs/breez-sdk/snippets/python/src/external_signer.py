from breez_sdk_spark import (
    default_config,
    default_external_signers,
    connect_with_signer,
    BreezSdk,
    Config,
    ConnectWithSignerRequest,
    ExternalSigners,
    Network,
    SdkBuilder,
    SigningOnlyExternalSigners,
)

# ANCHOR: default-external-signer
def create_signers() -> ExternalSigners:
    mnemonic = "<mnemonic words>"
    network = Network.MAINNET
    account_number = 0

    signers = default_external_signers(
        mnemonic=mnemonic,
        passphrase=None,
        network=network,
        account_number=account_number,
    )

    return signers
# ANCHOR_END: default-external-signer

# ANCHOR: connect-with-signer
async def example_connect_with_signer(signers: ExternalSigners) -> BreezSdk:
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

# ANCHOR: sdk-builder-with-signer
async def example_build_with_signer(signers: ExternalSigners) -> BreezSdk:
    config = default_config(Network.MAINNET)
    config.api_key = "<breez api key>"
    builder = SdkBuilder.new_with_signer(
        config=config,
        breez_signer=signers.breez_signer,
        spark_signer=signers.spark_signer,
    )
    # await builder.with_storage_backend(<your storage backend>)
    # await builder.with_shared_context(<your shared context>)
    sdk = await builder.build()
    return sdk
# ANCHOR_END: sdk-builder-with-signer

# ANCHOR: sdk-builder-with-signing-only-signer
async def example_build_with_signing_only_signer(
    config: Config, signers: SigningOnlyExternalSigners
) -> BreezSdk:
    builder = SdkBuilder.new_with_signing_only_signer(
        config=config,
        breez_signer=signers.breez_signer,
        spark_signer=signers.spark_signer,
    )
    sdk = await builder.build()
    return sdk
# ANCHOR_END: sdk-builder-with-signing-only-signer
