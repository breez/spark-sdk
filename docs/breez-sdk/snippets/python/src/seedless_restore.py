# pylint: disable=duplicate-code
from breez_sdk_spark import (
    BreezSdk,
    PasskeyPrfProvider,
    SeedlessRestore,
    SdkBuilder,
    default_config,
    Network,
)


# ANCHOR: implement-prf-provider
# Implement PRF provider using platform passkey APIs
class ExamplePasskeyPrfProvider(PasskeyPrfProvider):
    def derive_prf_seed(self, salt: str) -> bytes:
        # Call platform passkey API with PRF extension
        # Returns 32-byte PRF output
        raise NotImplementedError("Implement using platform passkey APIs")

    def is_prf_available(self) -> bool:
        # Check if PRF-capable passkey exists
        raise NotImplementedError("Check platform passkey availability")
# ANCHOR_END: implement-prf-provider


async def create_seed() -> BreezSdk:
    # ANCHOR: create-seed
    prf_provider = ExamplePasskeyPrfProvider()
    seedless = SeedlessRestore(prf_provider, None)

    # Create a new seed with user-chosen salt
    # The salt is published to Nostr for later discovery
    seed = await seedless.create_seed(salt="personal")

    # Use the seed to initialize the SDK
    config = default_config(network=Network.MAINNET)
    builder = SdkBuilder(config=config, seed=seed)
    await builder.with_default_storage(storage_dir="./.data")
    sdk = await builder.build()
    # ANCHOR_END: create-seed
    return sdk


async def list_salts() -> list[str]:
    # ANCHOR: list-salts
    prf_provider = ExamplePasskeyPrfProvider()
    seedless = SeedlessRestore(prf_provider, None)

    # Query Nostr for salts associated with this passkey
    salts = await seedless.list_salts()

    for salt in salts:
        print(f"Found wallet: {salt}")
    # ANCHOR_END: list-salts
    return salts


async def restore_seed() -> BreezSdk:
    # ANCHOR: restore-seed
    prf_provider = ExamplePasskeyPrfProvider()
    seedless = SeedlessRestore(prf_provider, None)

    # Restore seed using a known salt
    seed = await seedless.restore_seed(salt="personal")

    # Use the seed to initialize the SDK
    config = default_config(network=Network.MAINNET)
    builder = SdkBuilder(config=config, seed=seed)
    await builder.with_default_storage(storage_dir="./.data")
    sdk = await builder.build()
    # ANCHOR_END: restore-seed
    return sdk
