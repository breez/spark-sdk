# PRF providers

The built-in {{#name PasskeyProvider}} covers the common case. Reach for this page when:

- You need platform-specific provider options (iOS `URLSession` / presentation anchor, Android `Activity` wiring, web `authenticatorAttachment`).
- You're integrating Python, Go, or C# (no built-in {{#name PasskeyProvider}} ships for those bindings).
- You need a custom {{#name PrfProvider}} (CLI YubiKey, FIDO2, air-gapped backup file, hardware module).

## Built-in PasskeyProvider options

The built-in {{#name PasskeyProvider}} takes a {{#name PasskeyProviderOptions}}:

| Field | Default | Description |
|--------|---------|-------------|
| {{#name rp_id}} | Breez shared RP | Relying Party ID. Your app's domain, or `PasskeyProvider.BREEZ_RP_ID` (`keys.breez.technology`) if Breez-registered. Changing it makes existing passkeys derive a different seed (see [migration considerations](https://github.com/breez/passkey-login/blob/main/SDK%20implementation.md#passkey-migration-considerations)). |
| {{#name rp_name}} | `"Breez"` | Display name for your app, shown in some authenticator UIs. Registration-only. |
| {{#name user_name}} | {{#name rp_name}} | Account identifier shown beneath the display name in the OS picker, e.g. `john@doe.com`. Pass a stable per-user value so each registration is a distinct entry (Apple Passwords dedupes by `(rpId, user.name)`). Registration-only. |
| {{#name user_display_name}} | {{#name user_name}} | Human-friendly name shown most prominently, e.g. `John Doe`. Registration-only. |

The same {{#name PasskeyProviderOptions}} is settable on {{#name passkey_config}} via {{#name provider_options}}, which builds the provider for you (see [Initialization](./passkey_onboarding.md#initialization)). Construct {{#name PasskeyProvider}} directly only for platform-specific options (iOS `URLSession`, web `authenticatorAttachment`) or a custom backend.

<div class="warning">
<h4>C# / Go / Python limitation</h4>

The SDK does not ship a built-in {{#name PasskeyProvider}} for C#, Go, or Python (no native passkey API to wrap). On those bindings, implement your own {{#name PrfProvider}} and pass it to {{#name PasskeyClient}}.
</div>

## Custom PrfProvider

To support a custom authenticator (hardware security key, FIDO2/CTAP2 transport, air-gapped backup file), implement the {{#name PrfProvider}} interface directly. The Breez CLI ships [YubiKey](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/yubikey_prf.rs), [FIDO2](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/fido2_prf.rs), and [file-based](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/file_prf.rs) implementations as references.

{{#tabs passkey:implement-prf-provider}}
