# PRF providers

The built-in {{#name PasskeyProvider}} covers the common case. Reach for this page when:

- You need fine-grained `PasskeyProvider` options (custom `userName`, [CredentialRegistry](./passkey_credential_registry.md), etc.).
- You're integrating Python, Go, or C# (no built-in `PasskeyProvider` ships for those bindings).
- You need a custom `PrfProvider` (CLI YubiKey, FIDO2, air-gapped backup file, hardware module).

## Built-in PasskeyProvider options

In addition to `rpId` and `rpName`, the `PasskeyProvider` constructor accepts:

| Option | Default | Description |
|--------|---------|-------------|
| {{#name rp_id}} | **required** | Relying Party ID. Pass your app's domain, or `PasskeyProvider.BREEZ_RP_ID` (= `keys.breez.technology`) if your app is Breez-registered. Changing this means existing passkeys produce a different seed; see [migration considerations](https://github.com/breez/passkey-login/blob/main/SDK%20implementation.md#passkey-migration-considerations). |
| {{#name rp_name}} | **required** | Display name shown to the user in the OS passkey picker and credential-management UIs when choosing a credential. Only used at credential registration; changing it does not affect existing credentials. |
| {{#name user_name}} | {{#name rp_name}} | User name stored with the credential (registration only). On iOS this is the only field surfaced in the iCloud Keychain / passkey picker: the platform's `ASAuthorizationPlatformPublicKeyCredentialProvider` API has no `displayName` slot. Pass the per-credential friendly label here if you want users to see it. |
| {{#name user_display_name}} | {{#name user_name}} | Primary label shown in passkey picker on platforms that surface a separate display name (Android via WebAuthn JSON `user.displayName`, web via WebAuthn L3). iOS ignores. |
| {{#name credential_registry}} | none | Opt-in app-side store of known credential IDs. See [CredentialRegistry](./passkey_credential_registry.md). |

<div class="warning">
<h4>C# / Go / Python limitation</h4>

The SDK does not ship a built-in `PasskeyProvider` for C#, Go, or Python (no native passkey API to wrap on those targets). Integrators on those bindings implement their own `PrfProvider` and pass it to `PasskeyClient::new(...)` directly.
</div>

## Custom PrfProvider

If the built-in `PasskeyProvider` does not satisfy your requirements (e.g., you need a hardware security key, a FIDO2/CTAP2 transport, an air-gapped backup file, or a custom authenticator), implement the `PrfProvider` interface directly. The Breez CLI ships [YubiKey](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/yubikey_prf.rs), [FIDO2](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/fido2_prf.rs), and [file-based](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/file_prf.rs) implementations as references.

{{#tabs passkey:implement-prf-provider}}
