# PRF providers

The built-in `PasskeyProvider` covers the common case. Reach for this page when:

- You need platform-specific provider options (iOS `URLSession` / presentation anchor, Android `Activity` wiring, web `authenticatorAttachment`).
- You're integrating Python, Go, or C# (no built-in `PasskeyProvider` ships for those bindings).
- You need a custom `PrfProvider` (CLI YubiKey, FIDO2, air-gapped backup file, hardware module).

## Built-in PasskeyProvider options

The built-in `PasskeyProvider` takes a `PasskeyProviderOptions`:

| Field | Default | Description |
|--------|---------|-------------|
| `rpId` | Breez shared RP | Relying Party ID. Your app's domain, or `PasskeyProvider.BREEZ_RP_ID` (`keys.breez.technology`) if Breez-registered. Changing it makes existing passkeys derive a different seed (see [migration considerations](https://github.com/breez/passkey-login/blob/main/SDK%20implementation.md#passkey-migration-considerations)). |
| `rpName` | `"Breez"` | Display name for your app, shown in some authenticator UIs. Registration-only. |
| `userName` | `rpName` | Account identifier shown beneath the display name in the OS picker, e.g. `john@doe.com`. Pass a stable per-user value so each registration is a distinct entry (Apple Passwords dedupes by `(rpId, user.name)`). Registration-only. |
| `userDisplayName` | `userName` | Human-friendly name shown most prominently, e.g. `John Doe`. Registration-only. |

The same `PasskeyProviderOptions` is settable on `passkeyConfig` via `providerOptions`, which builds the provider for you (see [Initialization](./passkey_onboarding.md#initialization)). Construct `PasskeyProvider` directly only for platform-specific options (iOS `URLSession`, web `authenticatorAttachment`) or a custom backend.

**C# / Go / Python limitation**

The SDK does not ship a built-in `PasskeyProvider` for C#, Go, or Python (no native passkey API to wrap). On those bindings, implement your own `PrfProvider` and pass it to `PasskeyClient`.

## Custom PrfProvider

To support a custom authenticator (hardware security key, FIDO2/CTAP2 transport, air-gapped backup file), implement the `PrfProvider` interface directly. The Breez CLI ships [YubiKey](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/yubikey_prf.rs), [FIDO2](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/fido2_prf.rs), and [file-based](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/file_prf.rs) implementations as references.

```typescript
// Implement PrfProvider for a custom authenticator (hardware key, FIDO2,
// file-backed). Every method is required: the generated interface has no
// defaults, unlike the Rust trait.
class CustomPrfProvider {
  deriveSeeds = async (
    _request: { salts: string[] }
  ): Promise<{ seeds: Uint8Array[], credentialId?: Uint8Array }> => {
    // Return one 32-byte PRF output per salt, in input order.
    throw new Error('Implement using WebAuthn or native passkey APIs')
  }

  createPasskey = async (
    _excludeCredentials: Uint8Array[],
    _salts: string[]
  ): Promise<CreatePasskeyOutput> => {
    // Register a credential and return its ID plus attestation.
    //
    // Return `seeds: null` (the field is nullable) unless the platform evaluated PRF during the
    // create ceremony and gave one output per salt. Seeds returned here
    // must equal what `deriveSeeds` returns for the same salts.
    throw new Error('Implement registration via native passkey API')
  }

  isSupported = async (): Promise<boolean> => {
    throw new Error('Check platform passkey availability')
  }
}
```
