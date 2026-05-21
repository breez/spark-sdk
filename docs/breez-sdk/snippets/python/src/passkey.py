# pylint: disable=duplicate-code
from breez_sdk_spark import (
    ConnectRequest,
    ConnectWithPasskeyRequest,
    DomainAssociation,
    Network,
    PasskeyAvailability,
    PasskeyClient,
    PasskeyConfig,
    PrfProvider,
    PrfProviderError,
    RegisterRequest,
    RegisteredCredential,
    SignInRequest,
    connect,
    default_config,
)


# ANCHOR: implement-prf-provider
# Implement the PrfProvider trait for custom logic if no built-in
# PasskeyProvider ships for your target. Three required methods:
# derive_seeds for derivation, is_supported for the capability probe;
# create_passkey for registration is optional.
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

    async def create_passkey(self, exclude_credential_ids: list[bytes]) -> RegisteredCredential:
        # Register a new credential and return its ID, the WebAuthn
        # user.id the platform recorded (returned for host-side
        # correlation, never host-supplied), AAGUID, and BE flag.
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
    passkey = PasskeyClient(prf_provider, None, None)

    # check_availability collapses is_supported + check_domain_association
    # into a single tagged value. Branch on the variant the host needs.
    availability = await passkey.check_availability()
    if isinstance(availability, PasskeyAvailability.AVAILABLE):
        pass  # Show passkey as primary option.
    elif isinstance(availability, PasskeyAvailability.PRF_UNSUPPORTED):
        pass  # Fall back to mnemonic flow.
    elif isinstance(availability, PasskeyAvailability.NOT_ASSOCIATED):
        print(f"Domain association failed (source={availability.source}): {availability.reason}")
    elif isinstance(availability, PasskeyAvailability.SKIPPED):
        pass  # No verification source on this platform; proceed normally.
    # ANCHOR_END: check-availability


def setup_passkey_client() -> PasskeyClient:
    # ANCHOR: setup-client
    prf_provider = CustomPrfProvider()
    passkey = PasskeyClient(prf_provider, "<breez api key>", None)
    # ANCHOR_END: setup-client
    return passkey


async def connect_with_passkey():
    # ANCHOR: connect-with-passkey
    # Single-CTA onboarding: silent sign-in for a returning user,
    # fall-through to register on a fresh device. Internally pins
    # `prefer_immediately_available_credentials = True` so the silent
    # attempt fast-fails (no UI) when no local credential exists; only
    # `CredentialNotFound` flips to register, all other errors (cancel
    # / timeout / configuration) propagate unchanged.
    prf_provider = CustomPrfProvider()
    passkey = PasskeyClient(prf_provider, None, None)

    response = await passkey.connect_with_passkey(
        ConnectWithPasskeyRequest(label="personal", exclude_credential_ids=[])
    )

    # `registered_credential` doubles as the path discriminator:
    # not None when a new credential was just registered (persist
    # credential_id for future exclude_credential_ids); None when
    # silent sign-in succeeded for an existing credential.
    if response.registered_credential is not None:
        _persist = response.registered_credential.credential_id

    config = default_config(network=Network.MAINNET)
    sdk = await connect(
        ConnectRequest(config=config, seed=response.wallet.seed, storage_dir="./.data")
    )
    # ANCHOR_END: connect-with-passkey
    return sdk


async def register_new_passkey():
    # ANCHOR: register-passkey
    # For a brand-new user with no existing passkey: register() creates
    # the credential AND derives the seed in one orchestrated
    # call. On iOS+Android this is 2 OS prompts total (1 create + 1
    # dual-salt assert) thanks to the SDK's bulk-PRF path.
    prf_provider = CustomPrfProvider()
    passkey = PasskeyClient(prf_provider, None, None)

    response = await passkey.register(RegisterRequest(label="personal"))

    # Hosts SHOULD persist credential.credential_id (for excludeCredentialIds
    # bookkeeping) and credential.user_id (for server-side correlation).
    # The SDK generates user_id; it is never host-supplied.
    _persisted_credential_id = response.credential.credential_id
    _persisted_user_id = response.credential.user_id

    config = default_config(network=Network.MAINNET)
    sdk = await connect(
        ConnectRequest(config=config, seed=response.wallet.seed, storage_dir="./.data")
    )
    # ANCHOR_END: register-passkey
    return sdk


