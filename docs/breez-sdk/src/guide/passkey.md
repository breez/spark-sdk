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

To register your web origin, [contact us](mailto:contact@breez.technology?subject=Passkey%20configuration) to have it added to this file.

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

Replace `com.example.yourapp` with your application package name and the fingerprint with your app's signing certificate SHA256 fingerprint. See the <a target="_blank" href="https://developers.google.com/digital-asset-links/v1/getting-started">Digital Asset Links</a> documentation and <a target="_blank" href="https://developer.android.com/identity/credential-manager/prerequisites">Credential Manager prerequisites</a> for details.

To register your Android app, [contact us](mailto:contact@breez.technology?subject=Passkey%20configuration) with the details outlined to have it added to this file.

#### iOS: Apple App Site Association

**File**: `https://keys.breez.technology/.well-known/apple-app-site-association`

Connects the domain to iOS applications for passkey sharing:

```json
{
  "webcredentials": {
    "apps": [
      "TEAMID.com.example.yourapp"
    ]
  }
}
```

Replace `TEAMID` with your Apple Developer Team ID and `com.example.yourapp` with your application bundle identifier.

Your app must have the <a target="_blank" href="https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.associated-domains">Associated Domains</a> capability enabled. In Xcode, go to **Signing & Capabilities** → add **Associated Domains** → add the entry `webcredentials:keys.breez.technology`.

<div class="warning">
<h4>Expo Managed Workflow</h4>
If you're using Expo, the Breez SDK plugin can configure this automatically. See the <a href="install_react_native.html#plugin-options">React Native/Expo installation guide</a> for details on the <code>enablePasskey</code> option.
</div>

To register your iOS app, [contact us](mailto:contact@breez.technology?subject=Passkey%20configuration) with the details outlined to have it added to this file.

### Nostr relay configuration

The SDK uses Nostr relays to store and discover labels. Configure relay access by passing a {{#name NostrRelayConfig}} when constructing the {{#name Passkey}} instance:

- {{#name breez_api_key}} - Your Breez API key. When provided, the SDK connects to the Breez-managed relay with <a target="_blank" href="https://github.com/nostr-protocol/nips/blob/master/42.md">NIP-42</a> authentication.
- {{#name timeout_secs}} - Connection timeout in seconds (defaults to 30).

The SDK also implements <a target="_blank" href="https://github.com/nostr-protocol/nips/blob/master/65.md">NIP-65</a> to discover and publish to additional public relays for redundancy. See the [Listing labels](#listing-labels) and [Storing a label](#storing-a-label) code examples below for usage.

## Implementing the PRF provider

Your application must implement the PRF provider to interface with platform passkey APIs.

{{#tabs passkey:implement-prf-provider}}

### Platform considerations

- **Web (browsers)**: Use the WebAuthn API with the `prf` extension. Browsers handle the salt transformation internally. Use discoverable credentials (`residentKey: 'required'`) with empty `allowCredentials` for assertion so the browser discovers the credential by RP ID.

- **Android / iOS**: Use native passkey APIs with PRF support. Ensure the Associated Domains / Asset Links configuration is in place for `keys.breez.technology`.

- **CLI / Desktop (CTAP2)**: Use the `hmac-secret` extension directly. Non-browser implementations must apply the WebAuthn salt transformation manually to produce the same PRF output as browsers:

  ```
  actualSalt = SHA-256("WebAuthn PRF" || 0x00 || developerSalt)
  ```

  This transformation is defined in the <a target="_blank" href="https://w3c.github.io/webauthn/#prf-extension">W3C WebAuthn PRF extension spec</a> and ensures that the same passkey + salt produces identical seeds across browser and native implementations.

<h2 id="connecting-with-passkey">
    <a class="header" href="#connecting-with-passkey">Connecting with a passkey</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/passkey/struct.Passkey.html#method.get_wallet">API docs</a>
</h2>

To connect with a passkey, call {{#name Passkey.get_wallet}} to derive a wallet, then pass its seed to {{#name connect}}. The label defaults to `"Default"` when omitted.

{{#tabs passkey:connect-with-passkey}}

<h2 id="listing-labels">
    <a class="header" href="#listing-labels">Listing labels</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/passkey/struct.Passkey.html#method.list_labels">API docs</a>
</h2>

Discover labels associated to the passkey using Nostr.

{{#tabs passkey:list-labels}}

<h2 id="storing-a-label">
    <a class="header" href="#storing-a-label">Storing a label</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/passkey/struct.Passkey.html#method.store_label">API docs</a>
</h2>

Publish a label to Nostr so it can be discovered later.

{{#tabs passkey:store-label}}

## Best practices

### Cache the user-selected label

Store the label locally (e.g., `localStorage` on web, `SharedPreferences` on Android, `UserDefaults` on iOS) if selected by the user. This allows the app to skip the label selection step on subsequent launches and go straight to passkey authentication.

### Never store the derived mnemonic

The mnemonic should always be re-derived from the passkey and label on each session. The passkey authentication (biometric, PIN, etc.) is the security boundary — storing the mnemonic would bypass it. On app restart, check for a cached label and prompt the user for passkey authentication to derive the seed.

### Allow manual mnemonic backup

Provide a way for users to reveal their derived 12-word mnemonic as an emergency backup. This should be user-initiated (e.g., behind a "Show recovery phrase" button) and derived on-demand via {{#name Passkey.get_wallet}} with the cached label. This gives users a safety net if they lose access to their passkey.

### Offer a mnemonic fallback

Not all devices support the PRF extension. Check {{#name Passkey.is_available}} at startup and present the appropriate flow — seedless for capable devices, traditional mnemonic backup/restore for others.

### Handle label discovery failures

When discovering labels, {{#name Passkey.list_labels}} may return an empty list if relays are unreachable or the label events have been pruned. Always allow manual label entry as a fallback alongside the Nostr-discovered list.

## Supported specs

- [Seedless Restore](https://github.com/breez/seedless-restore) Passkey-based wallet derivation and discovery
- [Nostr](https://github.com/nostr-protocol/nostr) Relay-based event protocol for label storage
- [NIP-42](https://github.com/nostr-protocol/nips/blob/master/42.md) Authentication of clients to relays
- [NIP-65](https://github.com/nostr-protocol/nips/blob/master/65.md) Relay List Metadata
