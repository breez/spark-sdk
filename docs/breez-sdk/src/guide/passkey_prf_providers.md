# PRF providers

The built-in {{#name PasskeyProvider}} covers the common case. Reach for this page when:

- You need fine-grained {{#name PasskeyProvider}} options (custom {{#name user_name}}, {{#name user_display_name}}, etc.).
- You're integrating Python, Go, or C# (no built-in {{#name PasskeyProvider}} ships for those bindings).
- You need a custom {{#name PrfProvider}} (CLI YubiKey, FIDO2, air-gapped backup file, hardware module).

## Built-in PasskeyProvider options

In addition to {{#name rp_id}} and {{#name rp_name}}, the {{#name PasskeyProvider}} constructor accepts:

| Option | Default | Description |
|--------|---------|-------------|
| {{#name rp_id}} | **required** | Relying Party ID. Pass your app's domain, or `PasskeyProvider.BREEZ_RP_ID` (= `keys.breez.technology`) if your app is Breez-registered. Changing this means existing passkeys produce a different seed; see [migration considerations](https://github.com/breez/passkey-login/blob/main/SDK%20implementation.md#passkey-migration-considerations). |
| {{#name rp_name}} | **required** | Maps to the WebAuthn `rp.name`. Required by current OS prompts (deprecated in WebAuthn L3 but still enforced everywhere). Surfaces in some authenticators' management UIs (Apple Passwords, Google Password Manager); platform UIs increasingly ignore it. Set at registration only; changing it does not affect existing credentials. |
| {{#name user_name}} | {{#name rp_name}} | Maps to the WebAuthn `user.name`. Treated as the user's unique identifier for the credential and shown in the OS account picker during sign-in. Pass a stable per-user value if each registration should surface as a distinct entry (Apple's Passwords app, in particular, dedupes credentials by `(rpId, user.name)`; iOS additionally treats this as the only label exposed by `ASAuthorizationPlatformPublicKeyCredentialProvider`). Registration-only. |
| {{#name user_display_name}} | {{#name user_name}} | Maps to the WebAuthn `user.displayName`. The user-friendly label the OS / browser MAY (but is not required to) show in the picker; Android Credential Manager and the WebAuthn L3 picker surface it, iOS ignores. Registration-only. |

<div class="warning">
<h4>C# / Go / Python limitation</h4>

The SDK does not ship a built-in {{#name PasskeyProvider}} for C#, Go, or Python (no native passkey API to wrap on those targets). Integrators on those bindings implement their own {{#name PrfProvider}} and pass it to {{#name PasskeyClient}} directly.
</div>

## Custom PrfProvider

If the built-in {{#name PasskeyProvider}} does not satisfy your requirements (e.g., you need a hardware security key, a FIDO2/CTAP2 transport, an air-gapped backup file, or a custom authenticator), implement the {{#name PrfProvider}} interface directly. The Breez CLI ships [YubiKey](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/yubikey_prf.rs), [FIDO2](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/fido2_prf.rs), and [file-based](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/file_prf.rs) implementations as references.

{{#tabs passkey:implement-prf-provider}}
