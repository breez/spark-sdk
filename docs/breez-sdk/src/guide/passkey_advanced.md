# Advanced

Customization points most apps never need. {{#name createPasskeyClient}} handles the typical onboarding flow on every platform with a built-in `PasskeyProvider`. Reach for the topics here when:

- You need a [custom `PrfProvider`](./passkey_custom_provider.md) (CLI YubiKey, FIDO2, air-gapped backup file).
- You're integrating Python, Go, or C# (no built-in `PasskeyProvider` ships for those bindings).
- You want fine-grained `PasskeyProvider` options ([CredentialRegistry](./passkey_credential_registry.md), custom `userName`, etc.).
- You manage [multiple labels per identity](./passkey_labels.md) (e.g. a "create a new wallet" path on a returning user).
- You need [platform-specific diagnostics](./passkey_platforms.md) (raw domain-association probe, Capacitor bridge, CTAP2 salt transformation).

## Built-in PasskeyProvider options

The convenience factory only takes `rpId` and `rpName`. The underlying `PasskeyProvider` constructor accepts a few more knobs:

| Option | Default | Description |
|--------|---------|-------------|
| {{#name rp_id}} | **required** | Relying Party ID. Pass your app's domain, or `PasskeyProvider.BREEZ_RP_ID` (= `keys.breez.technology`) if your app is Breez-registered. Changing this means existing passkeys produce a different seed; see [migration considerations](https://github.com/breez/passkey-login/blob/main/SDK%20implementation.md#passkey-migration-considerations). |
| {{#name rp_name}} | **required** | Display name shown to the user in the OS passkey picker and credential-management UIs when choosing a credential. Only used at credential registration; changing it does not affect existing credentials. |
| {{#name user_name}} | {{#name rp_name}} | User name stored with the credential (registration only). On iOS this is the only field surfaced in the iCloud Keychain / passkey picker: the platform's `ASAuthorizationPlatformPublicKeyCredentialProvider` API has no `displayName` slot. Pass the per-credential friendly label here if you want users to see it. |
| {{#name user_display_name}} | {{#name user_name}} | Primary label shown in passkey picker on platforms that surface a separate display name (Android via WebAuthn JSON `user.displayName`, web via WebAuthn L3). iOS ignores. |
| {{#name credential_registry}} | none | Opt-in app-side store of known credential IDs. See [CredentialRegistry](./passkey_credential_registry.md). |

<div class="warning">
<h4>C# / Go / Python limitation</h4>

The SDK does not ship a built-in `PasskeyProvider` for C#, Go, or Python (no native passkey API to wrap on those targets). Integrators on those bindings implement their own `PrfProvider` and pass it to `PasskeyClient::new(...)` directly. See [Custom PrfProvider](./passkey_custom_provider.md).
</div>

## Built-in behaviours

The native built-in providers (iOS / Android / Flutter / RN) handle several platform quirks internally so consumers don't need workarounds:

- **Bulk PRF (single OS prompt for N salts).** {{#name derive_seeds}} uses the WebAuthn dual-salt extension (`saltInput1` + `saltInput2` on iOS, `prfFirst` + `prfSecond` on Android) when the authenticator supports it, falling back to per-salt assertions otherwise. The SDK's {{#name PasskeyClient.sign_in}} and {{#name PasskeyClient.register}} both go through this path so a master + label derivation costs **one** prompt where supported.
- **Post-create grace (800ms).** After a successful {{#name create_passkey}}, the next derive call holds briefly so the OS finishes indexing the new credential before the immediate post-register assertion. Without this, on Apple Passwords the dual-salt assertion can drop `prf.second` and force a fallback prompt; on Google Password Manager the credential can be briefly invisible to the picker.
- **Fast-fail on no-credential.** Assertions set `preferImmediatelyAvailableCredentials` (iOS) / `preferImmediatelyAvailableCredentials=true` (Android) so a missing credential surfaces as `CredentialNotFound` immediately rather than the cross-device "use another device" hybrid sheet.
- **Opt-in CredentialRegistry auto-merge.** Supplying a `CredentialRegistry` unions its IDs into `allowCredentialIds` before assertion and into `excludeCredentialIds` before registration, and auto-adds the asserted / created credential ID back. See [CredentialRegistry](./passkey_credential_registry.md) for reference impls.

These run by default. To override (e.g. for a custom credential-management UI), pass an explicit `allowCredentialIds`.
