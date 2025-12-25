from breez_sdk_spark import *

# ANCHOR: default-external-signer
def create_signer() -> ExternalSigner:
    mnemonic = "<mnemonic words>"
    network = Network.MAINNET
    key_set_type = KeySetType.DEFAULT
    use_address_index = False
    account_number = 0
    
    signer = default_external_signer(
        mnemonic=mnemonic,
        passphrase=None,
        network=network,
        key_set_type=key_set_type,
        use_address_index=use_address_index,
        account_number=account_number
    )
    
    return signer
# ANCHOR_END: default-external-signer

# ANCHOR: connect-with-signer
def connect_with_signer() -> BreezSdk:
    # Create the signer
    signer = default_external_signer(
        mnemonic="<mnemonic words>",
        passphrase=None,
        network=Network.MAINNET,
        key_set_type=KeySetType.DEFAULT,
        use_address_index=False,
        account_number=0
    )
    
    # Create the config
    config = default_config(Network.MAINNET)
    config.api_key = "<breez api key>"
    
    # Connect using the external signer
    sdk = connect_with_signer(ConnectWithSignerRequest(
        config=config,
        signer=signer,
        storage_dir="./.data"
    ))
    
    return sdk
# ANCHOR_END: connect-with-signer
