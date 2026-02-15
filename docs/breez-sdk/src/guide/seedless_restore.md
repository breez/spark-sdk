# Seedless restore

Seedless restore enables wallet recovery using passkeys with the WebAuthn PRF extension, eliminating the need to backup mnemonic phrases. Wallet seeds are derived deterministically from a passkey and a user-chosen salt, with salts stored on Nostr relays for discovery during restore.

## Overview

The seedless restore flow uses two key derivations:

1. **Nostr Identity**: `PRF(passkey, magic_salt)` derives a Nostr keypair used to publish and discover salts
2. **Wallet Seed**: `PRF(passkey, user_salt)` derives a 24-word BIP39 mnemonic

Salts are published as Nostr kind-1 events, allowing users to discover their wallets on any device with access to their passkey.

```
┌─────────────┐     ┌──────────────────┐     ┌───────────────────┐     ┌───────────────┐
│ Application │────▶│ PRF Provider     │────▶│ SDK               │────▶│ Nostr Relays  │
│             │     │ (platform-       │     │ SeedlessRestore   │     │ (salt storage │
│             │     │  specific)       │     │                   │     │  & discovery) │
└─────────────┘     └──────────────────┘     └───────────────────┘     └───────────────┘
```

<div class="warning">
<h4>Developer note</h4>
The passkey PRF functionality must be implemented by your application using platform-specific APIs (WebAuthn in browsers, native passkey APIs on mobile). The SDK orchestrates the flow but requires you to provide a PRF provider implementation.
</div>

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

To register your web origin, contact Breez to have it added to this file.

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

Contact Breez to register your Android app.

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

Contact Breez to register your iOS app.

### Nostr relay configuration

The SDK uses Nostr relays to store and discover salts. A Breez API key is required for authenticated access to the Breez-managed relay via <a target="_blank" href="https://github.com/nostr-protocol/nips/blob/master/42.md">NIP-42</a>. The SDK also implements <a target="_blank" href="https://github.com/nostr-protocol/nips/blob/master/65.md">NIP-65</a> to discover and publish to additional public relays for redundancy.

## Implementing the PRF provider

Your application must implement the PRF provider to interface with platform passkey APIs.

{{#tabs seedless_restore:implement-prf-provider}}

### Platform considerations

- **Web (browsers)**: Use the WebAuthn API with the `prf` extension. Browsers handle the salt transformation internally. Use discoverable credentials (`residentKey: 'required'`) with empty `allowCredentials` for assertion so the browser discovers the credential by RP ID.

- **Android / iOS**: Use native passkey APIs with PRF support. Ensure the Associated Domains / Asset Links configuration is in place for `keys.breez.technology`.

- **CLI / Desktop (CTAP2)**: Use the `hmac-secret` extension directly. Non-browser implementations must apply the WebAuthn salt transformation manually to produce the same PRF output as browsers:

  ```
  actualSalt = SHA-256("WebAuthn PRF" || 0x00 || developerSalt)
  ```

  This transformation is defined in the <a target="_blank" href="https://w3c.github.io/webauthn/#prf-extension">W3C WebAuthn PRF extension spec</a> and ensures that the same passkey + salt produces identical seeds across browser and native implementations.

<h2 id="creating-a-seed">
    <a class="header" href="#creating-a-seed">Creating a seed</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/seedless_restore/struct.SeedlessRestore.html#method.create_seed">API docs</a>
</h2>

To create a new seedless wallet, provide a user-chosen salt (e.g., "personal", "business"). The salt is published to Nostr for later discovery.

{{#tabs seedless_restore:create-seed}}

<h2 id="listing-salts">
    <a class="header" href="#listing-salts">Listing available salts</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/seedless_restore/struct.SeedlessRestore.html#method.list_salts">API docs</a>
</h2>

To restore a wallet, first query Nostr for salts associated with the passkey's identity.

{{#tabs seedless_restore:list-salts}}

<h2 id="restoring-a-seed">
    <a class="header" href="#restoring-a-seed">Restoring a seed</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/seedless_restore/struct.SeedlessRestore.html#method.restore_seed">API docs</a>
</h2>

Once you have the salt, restore the wallet seed.

{{#tabs seedless_restore:restore-seed}}

## Best practices

### Cache the user-selected salt

Store the salt locally (e.g., `localStorage` on web, `SharedPreferences` on Android, `UserDefaults` on iOS) after a successful create or restore. This allows the app to skip the salt selection step on subsequent launches and go straight to passkey authentication.

### Never store the derived mnemonic

The mnemonic should always be re-derived from the passkey and salt on each session. The passkey authentication (biometric, PIN, etc.) is the security boundary — storing the mnemonic would bypass it. On app restart, check for a cached salt and prompt the user for passkey authentication to derive the seed.

### Allow manual mnemonic backup

Provide a way for users to reveal their derived 24-word mnemonic as an emergency backup. This should be user-initiated (e.g., behind a "Show recovery phrase" button) and derived on-demand via `restore_seed()` with the cached salt. This gives users a safety net if they lose access to their passkey.

### Offer a mnemonic fallback

Not all devices support the PRF extension. Check `is_prf_available()` at startup and present the appropriate flow — seedless for capable devices, traditional mnemonic backup/restore for others.

### Handle salt discovery failures

When restoring, `list_salts()` may return an empty list if relays are unreachable or the salt events have been pruned. Always allow manual salt entry as a fallback alongside the Nostr-discovered list.

## Security considerations

- **Passkey security**: The wallet's security depends on the passkey. Different passkeys produce different wallets.
- **Salt visibility**: Salts are published publicly on Nostr. Security comes from the passkey secret, not the salt.
- **PRF availability**: Check `is_prf_available()` to gracefully handle devices without PRF support.
- **Discoverable credentials**: Use resident keys so no credential IDs need to be stored by your application. The authenticator discovers the credential by RP ID.
- **User verification**: All passkey operations should require user verification (`userVerification: 'required'`) to ensure biometric or PIN confirmation.

## Passkey migration

Passkey portability across platforms depends on Credential Provider (CXP) protocol support:

- **iOS**: Supports CXP for exporting passkeys
- **Android**: CXP support expected in future versions
- **Third-party password managers**: Varying support for both CXP and PRF extensions. Neither iOS Password Manager nor Android's Google Password Manager currently support RP ID domain modification.

For users who need to move between ecosystems, the manual mnemonic backup serves as a universal fallback.
