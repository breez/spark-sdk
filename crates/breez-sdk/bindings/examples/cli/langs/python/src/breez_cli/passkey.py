import hashlib
import hmac
import os
import sys
from pathlib import Path

from breez_sdk_spark import (
    DeriveSeedsOutput,
    DeriveSeedsRequest,
    DomainAssociation,
    PasskeyClient,
    PrfProvider,
    Seed,
    SignInRequest,
)

SECRET_FILE_NAME = "seedless-restore-secret"


class FilePrfProvider(PrfProvider):
    """File-based PRF provider using HMAC-SHA256 with a secret stored on disk.

    The secret is generated randomly on first use and persisted.
    Suitable for development and testing.
    """

    def __init__(self, data_dir: Path):
        secret_path = data_dir / SECRET_FILE_NAME
        if secret_path.exists():
            secret = secret_path.read_bytes()
            if len(secret) != 32:
                raise RuntimeError(
                    f"Invalid secret file: expected 32 bytes, got {len(secret)}"
                )
            self._secret = secret
        else:
            self._secret = os.urandom(32)
            data_dir.mkdir(parents=True, exist_ok=True)
            secret_path.write_bytes(self._secret)

    def _derive_one(self, salt: str) -> bytes:
        return hmac.new(self._secret, salt.encode(), hashlib.sha256).digest()

    async def derive_seeds(self, request: DeriveSeedsRequest) -> DeriveSeedsOutput:
        seeds = [self._derive_one(s) for s in request.salts]
        return DeriveSeedsOutput(seeds=seeds, credential_id=None)

    async def is_supported(self) -> bool:
        return True

    async def create_passkey(self, exclude_credentials):
        raise NotImplementedError

    async def check_domain_association(self) -> DomainAssociation:
        return DomainAssociation.SKIPPED(
            reason="File provider does not verify domain association"
        )


class YubiKeyPrfProvider(PrfProvider):
    """YubiKey HMAC-SHA1 challenge-response PRF provider.

    Not yet supported in the Python CLI. Requires a YubiKey with
    HMAC-SHA1 challenge-response configured on Slot 2.
    """

    def __init__(self):
        print(
            "YubiKey PRF provider is not yet supported in the Python CLI.",
            file=sys.stderr,
        )
        raise SystemExit(1)

    async def derive_seeds(self, request: DeriveSeedsRequest) -> DeriveSeedsOutput:
        raise NotImplementedError

    async def is_supported(self) -> bool:
        return False

    async def create_passkey(self, exclude_credentials):
        raise NotImplementedError

    async def check_domain_association(self) -> DomainAssociation:
        raise NotImplementedError


class Fido2PrfProvider(PrfProvider):
    """FIDO2/WebAuthn PRF provider using CTAP2 hmac-secret extension.

    Not yet supported in the Python CLI. Requires a FIDO2 authenticator
    with hmac-secret extension support.
    """

    def __init__(self, rp_id=None):
        print(
            "FIDO2 PRF provider is not yet supported in the Python CLI.",
            file=sys.stderr,
        )
        raise SystemExit(1)

    async def derive_seeds(self, request: DeriveSeedsRequest) -> DeriveSeedsOutput:
        raise NotImplementedError

    async def is_supported(self) -> bool:
        return False

    async def create_passkey(self, exclude_credentials):
        raise NotImplementedError

    async def check_domain_association(self) -> DomainAssociation:
        raise NotImplementedError


def create_provider(provider_name: str, data_dir: Path, rpid=None):
    """Create a PrfProvider based on provider name."""
    name = provider_name.lower()
    if name == "file":
        return FilePrfProvider(data_dir)
    if name == "yubikey":
        return YubiKeyPrfProvider()
    if name == "fido2":
        return Fido2PrfProvider(rp_id=rpid)
    raise ValueError(
        f"Invalid passkey provider '{provider_name}'. Use 'file', 'yubikey', or 'fido2'."
    )


async def resolve_passkey_seed(
    provider,
    breez_api_key,
    label,
    list_labels,
    store_label,
) -> Seed:
    """Resolve a Seed from a passkey PRF provider, with optional Nostr label operations."""
    client = PasskeyClient(provider, breez_api_key, None)

    if list_labels:
        print("Querying Nostr for available labels...")
        response = await client.sign_in(request=SignInRequest(label=None))

        if not response.labels:
            print("No labels found on Nostr for this identity")
            raise SystemExit(1)

        print("Available labels:")
        for i, name in enumerate(response.labels, 1):
            print(f"  {i}: {name}")

        selection = input(f"Select label (1-{len(response.labels)}): ").strip()
        try:
            idx = int(selection)
        except ValueError:
            print("Invalid selection")
            raise SystemExit(1)

        if idx < 1 or idx > len(response.labels):
            print("Selection out of range")
            raise SystemExit(1)

        label = response.labels[idx - 1]

    if store_label and label:
        print(f"Publishing label '{label}' to Nostr...")
        await client.labels().store(label=label)
        print(f"Label '{label}' published successfully.")

    response = await client.sign_in(request=SignInRequest(label=label))
    return response.wallet.seed
