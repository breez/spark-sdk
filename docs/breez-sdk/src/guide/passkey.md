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
Without the Associated Domains entitlement, passkey operations will fail with a configuration error even though {{#name check_availability}} returns {{#enum PasskeyAvailability::Available}} (the OS-level check can't verify entitlements at runtime).
</div>

### Configuring the PasskeyClient

`PasskeyClient` takes three arguments: the `PrfProvider`, an optional `breezApiKey`, and an optional {{#name PasskeyConfig}}.

- **`breezApiKey`**: Your Breez API key. When provided, the SDK connects to the Breez-managed relay with <a target="_blank" href="https://github.com/nostr-protocol/nips/blob/master/42.md">NIP-42</a> authentication for label storage and discovery. Pass `None` / `undefined` for public-relay-only label sync. Hosts that already pass the SDK's main `Config` to `connect()` can forward the same `api_key` here.

{{#name PasskeyConfig}} carries one field:

- {{#name default_label}}: the wallet label used when {{#name PasskeyClient.register}} / {{#name PasskeyClient.sign_in}} receive no label. Falls back to the SDK's internal `"Default"` when unset. Useful when your app brands a single wallet under a stable name.

The SDK also implements <a target="_blank" href="https://github.com/nostr-protocol/nips/blob/master/65.md">NIP-65</a> to discover and publish to additional public relays for redundancy. See the [Listing labels](#listing-labels) and [Storing a label](#storing-a-label) code examples below for usage.

## PRF providers

A PRF provider handles passkey credential registration, assertion, and PRF evaluation against the platform's authenticator. The SDK uses the seed it returns to deterministically derive wallet keys.

### Built-in PasskeyProvider

The SDK ships a built-in `PasskeyProvider` on every platform with native passkey support: Web (browsers), iOS, macOS, Android, Kotlin Multiplatform, React Native, and Flutter. Each one implements the underlying `PrfProvider` trait against the platform's WebAuthn / Credential Manager / AuthenticationServices APIs, so most apps can use it as-is without writing any glue code.

Built-in providers are not available for C#, Go, and Python.

**Constructor options** (all built-in providers):

| Option | Default | Description |
|--------|---------|-------------|
| {{#name rp_id}} | **required** | Relying Party ID. Pass your app's domain, or `PasskeyProvider.BREEZ_RP_ID` (= `keys.breez.technology`) if your app is Breez-registered. Changing this means existing passkeys produce a different seed; see [migration considerations](https://github.com/breez/passkey-login/blob/main/SDK%20implementation.md#passkey-migration-considerations). |
| {{#name rp_name}} | **required** | Display name shown to the user in the OS passkey picker and credential-management UIs (iCloud Keychain, Google Password Manager, 1Password, etc.) when choosing a credential. Only used at credential registration; changing it does not affect existing credentials. |
| {{#name user_name}} | {{#name rp_name}} | User name stored with the credential (registration only). On iOS this is the only field surfaced in the iCloud Keychain / passkey picker: the platform's `ASAuthorizationPlatformPublicKeyCredentialProvider` API has no `displayName` slot. Pass the per-credential friendly label here if you want users to see it. |
| {{#name user_display_name}} | {{#name user_name}} | Primary label shown in passkey picker on platforms that surface a separate display name (Android via WebAuthn JSON `user.displayName`, web via WebAuthn L3). iOS ignores. |
| {{#name credential_registry}} | none | Opt-in app-side store of known credential IDs. When supplied, the SDK auto-merges stored IDs into `allowCredentialIds` / `excludeCredentialIds` and writes new IDs back after success. See [Implementing CredentialRegistry](#credentialregistry) below. |

`rpId` is required: there is no shared default. Pass `PasskeyProvider.BREEZ_RP_ID` to opt into Breez's `keys.breez.technology` shared RP (only valid for Breez-registered apps); apps with their own RP domain pass their own string. Apps that share an RP ID recognize the same user's passkey without per-app enrollment.

Pass a `PasskeyProvider` instance to `PasskeyClient` as shown in the [Connecting with a passkey](#connecting-with-passkey) snippets below. That is the recommended path for almost every app.

### Built-in behaviours

The native built-in providers (iOS / Android / Flutter / RN) handle several platform quirks internally so consumers don't need workarounds:

- **Bulk PRF (single OS prompt for N salts).** {{#name derive_seeds}} uses the WebAuthn dual-salt extension (`saltInput1` + `saltInput2` on iOS, `prfFirst` + `prfSecond` on Android) when the authenticator supports it, falling back to per-salt assertions otherwise. The SDK's {{#name PasskeyClient.sign_in}} and {{#name PasskeyClient.register}} both go through this path so a master + label derivation costs **one** prompt where supported.
- **Post-create grace (800ms).** After a successful {{#name create_passkey}}, the next derive call holds briefly so the OS finishes indexing the new credential before the immediate post-register assertion. Without this, on Apple Passwords the dual-salt assertion can drop `prf.second` and force a fallback prompt; on Google Password Manager the credential can be briefly invisible to the picker.
- **Fast-fail on no-credential.** Assertions set `preferImmediatelyAvailableCredentials` (iOS) / `preferImmediatelyAvailableCredentials=true` (Android) so a missing credential surfaces as `CredentialNotFound` immediately rather than the cross-device "use another device" hybrid sheet.
- **Opt-in CredentialRegistry auto-merge.** Hosts that supply a `CredentialRegistry` get registry IDs unioned into `allowCredentialIds` before assertion and into `excludeCredentialIds` before registration, plus the asserted/created credential ID is auto-added back. See [Implementing CredentialRegistry](#credentialregistry) below for reference impls.

These run by default. Hosts that need to override (e.g. for a custom credential-management UI) can pass an explicit `allowCredentialIds`.

<a id="credentialregistry"></a>

## Implementing CredentialRegistry

The SDK ships only the contract (a `CredentialRegistry` interface on each platform). Bring your own implementation: Keychain on iOS, Block Store + SharedPreferences on Android, `localStorage` on web, or a custom backend. Registry calls are best-effort with a 3 second timeout: failures fire `onRegistryError` (when set) and the WebAuthn ceremony proceeds. Pre-tracking credentials get seeded on first assertion so subsequent registrations correctly hit the platform-level "already exists" guard via `excludeCredentialIds`.

### iOS Keychain (with iCloud Keychain sync)

```swift
import Foundation
import Security
import BreezSdkSpark

/// iCloud-synced keychain registry. One generic-password item per RP.
/// `kSecAttrSynchronizable=true` opts the item into iCloud Keychain
/// sync so it survives reinstall and replicates across the user's
/// signed-in devices. No extra dependencies.
public struct KeychainCredentialRegistry: CredentialRegistry {
    private let service = "com.example.app.passkey.knownCredentials"

    public init() {}

    public func read(rpId: String) async throws -> [Data] {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: rpId,
            kSecAttrSynchronizable as String: kSecAttrSynchronizableAny,
            kSecMatchLimit as String: kSecMatchLimitOne,
            kSecReturnData as String: true,
        ]
        var result: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        guard status == errSecSuccess, let data = result as? Data else { return [] }
        guard let array = try? JSONSerialization.jsonObject(with: data) as? [String] else { return [] }
        return array.compactMap { Data(base64Encoded: $0) }
    }

    public func add(rpId: String, credentialId: Data) async throws {
        var ids = (try? await read(rpId: rpId)) ?? []
        if !ids.contains(credentialId) { ids.append(credentialId) }
        try write(rpId: rpId, ids: ids)
    }

    public func remove(rpId: String, credentialId: Data) async throws {
        let ids = (try? await read(rpId: rpId)) ?? []
        try write(rpId: rpId, ids: ids.filter { $0 != credentialId })
    }

    public func clear(rpId: String) async throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: rpId,
            kSecAttrSynchronizable as String: kSecAttrSynchronizableAny,
        ]
        SecItemDelete(query as CFDictionary)
    }

    private func write(rpId: String, ids: [Data]) throws {
        let blob = try JSONSerialization.data(
            withJSONObject: ids.map { $0.base64EncodedString() }
        )
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: rpId,
            kSecAttrSynchronizable as String: kSecAttrSynchronizableAny,
        ]
        let attrs: [String: Any] = [
            kSecValueData as String: blob,
            kSecAttrSynchronizable as String: true,
        ]
        let status = SecItemUpdate(query as CFDictionary, attrs as CFDictionary)
        if status == errSecItemNotFound {
            var add = query
            add[kSecValueData as String] = blob
            add[kSecAttrSynchronizable as String] = true
            SecItemAdd(add as CFDictionary, nil)
        }
    }
}
```

Pass to the provider:

```swift
let provider = PasskeyProvider(
    rpId: "my-app.com",
    rpName: "My App",
    credentialRegistry: KeychainCredentialRegistry()
)
```

### Android Block Store + SharedPreferences fallback

Add the dependencies your registry needs:

```gradle
implementation "com.google.android.gms:play-services-auth-blockstore:16.4.0"
implementation "org.jetbrains.kotlinx:kotlinx-coroutines-play-services:1.8.0"
```

Plain `SharedPreferences` is the local fallback (credential IDs are public per WebAuthn spec, so no encryption is required).

```kotlin
import android.content.Context
import com.google.android.gms.auth.blockstore.Blockstore
import com.google.android.gms.auth.blockstore.DeleteBytesRequest
import com.google.android.gms.auth.blockstore.RetrieveBytesRequest
import com.google.android.gms.auth.blockstore.StoreBytesData
import kotlinx.coroutines.tasks.await
import org.json.JSONArray
import technology.breez.spark.passkey.core.CredentialRegistry
import android.util.Base64

/**
 * Block Store (Google-account-synced) primary, SharedPreferences
 * local fallback. Reads union both sources, writes go to both.
 * Block Store survives reinstall on the same Google account; the
 * SharedPreferences fallback covers offline / not-signed-in cases.
 */
class BlockStoreCredentialRegistry(private val context: Context) : CredentialRegistry {
    private val client = Blockstore.getClient(context)
    private val prefs = context.getSharedPreferences("passkey.knownCredentials", Context.MODE_PRIVATE)
    private fun bsKey(rpId: String) = "passkey.knownCredentials.$rpId"
    private fun encode(b: ByteArray) =
        Base64.encodeToString(b, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)
    private fun decode(s: String) =
        Base64.decode(s, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)

    override suspend fun read(rpId: String): List<ByteArray> {
        val seen = LinkedHashSet<String>()
        runCatching {
            val req = RetrieveBytesRequest.Builder().setKeys(listOf(bsKey(rpId))).build()
            val map = client.retrieveBytes(req).await().blockstoreDataMap
            map[bsKey(rpId)]?.bytes?.toString(Charsets.UTF_8)?.let {
                JSONArray(it).let { arr ->
                    for (i in 0 until arr.length()) seen.add(arr.getString(i))
                }
            }
        }
        prefs.getStringSet(rpId, emptySet())?.forEach { seen.add(it) }
        return seen.map(::decode)
    }

    override suspend fun add(rpId: String, credentialId: ByteArray) {
        val current = LinkedHashSet(read(rpId).map(::encode))
        if (!current.add(encode(credentialId))) return
        write(rpId, current.toList())
    }

    override suspend fun remove(rpId: String, credentialId: ByteArray) {
        val updated = read(rpId).map(::encode).toMutableList().apply {
            remove(encode(credentialId))
        }
        write(rpId, updated)
    }

    override suspend fun clear(rpId: String) {
        runCatching {
            client.deleteBytes(
                DeleteBytesRequest.Builder().setKeys(listOf(bsKey(rpId))).build()
            ).await()
        }
        prefs.edit().remove(rpId).apply()
    }

    private suspend fun write(rpId: String, encoded: List<String>) {
        val payload = JSONArray(encoded).toString().toByteArray(Charsets.UTF_8)
        runCatching {
            client.storeBytes(
                StoreBytesData.Builder()
                    .setKey(bsKey(rpId))
                    .setBytes(payload)
                    .setShouldBackupToCloud(true)
                    .build()
            ).await()
        }
        prefs.edit().putStringSet(rpId, encoded.toSet()).apply()
    }
}
```

Pass to the provider:

```kotlin
val provider = PasskeyProvider(
    activityProvider = { activity },
    rpId = "my-app.com",
    rpName = "My App",
    credentialRegistry = BlockStoreCredentialRegistry(activity.applicationContext),
)
```

### Web localStorage

```typescript
import type { CredentialRegistry } from '@breeztech/breez-sdk-spark/passkey-prf-provider';

/**
 * Default registry backed by `window.localStorage`. JSON-encoded
 * array of base64url credential IDs per RP. Cleared by the browser
 * on site-data clear; not synced across devices.
 */
export class LocalStorageCredentialRegistry implements CredentialRegistry {
    constructor(private prefix = 'breez.spark.passkey.knownCredentials.') {}

    private encode(b: Uint8Array) {
        return btoa(String.fromCharCode(...b)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
    }
    private decode(s: string) {
        const pad = s.length % 4 === 0 ? 0 : 4 - (s.length % 4);
        const bin = atob(s.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat(pad));
        const out = new Uint8Array(bin.length);
        for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
        return out;
    }
    private list(rpId: string): string[] {
        try {
            return JSON.parse(localStorage.getItem(this.prefix + rpId) ?? '[]') ?? [];
        } catch { return []; }
    }
    private save(rpId: string, ids: string[]) {
        if (ids.length === 0) localStorage.removeItem(this.prefix + rpId);
        else localStorage.setItem(this.prefix + rpId, JSON.stringify(ids));
    }

    async read(rpId: string) { return this.list(rpId).map((s) => this.decode(s)); }
    async add(rpId: string, credentialId: Uint8Array) {
        const enc = this.encode(credentialId);
        const ids = this.list(rpId);
        if (!ids.includes(enc)) { ids.push(enc); this.save(rpId, ids); }
    }
    async remove(rpId: string, credentialId: Uint8Array) {
        const enc = this.encode(credentialId);
        this.save(rpId, this.list(rpId).filter((s) => s !== enc));
    }
    async clear(rpId: string) { this.save(rpId, []); }
}
```

Pass to the provider:

```typescript
const provider = new PasskeyProvider({
    rpId: 'my-app.com',
    rpName: 'My App',
    credentialRegistry: new LocalStorageCredentialRegistry(),
});
```

### CredentialRegistry error semantics

- Every registry call is bounded by a 3 second timeout. Slow backends never block the WebAuthn ceremony.
- Failures and timeouts are logged (Rust `tracing::warn`, Swift `os_log`, Kotlin `Log.w`, JS `console.warn`) and surfaced via the per-provider `onRegistryError` callback when set. `read` defaults to an empty list; `add`/`remove`/`clear` fire-and-forget.
- Web/RN/Flutter perform a constructor-time conformance check. A registry object missing one of `read` / `add` / `remove` / `clear` throws on construction so misconfiguration surfaces at startup. iOS / Kotlin / Rust enforce conformance at compile time.
- When a `CredentialNotFound` would propagate to the host AND no `allowCredentialIds` AND no `credentialRegistry` were supplied, the SDK appends a help URL to the error message pointing here.

## Domain association diagnostic

{{#name check_domain_association}} performs an Apple-app-site-association probe (iOS) or Digital Asset Links lookup (Android) against the configured RP and returns a typed {{#name DomainAssociation}}: `Associated`, `NotAssociated { source, reason }`, or `Skipped { reason }`. Use it to surface configuration mistakes (entitlement missing, AASA file not deployed) before a WebAuthn ceremony fails opaquely. The SDK never blocks on this check; treat `Skipped` as advisory.

{{#tabs passkey:domain-association}}

## Checking passkey availability

Use {{#name PasskeyClient.check_availability}} to gate passkey UI elements. The single call collapses {{#name PrfProvider.is_supported}} and {{#name PrfProvider.check_domain_association}} into a tagged {{#name PasskeyAvailability}} value with four variants: `Available`, `PrfUnsupported`, `NotAssociated { source, reason }`, `Skipped { reason }`. Hosts can fall back to mnemonic-based onboarding on unsupported platforms (Android < 9, iOS < 18) and surface configuration mistakes (missing entitlement, AASA not deployed) without firing a WebAuthn ceremony:

{{#tabs passkey:check-availability}}

<h2 id="connecting-with-passkey">
    <a class="header" href="#connecting-with-passkey">Connecting with a passkey</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/passkey/struct.PasskeyClient.html#method.connect_with_passkey">API docs</a>
</h2>

The recommended onboarding flow on mobile is a single CTA backed by {{#name PasskeyClient.connect_with_passkey}}: silent sign-in for a returning user, automatic fall-through to registration on a fresh device. The response's `registered_credential` field doubles as the path discriminator: `Some` (with the new credential metadata) on the register path, `None` on the sign-in path.

Internally the silent attempt pins `preferImmediatelyAvailableCredentials = true` so the OS fast-fails (no UI, sub-300ms on iOS / Android) when no local credential exists; only {{#enum PrfProviderError::CredentialNotFound}} flips to register, all other errors (`Cancel`, `Timeout`, `Configuration`) propagate unchanged.

{{#tabs passkey:connect-with-passkey}}

For finer control, call {{#name PasskeyClient.sign_in}} and {{#name PasskeyClient.register}} directly. {{#name PasskeyClient.register}} alone is the right entry point for a deliberate "create a new wallet" UI (e.g. adding a new label to an existing identity). Pass `wallet.seed` to {{#name connect}} in both cases.

### Web: manual catch-and-register

`connect_with_passkey` is not surfaced on the WASM target: it depends on `preferImmediatelyAvailableCredentials` for a silent fast-fail, and the web equivalent (`mediation: 'immediate'` / `uiMode: 'immediate'`) is not yet stable cross-browser. On web, implement the equivalent flow manually by calling `signIn` and catching the credential-not-found error:

{{#tabs passkey:signin-fallback-register}}

<h2 id="listing-labels">
    <a class="header" href="#listing-labels">Listing labels</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/passkey/struct.PasskeyLabels.html#method.list">API docs</a>
</h2>

Discover labels associated to the passkey using Nostr. {{#name PasskeyClient.sign_in}} already lists labels in discovery mode (when no `label` is specified), so a separate {{#name PasskeyLabels.list}} call (via {{#name PasskeyClient.labels}}) is only needed when re-fetching the label set after sign-in.

{{#tabs passkey:list-labels}}

<h2 id="storing-a-label">
    <a class="header" href="#storing-a-label">Storing a label</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/passkey/struct.PasskeyLabels.html#method.store">API docs</a>
</h2>

Publish a label to Nostr so it can be discovered later. {{#name PasskeyClient.register}} publishes the label automatically on registration; use {{#name PasskeyLabels.store}} (via {{#name PasskeyClient.labels}}) only when adding a new label to an existing identity (e.g. a "create a new wallet" path on a returning user).

{{#tabs passkey:store-label}}

## Error recovery

The SDK collapses every passkey failure into seven actionable [`ErrorKind`](https://breez.github.io/spark-sdk/breez_sdk_spark/passkey/enum.ErrorKind.html) values: branching on `error.kind()` is the canonical recovery pattern:

| `ErrorKind` | What it means | Recommended action |
|---|---|---|
| `Cancel` | User dismissed the OS prompt (≥ 300ms, < 55s) | Sticky retry UI with "Try Again" |
| `NoCredential` | No matching credential on this device (includes iOS sub-300ms fast-fail) | Fall through to {{#name PasskeyClient.register}} |
| `AlreadyExists` | Register hit a credential in `excludeCredentialIds` | Flip to {{#name PasskeyClient.sign_in}}; the OS picker surfaces the existing credential |
| `Timeout` | OS biometric inactivity timeout (≥ 55s): distinct from a user-cancel | Sticky retry with timeout-specific copy. **Do not** auto-retry |
| `PrfUnsupported` | Authenticator doesn't implement the PRF extension | Fall back to mnemonic onboarding |
| `Configuration` | Entitlement missing, AASA stale, or assetlinks malformed | Developer-facing error; surface {{#name check_domain_association}}'s `NotAssociated` reason |
| `Internal` | Network / library / generic failure | Generic "try again later" UI |

Web also exposes typed exception classes (`PasskeyAlreadyExistsError`, `PasskeyTimedOutError`, `PasskeyCredentialNotFoundError`) as an alternative to `error.kind()` for `instanceof` matching.

Two recovery paths are common enough to warrant runnable examples: `AlreadyExists` (flip to sign-in) and `Timeout` (sticky retry):

{{#tabs passkey:recover-already-exists}}

{{#tabs passkey:handle-timeout}}

For the full mapping (including the iOS sub-300ms fast-fail nuance that bundles "no-credential" under `UserCancelled`), see the [UX guide](./uxguide_passkey.md).

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