async def list_labels() -> list[str]:
    # ANCHOR: list-labels
    prf_provider = CustomPrfProvider()
    config = PasskeyConfig(
        # Optional: override the default label used when
        # register / sign_in receive `label = None`. Falls back to the
        # SDK's internal "Default" when unset.
        default_label="personal",
    )
    # breez_api_key enables authenticated (NIP-42) Breez relay access
    # for label sync; pass None for public-relay-only.
    passkey = PasskeyClient(prf_provider, "<breez api key>", config)

    # sign_in with no label runs in discovery mode: it derives the
    # master seed AND lists labels in the same ceremony, so a follow-up
    # labels().list() reads from the cached identity for free.
    labels = await passkey.labels().list()

    for label in labels:
        print(f"Found label: {label}")
    # ANCHOR_END: list-labels
    return labels


async def store_label():
    # ANCHOR: store-label
    prf_provider = CustomPrfProvider()
    passkey = PasskeyClient(prf_provider, "<breez api key>", None)

    # For a new label on an existing identity, call sign_in(new_label)
    # first to warm the SDK's identity cache, THEN
    # labels().store() uses the cached identity for free (1 OS prompt total).
    await passkey.labels().store(label="personal")
    # ANCHOR_END: store-label




async def check_domain():
    # ANCHOR: domain-association
    # Verify Apple AASA / Android Asset Links / Web Related Origins
    # before the first WebAuthn ceremony. Diagnostic only: never blocks.
    prf_provider = CustomPrfProvider()
    result = await prf_provider.check_domain_association()

    if isinstance(result, DomainAssociation.ASSOCIATED):
        pass  # Safe to proceed.
    elif isinstance(result, DomainAssociation.NOT_ASSOCIATED):
        print(f"Domain association failed (source={result.source}): {result.reason}")
        return
    elif isinstance(result, DomainAssociation.SKIPPED):
        pass  # Verification could not be performed; proceed normally.
    # ANCHOR_END: domain-association


async def recover_from_already_exists():
    # ANCHOR: recover-already-exists
    # The OS rejected register because the user's password manager
    # already holds a credential matching `exclude_credential_ids`.
    # Route the user to the sign-in path: the OS picker will surface
    # the existing credential and the SDK's identity cache will warm
    # up on the assertion.
    prf_provider = CustomPrfProvider()
    passkey = PasskeyClient(prf_provider, None, None)

    try:
        await passkey.register(
            RegisterRequest(
                label="personal",
                exclude_credential_ids=[
                    # app-persisted credential IDs from prior registrations
                ],
            )
        )
    except PrfProviderError.CredentialAlreadyExists:
        # Flip to sign-in. The existing credential's PRF output is
        # the same seed the host would have minted on register.
        response = await passkey.sign_in(SignInRequest(label="personal"))
        return response.wallet
    # ANCHOR_END: recover-already-exists


async def handle_timeout():
    # ANCHOR: handle-timeout
    # The OS biometric inactivity timeout (~55s+) tore down the prompt
    # without user intent. Distinct from a real cancel: hosts may
    # surface a re-prompt UI without treating it as the user opting
    # out. The SDK fires PrfProviderError.UserTimedOut when assertion
    # or register elapsed time crosses 55_000 ms.
    prf_provider = CustomPrfProvider()
    passkey = PasskeyClient(prf_provider, None, None)

    try:
        return await passkey.sign_in(SignInRequest(label="personal"))
    except PrfProviderError.UserTimedOut:
        print("Sign-in timed out: show \"Try Again\" UI.")
        raise
    # ANCHOR_END: handle-timeout
