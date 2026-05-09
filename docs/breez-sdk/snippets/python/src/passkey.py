# pylint: disable=duplicate-code
from breez_sdk_spark import (
    ConnectRequest,
    CreatePasskeyRequest,
    DomainAssociation,
    Network,
    NostrRelayConfig,
    PasskeyClient,
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


async def single_cta_onboarding():
    # ANCHOR: signin-fallback-register
    # Single-CTA onboarding: try silent sign_in first, fall through to
    # register on CredentialNotFound. The OS shows ONE prompt for a
    # returning user (silent assertion succeeds), TWO for a new user
    # (silent assertion fast-fails, then create + dual-salt assert).
    prf_provider = CustomPrfProvider()
    passkey = PasskeyClient(prf_provider, None)

    try:
        # Discovery mode (label=None): derives master + DEFAULT label
        # in a single ceremony. The fresh-device user fast-fails in
        # <300ms with no UI shown.
        response = await passkey.sign_in(SignInRequest(label=None, extra_salts=[]))
        return response.wallet
    except PrfProviderError.CredentialNotFound:
        # CredentialNotFound is the SDK's classification for "no matching
        # credential on this device", including iOS's <300ms fast-fail
        # case where the platform conflates no-cred with user-cancel.
        response = await passkey.register(
            RegisterRequest(
                label="personal",
                extra_salts=[],
                exclude_credential_ids=[],
            )
        )
        return response.wallet
    # ANCHOR_END: signin-fallback-register


async def check_domain():
    # ANCHOR: domain-association
    # Verify Apple AASA / Android Asset Links / Web Related Origins
    # before the first WebAuthn ceremony. Diagnostic only: never blocks.
    prf_provider = CustomPrfProvider()
    result = await prf_provider.check_domain_association()

    if isinstance(result, DomainAssociation.ASSOCIATED):
        # Safe to proceed.
        pass
    elif isinstance(result, DomainAssociation.NOT_ASSOCIATED):
        # Configuration is wrong (entitlement missing, AASA stale,
        # assetlinks malformed). Surface a developer-facing error.
        print(f"Domain association failed (source={result.source}): {result.reason}")
        return
    elif isinstance(result, DomainAssociation.SKIPPED):
        # Verification could not be performed (offline, endpoint
        # timeout, no public-suffix match). Proceed normally: this
        # is NOT a negative signal.
        pass
    # ANCHOR_END: domain-association


async def recover_from_already_exists():
    # ANCHOR: recover-already-exists
    # The OS rejected register because the user's password manager
    # already holds a credential matching `exclude_credential_ids`.
    # Route the user to the sign-in path: the OS picker will surface
    # the existing credential and the SDK's identity cache will warm
    # up on the assertion.
    prf_provider = CustomPrfProvider()
    passkey = PasskeyClient(prf_provider, None)

    try:
        await passkey.register(
            RegisterRequest(
                label="personal",
                extra_salts=[],
                exclude_credential_ids=[
                    # app-persisted credential IDs from prior registrations
                ],
            )
        )
    except PrfProviderError.CredentialAlreadyExists:
        # Flip to sign-in. The existing credential's PRF output is
        # the same wallet seed the host would have minted on register.
        response = await passkey.sign_in(
            SignInRequest(label="personal", extra_salts=[])
        )
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
    passkey = PasskeyClient(prf_provider, None)

    try:
        return await passkey.sign_in(SignInRequest(label="personal", extra_salts=[]))
    except PrfProviderError.UserTimedOut:
        # Show a sticky retry screen with timeout-specific copy.
        # Do NOT auto-retry without user input.
        print("Sign-in timed out: show \"Try Again\" UI.")
        raise
    # ANCHOR_END: handle-timeout
