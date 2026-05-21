# Initialization

{{#name PasskeyClient}} is the entry point for every passkey-derived wallet operation. It composes a {{#name PrfProvider}} (the platform-specific bridge to WebAuthn / Credential Manager / AuthenticationServices) with the SDK's internal Nostr-backed label store, then exposes register / sign-in / connect / labels / credentials to your app code. Construct one per app session and reuse it.

The recommended setup is the {{#name createPasskeyClient}} convenience factory, which builds the platform-default {{#name PasskeyProvider}} and forwards the Breez API key from your SDK {{#name Config}}:

{{#tabs passkey:setup-client}}

Parameters:

- **`rpId`**: Relying Party ID. Pass your app's domain, or `PasskeyProvider.BREEZ_RP_ID` (= `keys.breez.technology`) if your app is Breez-registered. Required because changing it later strands existing credentials.
- **`rpName`**: Display name shown to the user in the OS passkey picker and credential-management UIs (iCloud Keychain, Google Password Manager, 1Password, etc.) when choosing a credential. Required.
- **`sdkConfig`**: Your main SDK {{#name Config}}. The factory forwards `sdkConfig.api_key` to the Breez relay for authenticated (<a target="_blank" href="https://github.com/nostr-protocol/nips/blob/master/42.md">NIP-42</a>) label storage. Without an API key, label sync falls back to public relays only.
- **`passkeyConfig`** (optional): Carries {{#name default_label}}, the label used when {{#name PasskeyClient.register}} / {{#name PasskeyClient.sign_in}} receive no label. Falls back to the SDK's internal `"Default"` when unset.

The SDK also implements <a target="_blank" href="https://github.com/nostr-protocol/nips/blob/master/65.md">NIP-65</a> to discover and publish to additional public relays for redundancy.

`createPasskeyClient` is not surfaced on web; the WASM tab shows the equivalent direct construction. For custom PRF providers (CLI YubiKey, FIDO2, file-backed) or non-default platform options (`credentialRegistry`, `userName`, etc.), see [Advanced](./passkey_advanced.md).

## Checking passkey availability

Use {{#name PasskeyClient.check_availability}} to gate passkey UI elements. The single call collapses {{#name PrfProvider.is_supported}} and {{#name PrfProvider.check_domain_association}} into a tagged {{#name PasskeyAvailability}} value with four variants: `Available`, `PrfUnsupported`, `NotAssociated { source, reason }`, `Skipped { reason }`. Branch on the variant to fall back to mnemonic-based onboarding on unsupported platforms (Android < 9, iOS < 18) or surface configuration mistakes (missing entitlement, AASA not deployed) without firing a WebAuthn ceremony:

{{#tabs passkey:check-availability}}
