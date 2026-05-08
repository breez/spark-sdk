# Passkey Login

Passkey Login lets users access their wallet with biometrics (fingerprint, face scan, or device PIN) instead of writing down and safeguarding a seed phrase. The SDK uses the <a target="_blank" href="https://w3c.github.io/webauthn/#prf-extension">WebAuthn PRF extension</a> to deterministically derive wallet keys from a passkey. Keys are never stored; they're regenerated on demand each time the user authenticates. The protocol also supports multiple wallets, each derived from a different label, with labels discoverable via Nostr relays.

For the full technical specification, see the <a target="_blank" href="https://github.com/breez/passkey-login/blob/main/spec.md">Passkey Login spec</a>.

## Application configuration

### Relying Party ID

The domain `keys.breez.technology` serves as a common Relying Party (RP) that enables cross-app passkey sharing. Applications that use this RP ID allow users to access the same passkey credentials across different platforms and apps.

To enable this cross-domain passkey sharing, `keys.breez.technology` serves three configuration files that declare which origins and apps are authorized to use it as an RP ID.

#### Web: Related Origins

**File**: `https://keys.breez.technology/.well-known/webauthn`

Declares which web origins can use the centralized RP ID for WebAuthn operations:

```json
{
  "related_origins": [
    "https://keys.breez.technology",
    "https://your-app.example.com"
  ]
}
```

**Default RP domain**: [Contact us](mailto:contact@breez.technology?subject=Passkey%20configuration) to register your web origin. Breez manages the configuration files for registered apps.

**Custom RP domain**: Host this file on your own domain at `/.well-known/webauthn`.

**Requirements**:
- Chrome 116+, Safari 18+, Edge 116+
- HTTPS (localhost is exempt during development)

#### Android: Asset Links

**File**: `https://keys.breez.technology/.well-known/assetlinks.json`

Establishes digital asset links between the domain and Android applications:

```json
[
  {
    "relation": [
      "delegate_permission/common.handle_all_urls",
      "delegate_permission/common.get_login_creds"
    ],
    "target": {
      "namespace": "android_app",
      "package_name": "com.example.yourapp",
      "sha256_cert_fingerprints": [
        "B6:16:AD:FE:C5:C6:D3:4C:93:01:5B:4A:79:20:21:4E:62:43:AB:29:28:EE:34:9A:F2:46:55:4B:54:FC:42:DF"
      ]
    }
  }
]
```

**Default RP domain**: [Contact us](mailto:contact@breez.technology?subject=Passkey%20configuration) to register your Android app. Breez manages the configuration files for registered apps.

**Custom RP domain**: Host this file on your own domain at `/.well-known/assetlinks.json`. Replace `com.example.yourapp` with your application package name and the fingerprint with your app's signing certificate SHA256 fingerprint. See the <a target="_blank" href="https://developers.google.com/digital-asset-links/v1/getting-started">Digital Asset Links</a> documentation and <a target="_blank" href="https://developer.android.com/identity/credential-manager/prerequisites">Credential Manager prerequisites</a> for details.

**Requirements**:
- Android 9+ (API 28) with Google Play Services, or Android 14+ (API 34) with any compatible credential provider
- `compileSdkVersion` must be at least 34 (required by the `androidx.credentials` library, not the device)

<div class="warning">
<h4>Android emulators</h4>
Most Android emulator images ship with Google Play Services as the only available credential provider, and many don't have a passkey-capable provider at all. Physical devices are recommended for passkey testing.
</div>

<div class="warning">
<h4>Credential provider requirement</h4>
On Android 9-13, passkeys are provided via Google Play Services and require Google Password Manager to be present and up to date. Android 14+ adds native passkey support and can integrate with any installed credential provider. However, at least one credential provider must be installed for passkeys to function (e.g., Google Password Manager, Bitwarden, 1Password etc.).
</div>

#### iOS / macOS: Apple App Site Association

**File**: `https://keys.breez.technology/.well-known/apple-app-site-association`

Connects the domain to iOS and macOS applications for passkey sharing:

```json
{
  "webcredentials": {
    "apps": [
      "TEAMID.com.example.yourapp"
    ]
  }
}
```

Your app must have the <a target="_blank" href="https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.associated-domains">Associated Domains</a> capability enabled. In Xcode, go to **Signing & Capabilities** → add **Associated Domains** → add the entry `webcredentials:keys.breez.technology`.

