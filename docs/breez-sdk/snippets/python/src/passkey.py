# pylint: disable=duplicate-code
from typing import cast

from breez_sdk_spark import (
    ConnectRequest,
    ConnectWithPasskeyRequest,
    DeriveSeedsOutput,
    DeriveSeedsRequest,
    DomainAssociation,
    Network,
    PasskeyAvailability,
    PasskeyClient,
    PrfProvider,
    PrfProviderError,
    PasskeyCredential,
    RegisterRequest,
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
    async def derive_seeds(self, request: DeriveSeedsRequest) -> DeriveSeedsOutput:
        # Return one 32-byte PRF output per salt, in input order.
        raise NotImplementedError("Implement using WebAuthn or native passkey APIs")

    async def is_supported(self) -> bool:
        raise NotImplementedError("Check platform passkey availability")

    async def create_passkey(self, exclude_credentials: list[bytes]) -> PasskeyCredential:
        # Register a credential and return its ID plus attestation.
        raise NotImplementedError("Implement registration via native passkey API")

    async def check_domain_association(self) -> DomainAssociation:
        # Optional: verify the app's identity against the platform's
        # domain verification source. Custom providers without a
        # verification source return SKIPPED, which tells callers
        # "proceed with WebAuthn as normal". The UniFFI-generated
        # variant classes are reparented to DomainAssociation at
        # runtime but mypy can't see that, hence the cast.
        return cast(
            DomainAssociation,
            DomainAssociation.SKIPPED(
                reason="CustomPrfProvider does not verify domain association"
            ),
        )
# ANCHOR_END: implement-prf-provider


async def check_availability():
    prf_provider = CustomPrfProvider()
    passkey = PasskeyClient(prf_provider, None, None)

    # ANCHOR: check-availability
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
    prf_provider = CustomPrfProvider()
    passkey = PasskeyClient(prf_provider, None, None)

    # ANCHOR: connect-with-passkey
    # Silent sign-in for a returning user, fall-through to register on a fresh device.
    response = await passkey.connect_with_passkey(
        ConnectWithPasskeyRequest(label="personal")
    )

    config = default_config(network=Network.MAINNET)
    sdk = await connect(
        ConnectRequest(config=config, seed=response.wallet.seed, storage_dir="./.data")
    )
    # ANCHOR_END: connect-with-passkey
    return sdk


async def register_new_passkey():
    prf_provider = CustomPrfProvider()
    passkey = PasskeyClient(prf_provider, None, None)

    # ANCHOR: register-passkey
    response = await passkey.register(RegisterRequest(label="personal"))

    config = default_config(network=Network.MAINNET)
    sdk = await connect(
        ConnectRequest(config=config, seed=response.wallet.seed, storage_dir="./.data")
    )
    # ANCHOR_END: register-passkey
    return sdk


async def credential_metadata():
    prf_provider = CustomPrfProvider()
    passkey = PasskeyClient(prf_provider, None, None)

    # ANCHOR: credential-metadata
    response = await passkey.register(RegisterRequest(label="personal"))

    if response.credential is not None:
        # Persist to reopen the same wallet on sign-in
        print(response.credential.credential_id)
        # Authenticator model (display hint, unverified)
        print(response.credential.aaguid)
        # Whether the passkey syncs across devices
        print(response.credential.backup_eligible)

    # Pin the stored credential ID so the OS can't substitute a sibling
    # credential, which would derive a different wallet.
    sign_in_response = await passkey.sign_in(
        SignInRequest(
            label="personal",
            allow_credentials=[
                # stored credential_id bytes
            ],
        )
    )
    # Pass to connect() to open the wallet
    print(sign_in_response.wallet.seed)
    # Label this wallet was derived from
    print(sign_in_response.wallet.label)
    # This passkey's labels (populated on discovery sign-in)
    print(sign_in_response.labels)
    # Credential signed in with (credential_id only)
    print(sign_in_response.credential)
    # ANCHOR_END: credential-metadata


async def list_labels() -> list[str]:
    prf_provider = CustomPrfProvider()
    passkey = PasskeyClient(prf_provider, "<breez api key>", None)
    # ANCHOR: list-labels
    labels = await passkey.labels().list()
    for label in labels:
        print(f"Found label: {label}")
    # ANCHOR_END: list-labels
    return labels


async def store_label():
    prf_provider = CustomPrfProvider()
    passkey = PasskeyClient(prf_provider, "<breez api key>", None)
    # ANCHOR: store-label
    await passkey.labels().store(label="personal")
    # ANCHOR_END: store-label




async def check_domain():
    # ANCHOR: domain-association
    # Diagnostic only: never blocks the ceremony.
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
    prf_provider = CustomPrfProvider()
    passkey = PasskeyClient(prf_provider, None, None)

    # ANCHOR: recover-already-exists
    try:
        await passkey.register(
            RegisterRequest(
                label="personal",
                exclude_credentials=[
                    # app-persisted credential IDs from prior registrations
                ],
            )
        )
    except PrfProviderError.CredentialAlreadyExists:
        # A matching credential already exists; sign in to it instead.
        response = await passkey.sign_in(SignInRequest(label="personal"))
        return response.wallet
    # ANCHOR_END: recover-already-exists


async def handle_timeout():
    prf_provider = CustomPrfProvider()
    passkey = PasskeyClient(prf_provider, None, None)

    # ANCHOR: handle-timeout
    # Biometric inactivity timeout, distinct from a user cancel.
    try:
        return await passkey.sign_in(SignInRequest(label="personal"))
    except PrfProviderError.UserTimedOut:
        # Show a retry UI. Do NOT auto-retry without user input.
        print("Sign-in timed out: show \"Try Again\" UI.")
        raise
    # ANCHOR_END: handle-timeout
