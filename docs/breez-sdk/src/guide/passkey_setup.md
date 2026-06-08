# Setup

Passkey Login uses a Relying Party (RP) to tie passkeys to your apps. Each platform you target (Web, Android, iOS / macOS) needs a configuration file declaring your app under the RP. Platform authenticators validate the RP association against these files before any WebAuthn ceremony.

## Hosting the configuration files

Two ways to set up the RP:

- **Shared with the Breez ecosystem (Breez-hosted).** A passkey registered in one Breez-registered app works in every other Breez-registered app on the same device, with no re-registration. [Contact us](mailto:contact@breez.technology?subject=Passkey%20configuration) to register your app, then pass `PasskeyProvider.BREEZ_RP_ID` as your `rpId`.
- **Scoped to your ecosystem (self-hosted).** A passkey registered against your RP works across the apps and web origins you list in your configuration files. You host the well-known files yourself on an HTTPS domain you control. Pass that domain as your `rpId` (for example, `"<your-rp-domain>"`).

Same code paths in either case; only the `rpId` value and who hosts the JSON differs.

## Web: Related Origins

**Path**: `/.well-known/webauthn`

```json
{
  "related_origins": [
    "https://keys.breez.technology",
    "https://your-app.example.com"
  ]
}
```

**Requirements**: Chrome 116+, Safari 18+, Edge 116+. HTTPS required (localhost exempt during development).

<div class="warning">
<h4>Related Origins: developer notes</h4>

**Firefox does not implement Related Origins.** Its users register fresh on each origin. For multi-domain support, host a separate RP ID per domain.

**Chrome and Edge cap the number of distinct origins** in `related_origins` (around 5 per RP). For larger app families, partition into multiple RP IDs.

**Browsers cache `.well-known/webauthn` aggressively.** Adding or removing an origin takes effect only after the cache TTL expires.
</div>

## Android: Asset Links

**Path**: `/.well-known/assetlinks.json`

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

Replace `com.example.yourapp` with your application's package name and the fingerprint with your app's signing certificate SHA256. See the <a target="_blank" href="https://developers.google.com/digital-asset-links/v1/getting-started">Digital Asset Links</a> documentation and <a target="_blank" href="https://developer.android.com/identity/credential-manager/prerequisites">Credential Manager prerequisites</a>.

**Requirements**: Android 9+ (API 28) with Google Play Services, or Android 14+ (API 34) with any compatible authenticator. `compileSdkVersion` must be at least 34 (required by the `androidx.credentials` library, not the device).

## iOS / macOS: Apple App Site Association

**Path**: `/.well-known/apple-app-site-association`

```json
{
  "webcredentials": {
    "apps": [
      "TEAMID.com.example.yourapp"
    ]
  }
}
```

Replace `TEAMID` with your Apple Developer Team ID and `com.example.yourapp` with your bundle identifier. Your app must also declare the <a target="_blank" href="https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.associated-domains">Associated Domains</a> capability in Xcode (**Signing & Capabilities**, then **Associated Domains**, then add `webcredentials:<your-rp-domain>`).

<div class="warning">
<h4>iOS / macOS: Associated Domains entitlement required</h4>
Without the Associated Domains entitlement declared in Xcode, passkey operations on iOS / macOS fail with a configuration error, even when {{#name PasskeyClient.check_availability}} returns {{#enum PasskeyAvailability::Available}}.
</div>

<div class="warning">
<h4>iOS / macOS: Expo Managed Workflow</h4>
If you're using Expo, the Breez SDK plugin can configure the Associated Domains entitlement automatically. See the <a href="install_react_native.html#plugin-options">React Native/Expo installation guide</a> for details on the <code>enablePasskey</code> option.
</div>

**Requirements**: iOS 18.0+, macOS 15.0+.
