# PRF providers

The built-in `PasskeyProvider` covers the common case. Reach for this page when:

- You need platform-specific provider options (iOS `URLSession` / presentation anchor, Android `Activity` wiring, web `authenticatorAttachment`).
- You're integrating Python, Go, or C# (no built-in `PasskeyProvider` ships for those bindings).
- You need a custom `PrfProvider` (CLI YubiKey, FIDO2, air-gapped backup file, hardware module).

## Built-in PasskeyProvider options

The built-in `PasskeyProvider` takes a `PasskeyProviderOptions`:

| Field | Default | Description |
|--------|---------|-------------|
| `RpId` | Breez shared RP | Relying Party ID. Your app's domain, or `PasskeyProvider.BREEZ_RP_ID` (`keys.breez.technology`) if Breez-registered. Changing it makes existing passkeys derive a different seed (see [migration considerations](https://github.com/breez/passkey-login/blob/main/SDK%20implementation.md#passkey-migration-considerations)). |
| `RpName` | `"Breez"` | Display name for your app, shown in some authenticator UIs. Registration-only. |
| `UserName` | `RpName` | Account identifier shown beneath the display name in the OS picker, e.g. `john@doe.com`. Pass a stable per-user value so each registration is a distinct entry (Apple Passwords dedupes by `(rpId, user.name)`). Registration-only. |
| `UserDisplayName` | `UserName` | Human-friendly name shown most prominently, e.g. `John Doe`. Registration-only. |

The same `PasskeyProviderOptions` is settable on `PasskeyConfig` via `ProviderOptions`, which builds the provider for you (see [Initialization](./passkey_onboarding.md#initialization)). Construct `PasskeyProvider` directly only for platform-specific options (iOS `URLSession`, web `authenticatorAttachment`) or a custom backend.

**C# / Go / Python limitation**

The SDK does not ship a built-in `PasskeyProvider` for C#, Go, or Python (no native passkey API to wrap). On those bindings, implement your own `PrfProvider` and pass it to `PasskeyClient`.

## Custom PrfProvider

To support a custom authenticator (hardware security key, FIDO2/CTAP2 transport, air-gapped backup file), implement the `PrfProvider` interface directly. The Breez CLI ships [YubiKey](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/yubikey_prf.rs), [FIDO2](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/fido2_prf.rs), and [file-based](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/file_prf.rs) implementations as references.

```csharp
// Implement PrfProvider for a custom authenticator (hardware key, FIDO2,
// file-backed). Every method is required: interface members are
// bodiless, unlike the Rust trait's defaults.
class CustomPrfProvider : PrfProvider
{
    public async Task<DeriveSeedsOutput> DeriveSeeds(DeriveSeedsRequest request)
    {
        // Return one 32-byte PRF output per salt, in input order.
        throw new NotImplementedException("Implement using WebAuthn or native passkey APIs");
    }

    public async Task<bool> IsSupported()
    {
        throw new NotImplementedException("Check platform passkey availability");
    }

    public async Task<CreatePasskeyOutput> CreatePasskey(byte[][] excludeCredentials, string[] salts)
    {
        // Register a credential and return its ID plus attestation.
        //
        // Return a null Seeds unless the platform evaluated PRF during the
        // create ceremony and gave one output per salt. Seeds returned here
        // must equal what DeriveSeeds returns for the same salts.
        throw new NotImplementedException("Implement registration via native passkey API");
    }

    public async Task<DomainAssociation> CheckDomainAssociation()
    {
        return await Task.FromResult<DomainAssociation>(
            new DomainAssociation.Skipped("CustomPrfProvider does not verify domain association"));
    }

}
```
