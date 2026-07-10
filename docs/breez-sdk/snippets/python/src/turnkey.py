from breez_sdk_spark import (
    default_config,
    create_turnkey_signer,
    connect_with_signer,
    BreezSdk,
    ConnectWithSignerRequest,
    Network,
    TurnkeyConfig,
)

async def connect_with_turnkey() -> BreezSdk:
    # ANCHOR: turnkey-connect
    turnkey_config = TurnkeyConfig(
        base_url=None,
        organization_id="<turnkey sub-organization id>",
        api_public_key="<api public key hex>",
        api_private_key="<api private key hex>",
        wallet_id="<turnkey wallet id>",
        network=Network.MAINNET,
        account_number=None,
        # Set after the first connect to make later signer setup network-free
        identity_public_key=None,
        retry=None,
        max_rps=None,
    )

    signers = await create_turnkey_signer(config=turnkey_config)

    config = default_config(Network.MAINNET)
    config.api_key = "<breez api key>"

    sdk = await connect_with_signer(ConnectWithSignerRequest(
        config=config,
        breez_signer=signers.breez_signer,
        spark_signer=signers.spark_signer,
        storage_dir="./.data"
    ))
    # ANCHOR_END: turnkey-connect
    return sdk
