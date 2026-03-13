import hashlib
import hmac
import os
import sys
from pathlib import Path

from breez_sdk_spark import (
    NostrRelayConfig,
    Passkey,
    PasskeyPrfProvider,
    Seed,
)

SECRET_FILE_NAME = "seedless-restore-secret"


class FilePrfProvider(PasskeyPrfProvider):
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

    async def derive_prf_seed(self, salt: str):
        return hmac.new(self._secret, salt.encode(), hashlib.sha256).digest()

    async def is_prf_available(self):
        return True


class YubiKeyPrfProvider(PasskeyPrfProvider):
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

    async def derive_prf_seed(self, salt: str):
        raise NotImplementedError

    async def is_prf_available(self):
        return False


class Fido2PrfProvider(PasskeyPrfProvider):
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

    async def derive_prf_seed(self, salt: str):
        raise NotImplementedError

    async def is_prf_available(self):
        return False


def create_provider(provider_name: str, data_dir: Path, rpid=None):
    """Create a PasskeyPrfProvider based on provider name."""
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
    relay_config = NostrRelayConfig(breez_api_key=breez_api_key)
    passkey = Passkey(provider, relay_config)

    # --store-label: publish the label to Nostr
    if store_label and label:
        print(f"Publishing label '{label}' to Nostr...")
        await passkey.store_label(label=label)
        print(f"Label '{label}' published successfully.")

    # --list-labels: query Nostr and prompt user to select
    if list_labels:
        print("Querying Nostr for available labels...")
        labels = await passkey.list_labels()

        if not labels:
            print("No labels found on Nostr for this identity")
            raise SystemExit(1)

        print("Available labels:")
        for i, name in enumerate(labels, 1):
            print(f"  {i}: {name}")

        selection = input(f"Select label (1-{len(labels)}): ").strip()
        try:
            idx = int(selection)
        except ValueError:
            print("Invalid selection")
            raise SystemExit(1)

        if idx < 1 or idx > len(labels):
            print("Selection out of range")
            raise SystemExit(1)

        label = labels[idx - 1]

    wallet = await passkey.get_wallet(label)
    return wallet.seed
