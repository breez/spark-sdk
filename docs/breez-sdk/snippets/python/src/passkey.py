# pylint: disable=duplicate-code
from breez_sdk_spark import (
    ConnectRequest,
    NostrRelayConfig,
    PasskeyPrfProvider,
    Passkey,
    connect,
    default_config,
    Network,
)


# ANCHOR: implement-prf-provider
# In practice, implement using platform-specific passkey APIs.
class ExamplePasskeyPrfProvider(PasskeyPrfProvider):
    async def derive_prf_seed(self, salt: str):
        # Call platform passkey API with PRF extension
        # Returns 32-byte PRF output
        raise NotImplementedError("Implement using WebAuthn or native passkey APIs")

    async def is_prf_available(self):
        # Check if PRF-capable passkey exists
        raise NotImplementedError("Check platform passkey availability")
# ANCHOR_END: implement-prf-provider


async def connect_with_passkey():
    # ANCHOR: connect-with-passkey
    prf_provider = ExamplePasskeyPrfProvider()
    passkey = Passkey(prf_provider, None)

    # Derive the wallet from the passkey (pass None for the default wallet)
    wallet = await passkey.get_wallet("personal")

    config = default_config(network=Network.MAINNET)
    sdk = await connect(ConnectRequest(config=config, seed=wallet.seed, storage_dir="./.data"))
    # ANCHOR_END: connect-with-passkey
    return sdk


async def list_labels() -> list[str]:
    # ANCHOR: list-labels
    prf_provider = ExamplePasskeyPrfProvider()
    relay_config = NostrRelayConfig(breez_api_key="<breez api key>")
    passkey = Passkey(prf_provider, relay_config)

    # Query Nostr for labels associated with this passkey
    labels = await passkey.list_labels()

    for label in labels:
        print(f"Found label: {label}")
    # ANCHOR_END: list-labels
    return labels


async def store_label():
    # ANCHOR: store-label
    prf_provider = ExamplePasskeyPrfProvider()
    relay_config = NostrRelayConfig(breez_api_key="<breez api key>")
    passkey = Passkey(prf_provider, relay_config)

    # Publish the label to Nostr for later discovery
    await passkey.store_label(label="personal")
    # ANCHOR_END: store-label
