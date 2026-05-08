# pylint: disable=duplicate-code
from breez_sdk_spark import (
    ConnectRequest,
    CreatePasskeyRequest,
    DomainAssociation,
    Network,
    NostrRelayConfig,
    PasskeyClient,
    PrfProvider,
    RegisterRequest,
    RegisteredCredential,
    SignInRequest,
    connect,
    default_config,
)


# ANCHOR: implement-prf-provider
# Implement the PrfProvider trait for custom logic if no built-in
# PasskeyProvider ships for your target. Single API surface:
# derive_seeds for derivation, create_passkey for registration,
# is_supported / check_domain_association for diagnostics.
# Single-salt derivation is the trivial 1-element bulk case.
class CustomPrfProvider(PrfProvider):
    async def derive_seeds(self, salts: list[str]) -> list[bytes]:
        # Call platform passkey API with PRF extension. Use the dual-salt
        # ceremony when the authenticator supports it (one OS prompt for
        # N salts) and fall back to per-salt assertions otherwise.
        # Returns one 32-byte PRF output per salt in input order.
        raise NotImplementedError("Implement using WebAuthn or native passkey APIs")

    async def is_supported(self) -> bool:
        # Check if a PRF-capable authenticator is reachable from this
        # platform / device.
        raise NotImplementedError("Check platform passkey availability")

    async def create_passkey(self, request: CreatePasskeyRequest) -> RegisteredCredential:
        # Register a new credential and return its ID + AAGUID + BE flag.
        raise NotImplementedError("Implement registration via native passkey API")

    async def check_domain_association(self) -> DomainAssociation:
        # Optional: verify the app's identity against the platform's
        # domain verification source. Custom providers without a
        # verification source return SKIPPED, which tells callers
        # "proceed with WebAuthn as normal".
        return DomainAssociation.SKIPPED(
            reason="CustomPrfProvider does not verify domain association"
        )
# ANCHOR_END: implement-prf-provider


async def check_availability():
    # ANCHOR: check-availability
    prf_provider = CustomPrfProvider()

    if await prf_provider.is_supported():
        pass  # Show passkey as primary option
    else:
        pass  # Fall back to mnemonic flow
    # ANCHOR_END: check-availability


async def connect_with_passkey():
    # ANCHOR: connect-with-passkey
    prf_provider = CustomPrfProvider()
    passkey = PasskeyClient(prf_provider, None)

    # sign_in derives the wallet seed for an existing credential. With
    # bulk PRF on iOS+Android this is a single OS prompt that derives
    # master + label seeds in one ceremony.
    response = await passkey.sign_in(SignInRequest(label="personal", extra_salts=[]))

    config = default_config(network=Network.MAINNET)
    sdk = await connect(
        ConnectRequest(config=config, seed=response.wallet.seed, storage_dir="./.data")
    )
    # ANCHOR_END: connect-with-passkey
    return sdk


async def register_new_passkey():
    # ANCHOR: register-passkey
    # For a brand-new user with no existing passkey: register() creates
    # the credential AND derives the wallet seed in one orchestrated
    # call. On iOS+Android this is 2 OS prompts total (1 create + 1
    # dual-salt assert) thanks to the SDK's bulk-PRF setup_wallet path.
    prf_provider = CustomPrfProvider()
    passkey = PasskeyClient(prf_provider, None)

    response = await passkey.register(
        RegisterRequest(
            label="personal",
            extra_salts=[],
            exclude_credential_ids=[],
        )
    )

    config = default_config(network=Network.MAINNET)
    sdk = await connect(
        ConnectRequest(config=config, seed=response.wallet.seed, storage_dir="./.data")
    )
    # ANCHOR_END: register-passkey
    return sdk


async def list_labels() -> list[str]:
    # ANCHOR: list-labels
    prf_provider = CustomPrfProvider()
    relay_config = NostrRelayConfig(breez_api_key="<breez api key>")
    passkey = PasskeyClient(prf_provider, relay_config)

    # sign_in with no label runs in discovery mode: it derives the
    # master seed AND lists labels in the same ceremony, so a follow-up
    # list_labels() reads from the cached identity for free.
    labels = await passkey.list_labels()

    for label in labels:
        print(f"Found label: {label}")
    # ANCHOR_END: list-labels
    return labels


async def store_label():
    # ANCHOR: store-label
    prf_provider = CustomPrfProvider()
    relay_config = NostrRelayConfig(breez_api_key="<breez api key>")
    passkey = PasskeyClient(prf_provider, relay_config)

    # For a new label on an existing identity, call sign_in(new_label)
    # first to seed the SDK's identity cache via setup_wallet, THEN
    # store_label uses the cached identity for free (1 OS prompt total).
    await passkey.store_label(label="personal")
    # ANCHOR_END: store-label
