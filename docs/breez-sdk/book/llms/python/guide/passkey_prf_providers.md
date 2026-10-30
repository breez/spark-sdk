# PRF providers

The built-in `PasskeyProvider` covers the common case. Reach for this page when:

- You need platform-specific provider options (iOS `URLSession` / presentation anchor, Android `Activity` wiring, web `authenticatorAttachment`).
- You're integrating Python, Go, or C# (no built-in `PasskeyProvider` ships for those bindings).
- You need a custom `PrfProvider` (CLI YubiKey, FIDO2, air-gapped backup file, hardware module).

## Built-in PasskeyProvider options

The built-in `PasskeyProvider` takes a `PasskeyProviderOptions`:

| Field | Default | Description |
|--------|---------|-------------|
| `rp_id` | Breez shared RP | Relying Party ID. Your app's domain, or `PasskeyProvider.BREEZ_RP_ID` (`keys.breez.technology`) if Breez-registered. Changing it makes existing passkeys derive a different seed (see [migration considerations](https://github.com/breez/passkey-login/blob/main/SDK%20implementation.md#passkey-migration-considerations)). |
| `rp_name` | `"Breez"` | Display name for your app, shown in some authenticator UIs. Registration-only. |
| `user_name` | `rp_name` | Account identifier shown beneath the display name in the OS picker, e.g. `john@doe.com`. Pass a stable per-user value so each registration is a distinct entry (Apple Passwords dedupes by `(rpId, user.name)`). Registration-only. |
| `user_display_name` | `user_name` | Human-friendly name shown most prominently, e.g. `John Doe`. Registration-only. |

The same `PasskeyProviderOptions` is settable on `passkey_config` via `provider_options`, which builds the provider for you (see [Initialization](./passkey_onboarding.md#initialization)). Construct `PasskeyProvider` directly only for platform-specific options (iOS `URLSession`, web `authenticatorAttachment`) or a custom backend.

**C# / Go / Python limitation**

The SDK does not ship a built-in `PasskeyProvider` for C#, Go, or Python (no native passkey API to wrap). On those bindings, implement your own `PrfProvider` and pass it to `PasskeyClient`.

## Custom PrfProvider

To support a custom authenticator (hardware security key, FIDO2/CTAP2 transport, air-gapped backup file), implement the `PrfProvider` interface directly. The Breez CLI ships [YubiKey](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/yubikey_prf.rs), [FIDO2](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/fido2_prf.rs), and [file-based](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/file_prf.rs) implementations as references.

```python
# Implement the PrfProvider trait for custom logic if no built-in
# PasskeyProvider ships for your target. Every method is required:
# derive_seeds for derivation, is_supported for the capability probe,
# create_passkey for registration, check_domain_association for the
# advisory RP check.
class CustomPrfProvider(PrfProvider):
    async def derive_seeds(self, request: DeriveSeedsRequest) -> DeriveSeedsOutput:
        # Return one 32-byte PRF output per salt, in input order.
        raise NotImplementedError("Implement using WebAuthn or native passkey APIs")

    async def is_supported(self) -> bool:
        raise NotImplementedError("Check platform passkey availability")

    async def create_passkey(
        self, exclude_credentials: list[bytes], salts: list[str]
    ) -> CreatePasskeyOutput:
        # Register a credential and return its ID plus attestation.
        #
        # Return seeds=None unless the platform evaluated PRF during the create
        # ceremony and gave one output per salt. Seeds returned here must equal
        # what derive_seeds returns for the same salts.
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
```