<div class="warning">
<h4>Expo Managed Workflow</h4>
If you're using Expo, the Breez SDK plugin can configure this automatically. See the <a href="install_react_native.html#plugin-options">React Native/Expo installation guide</a> for details on the <code>enablePasskey</code> option.
</div>

**Default RP domain**: [Contact us](mailto:contact@breez.technology?subject=Passkey%20configuration) to register your iOS app. Breez manages the configuration files for registered apps.

**Custom RP domain**: Host this file on your own domain at `/.well-known/apple-app-site-association`. Replace `TEAMID` with your Apple Developer Team ID and `com.example.yourapp` with your bundle identifier.

**Requirements**:
- iOS 18.0+, macOS 15.0+

<div class="warning">
<h4>Associated Domains entitlement must be enabled</h4>
Without the Associated Domains entitlement, passkey operations will fail with a configuration error even though {{#name is_prf_available}} returns `true` (the OS-level check can't verify entitlements at runtime).
</div>

### Nostr relay configuration

The SDK uses Nostr relays to store and discover labels. Configure relay access by passing a {{#name NostrRelayConfig}} when constructing the {{#name Passkey}} instance:

- {{#name breez_api_key}} - Your Breez API key. When provided, the SDK connects to the Breez-managed relay with <a target="_blank" href="https://github.com/nostr-protocol/nips/blob/master/42.md">NIP-42</a> authentication.
- {{#name timeout_secs}} - Connection timeout in seconds (defaults to 30).

The SDK also implements <a target="_blank" href="https://github.com/nostr-protocol/nips/blob/master/65.md">NIP-65</a> to discover and publish to additional public relays for redundancy. See the [Listing labels](#listing-labels) and [Storing a label](#storing-a-label) code examples below for usage.

## PRF providers

A PRF provider handles passkey credential registration, assertion, and PRF evaluation against the platform's authenticator. The SDK uses the seed it returns to deterministically derive wallet keys.

### Built-in PasskeyProvider

The SDK ships a built-in `PasskeyProvider` on every platform with native passkey support: Web (browsers), iOS, macOS, Android, Kotlin Multiplatform, React Native, and Flutter. Each one implements the underlying `PrfProvider` trait against the platform's WebAuthn / Credential Manager / AuthenticationServices APIs, so most apps can use it as-is without writing any glue code.

Built-in providers are not currently available for C#, Go, and Python.

**Constructor options** (all built-in providers):

| Option | Default | Description |
|--------|---------|-------------|
| {{#name rp_id}} | `keys.breez.technology` | Relying Party ID. Changing this means existing passkeys produce a different seed; see [migration considerations](https://github.com/breez/passkey-login/blob/main/SDK%20implementation.md#passkey-migration-considerations). |
| {{#name rp_name}} | `Breez SDK` | RP display name (registration only, does not affect existing credentials) |
| {{#name user_name}} | {{#name rp_name}} | User name stored with the credential (registration only). On iOS this is the only field surfaced in the iCloud Keychain / passkey picker — the platform's `ASAuthorizationPlatformPublicKeyCredentialProvider` API has no `displayName` slot. Pass the per-credential friendly label here if you want users to see it. |
| {{#name user_display_name}} | {{#name user_name}} | Primary label shown in passkey picker on platforms that surface a separate display name (Android via WebAuthn JSON `user.displayName`, web via WebAuthn L3). iOS ignores. |
| {{#name auto_register}} | `false` | When `true`, {{#name derive_seeds}} auto-creates a new passkey on `CredentialNotFound` and retries. When `false` (default), throws so the host can drive registration explicitly via {{#name create_passkey}}. |
| {{#name allow_credential_ids}} | empty | Pin assertion to specific credential IDs. Empty = the platform picks any credential matching the RP. Set when binding sign-in to a specific cred (e.g. when multiple Glow passkeys exist for the same RP). |

Apps that share an RP ID, whether the default `keys.breez.technology` or a custom domain, recognize the same user's passkey without per-app enrollment.

Pass a `PasskeyProvider` instance to `PasskeyClient` as shown in the [Connecting with a passkey](#connecting-with-passkey) snippets below. That is the recommended path for almost every app.

### Built-in behaviours

The native built-in providers (iOS / Android / Flutter / RN) handle several platform quirks internally so consumers don't need workarounds:

- **Bulk PRF (single OS prompt for N salts).** {{#name derive_seeds}} uses the WebAuthn dual-salt extension (`saltInput1` + `saltInput2` on iOS, `prfFirst` + `prfSecond` on Android) when the authenticator supports it, falling back to per-salt assertions otherwise. The SDK's {{#name PasskeyClient.sign_in}} and {{#name PasskeyClient.register}} both go through this path so a master + label derivation costs **one** prompt where supported.
- **Post-create grace (800ms).** After a successful {{#name create_passkey}}, the next derive call holds briefly so the OS finishes indexing the new credential before the immediate post-register assertion. Without this, on Apple Passwords the dual-salt assertion can drop `prf.second` and force a fallback prompt; on Google Password Manager the credential can be briefly invisible to the picker.
- **Capture-on-sign-in.** Every successful assertion auto-adds the credential ID to the iCloud-Keychain-synced (iOS) or Block-Store-backed (Android) `KnownCredentialsStore`. This migrates pre-tracking credentials so subsequent registrations correctly hit the platform-level "already exists" guard via `excludeCredentialIds`.
- **Fast-fail on no-credential.** Assertions set `preferImmediatelyAvailableCredentials` (iOS) / `preferImmediatelyAvailableCredentials=true` (Android) so a missing credential surfaces as `CredentialNotFound` immediately rather than the cross-device "use another device" hybrid sheet.
- **`allowCredentialIds` auto-merge.** Assertions implicitly include the IDs in `KnownCredentialsStore` as the allow-list, so the OS auto-routes to the just-registered credential and skips the picker between create and the immediate post-create assert.

These run by default. Hosts that need to override (e.g. for a custom credential-management UI) can pass an explicit `allowCredentialIds`.

## Domain association diagnostic

{{#name check_domain_association}} performs an Apple-app-site-association probe (iOS) or Digital Asset Links lookup (Android) against the configured RP and returns a typed {{#name DomainAssociation}} — `Associated`, `NotAssociated { source, reason }`, or `Skipped { reason }`. Use it to surface configuration mistakes (entitlement missing, AASA file not deployed) before a WebAuthn ceremony fails opaquely. The SDK never blocks on this check; treat `Skipped` as advisory.

{{#tabs passkey:domain-association}}

## Checking passkey availability

Use {{#name is_available}} to gate passkey UI elements. This returns `false` on unsupported platforms (e.g., Android < 9, iOS < 18), allowing you to fall back to mnemonic-based onboarding gracefully:

{{#tabs passkey:check-availability}}

<h2 id="connecting-with-passkey">
    <a class="header" href="#connecting-with-passkey">Connecting with a passkey</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/passkey/struct.PasskeyClient.html#method.sign_in">API docs</a>
</h2>

To connect with a passkey, instantiate the built-in `PasskeyProvider`, pass it to `PasskeyClient`, call {{#name PasskeyClient.sign_in}} to derive a wallet, then pass its seed to {{#name connect}}. The label defaults to discovery mode when omitted (the SDK lists existing labels via Nostr in the same ceremony).

{{#tabs passkey:connect-with-passkey}}

For a brand-new user with no existing passkey, call {{#name PasskeyClient.register}} instead — it creates the credential AND derives the wallet seed in one orchestrated call. Hosts that want a single-CTA onboarding can try `sign_in` first and fall through to `register` when the SDK returns {{#enum PrfProviderError::CredentialNotFound}}; on iOS+Android with `preferImmediatelyAvailableCredentials` this fast-fails (sub-300ms, no UI) when no credential exists, so the silent probe is cheap.

{{#tabs passkey:signin-fallback-register}}

<h2 id="listing-labels">
    <a class="header" href="#listing-labels">Listing labels</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/passkey/struct.PasskeyClient.html#method.list_labels">API docs</a>
</h2>

Discover labels associated to the passkey using Nostr. {{#name PasskeyClient.sign_in}} already lists labels in discovery mode (when no `label` is specified), so a separate `list_labels` call is only needed when re-fetching the label set after sign-in.

{{#tabs passkey:list-labels}}

<h2 id="storing-a-label">
    <a class="header" href="#storing-a-label">Storing a label</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/passkey/struct.PasskeyClient.html#method.store_label">API docs</a>
</h2>

Publish a label to Nostr so it can be discovered later. {{#name PasskeyClient.register}} publishes the label automatically on registration; use {{#name PasskeyClient.store_label}} only when adding a new label to an existing identity (e.g. a "create a new wallet" path on a returning user).

{{#tabs passkey:store-label}}

## Error recovery

Two error variants on {{#name PrfProviderError}} carry recovery actions hosts should branch on:

- {{#enum PrfProviderError::CredentialAlreadyExists}}: the OS rejected `register` because the user's password manager already holds a matching credential. Flip to {{#name PasskeyClient.sign_in}} so the OS picker surfaces the existing credential. Web emits a typed `PasskeyAlreadyExistsError`; native bindings emit the `CredentialAlreadyExists` variant.

{{#tabs passkey:recover-already-exists}}

- {{#enum PrfProviderError::UserTimedOut}}: the OS biometric inactivity timeout (~55 seconds) tore down the prompt without user intent. Distinct from a real cancel — surface a sticky retry UI with timeout-specific copy. Do not auto-retry without user input. Web emits a typed `PasskeyTimedOutError`; native bindings emit the `UserTimedOut` variant.

{{#tabs passkey:handle-timeout}}

For the full recovery-path table (`UserCancelled` vs `CredentialNotFound` vs `Configuration`, plus the iOS sub-300ms fast-fail nuance), see the [UX guide](./uxguide_passkey.md).

## Custom PrfProvider (advanced)

If the built-in `PasskeyProvider` does not satisfy your requirements (e.g., you need a hardware security key, a FIDO2/CTAP2 transport, an air-gapped backup file, or a custom authenticator), implement the `PrfProvider` interface directly. The Breez CLI ships [YubiKey](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/yubikey_prf.rs), [FIDO2](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/fido2_prf.rs), and [file-based](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/file_prf.rs) implementations as references.

{{#tabs passkey:implement-prf-provider}}

### Capacitor plugins

When the host is a Capacitor app, the JS layer talks to the iOS Swift / Android Kotlin native plugin through Capacitor's bridge. The SDK ships a TypeScript-only contract sub-export so plugin authors can keep their `definitions.ts` in lockstep with the canonical native shape:

```ts
import type {
  PasskeyPrfPlugin,
  DomainAssociation,
} from '@breeztech/breez-sdk-spark/passkey-capacitor-bridge';

import { registerPlugin } from '@capacitor/core';

export const PasskeyPrf =
  registerPlugin<PasskeyPrfPlugin>('PasskeyPrf');
```

Mirror the contract on both sides. The interface matches the canonical iOS `PasskeyAssertionCore` and Android `CredentialManagerPrfCore` plugin surface bundled with the SDK. `Uint8Array` values are exchanged as base64-url-safe strings (no padding), since Capacitor's bridge cannot transport binary data directly.

### Platform considerations

- **Web (browsers)**: Use the WebAuthn API with the `prf` extension. Browsers handle the salt transformation internally. Use discoverable credentials (`residentKey: 'required'`) with empty `allowCredentials` for assertion so the browser discovers the credential by RP ID.
- **Android / iOS**: Use native passkey APIs with PRF support. Ensure the Associated Domains / Asset Links configuration is in place for `keys.breez.technology`.
- **CLI / Desktop (CTAP2)**: Use the `hmac-secret` extension directly. Non-browser implementations must apply the WebAuthn salt transformation manually to produce the same PRF output as browsers:

  ```
  actualSalt = SHA-256("WebAuthn PRF" || 0x00 || developerSalt)
  ```

  This transformation is defined in the <a target="_blank" href="https://w3c.github.io/webauthn/#prf-extension">W3C WebAuthn PRF extension spec</a> and ensures that the same passkey + salt produces identical seeds across browser and native implementations.

## Supported specs

- [Seedless Restore](https://github.com/breez/seedless-restore) Passkey-based wallet derivation and discovery
- [Nostr](https://github.com/nostr-protocol/nostr) Relay-based event protocol for label storage
- [NIP-42](https://github.com/nostr-protocol/nips/blob/master/42.md) Authentication of clients to relays
- [NIP-65](https://github.com/nostr-protocol/nips/blob/master/65.md) Relay List Metadata
