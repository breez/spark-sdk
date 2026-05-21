# Passkey Login

Passkey Login lets users access their wallet with biometrics (fingerprint, face scan, or device PIN) instead of writing down and safeguarding a seed phrase. The SDK uses the <a target="_blank" href="https://w3c.github.io/webauthn/#prf-extension">WebAuthn PRF extension</a> to deterministically derive wallet keys from a passkey. Keys are never stored; they're regenerated on demand each time the user authenticates. The protocol also supports multiple wallets, each derived from a different label, with labels discoverable via Nostr relays.

For the full technical specification, see the <a target="_blank" href="https://github.com/breez/passkey-login/blob/main/spec.md">Passkey Login spec</a>.

## Setup

Passkey Login uses a Relying Party (RP) to tie passkeys to your apps. Each platform you target (Web, Android, iOS / macOS) needs a configuration file declaring your app under the RP. Platform authenticators validate the RP association against these files before any WebAuthn ceremony.

### Hosting the configuration files

Two ways to set up the RP:

- **Shared with the Breez ecosystem (Breez-hosted).** A passkey registered in one Breez-registered app works in every other Breez-registered app on the same device, with no re-registration. [Contact us](mailto:contact@breez.technology?subject=Passkey%20configuration) to register your app, then pass `PasskeyProvider.BREEZ_RP_ID` as your `rpId`.
- **Scoped to your ecosystem (self-hosted).** A passkey registered against your RP works across the apps and web origins you list in your configuration files. You host the well-known files yourself on an HTTPS domain you control. Pass that domain as your `rpId` (for example, `"<your-rp-domain>"`).

Same code paths in either case; only the `rpId` value and who hosts the JSON differs.

### Web: Related Origins

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

**Firefox does not implement Related Origins.** Users on Firefox can only use credentials whose RP ID matches their current origin's eTLD+1. If you need Firefox support across multiple domains, host your own RP ID per domain or accept that Firefox users register fresh on each origin.

**Chrome and Edge cap the number of distinct labels** in `related_origins` (around 5 distinct eTLD+1 labels per RP). For larger app families, partition into multiple RP IDs.

**Browsers cache the `.well-known/webauthn` file aggressively.** Adding or removing an origin won't propagate immediately; expect a delay until the cache TTL expires.
</div>

### Android: Asset Links

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

**Requirements**: Android 9+ (API 28) with Google Play Services, or Android 14+ (API 34) with any compatible credential provider. `compileSdkVersion` must be at least 34 (required by the `androidx.credentials` library, not the device).

### iOS / macOS: Apple App Site Association

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

Replace `TEAMID` with your Apple Developer Team ID and `com.example.yourapp` with your bundle identifier. Your app must also declare the <a target="_blank" href="https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.associated-domains">Associated Domains</a> capability in Xcode (**Signing & Capabilities** → **Associated Domains** → `webcredentials:<your-rp-domain>`).

<div class="warning">
<h4>iOS / macOS: Associated Domains entitlement required</h4>
Without the Associated Domains entitlement declared in Xcode, passkey operations on iOS / macOS will fail with a configuration error even though {{#name check_availability}} returns {{#enum PasskeyAvailability::Available}} (the OS-level check can't verify entitlements at runtime).
</div>

<div class="warning">
<h4>iOS / macOS: Expo Managed Workflow</h4>
If you're using Expo, the Breez SDK plugin can configure the Associated Domains entitlement automatically. See the <a href="install_react_native.html#plugin-options">React Native/Expo installation guide</a> for details on the <code>enablePasskey</code> option.
</div>

**Requirements**: iOS 18.0+, macOS 15.0+.

## Configuring the PasskeyClient

{{#name PasskeyClient}} is the entry point for every passkey-derived wallet operation. It composes a {{#name PrfProvider}} (the platform-specific bridge to WebAuthn / Credential Manager / AuthenticationServices) with the SDK's internal Nostr-backed label store, then exposes register / sign-in / connect / labels / credentials to your app code. Construct one per app session and reuse it.

The recommended setup is the {{#name createPasskeyClient}} convenience factory, which builds the platform-default {{#name PasskeyProvider}} and forwards the Breez API key from your SDK {{#name Config}}:

{{#tabs passkey:setup-client}}

Parameters:

- **`rpId`**: Relying Party ID. Pass your app's domain, or `PasskeyProvider.BREEZ_RP_ID` (= `keys.breez.technology`) if your app is Breez-registered. Required because changing it later strands existing credentials.
- **`rpName`**: Display name shown to the user in the OS passkey picker and credential-management UIs (iCloud Keychain, Google Password Manager, 1Password, etc.) when choosing a credential. Required.
- **`sdkConfig`**: Your main SDK {{#name Config}}. The factory forwards `sdkConfig.api_key` to the Breez relay for authenticated (<a target="_blank" href="https://github.com/nostr-protocol/nips/blob/master/42.md">NIP-42</a>) label storage. Without an API key, label sync falls back to public relays only.
- **`passkeyConfig`** (optional): Carries {{#name default_label}}, the label used when {{#name PasskeyClient.register}} / {{#name PasskeyClient.sign_in}} receive no label. Falls back to the SDK's internal `"Default"` when unset.

The SDK also implements <a target="_blank" href="https://github.com/nostr-protocol/nips/blob/master/65.md">NIP-65</a> to discover and publish to additional public relays for redundancy.

`createPasskeyClient` is not surfaced on web; the WASM tab shows the equivalent direct construction. For custom PRF providers (CLI YubiKey, FIDO2, file-backed) or non-default platform options (`credentialRegistry`, `userName`, etc.), see [Advanced](#advanced).

## Checking passkey availability

Use {{#name PasskeyClient.check_availability}} to gate passkey UI elements. The single call collapses {{#name PrfProvider.is_supported}} and {{#name PrfProvider.check_domain_association}} into a tagged {{#name PasskeyAvailability}} value with four variants: `Available`, `PrfUnsupported`, `NotAssociated { source, reason }`, `Skipped { reason }`. Branch on the variant to fall back to mnemonic-based onboarding on unsupported platforms (Android < 9, iOS < 18) or surface configuration mistakes (missing entitlement, AASA not deployed) without firing a WebAuthn ceremony:

{{#tabs passkey:check-availability}}

<h2 id="onboarding">
    <a class="header" href="#onboarding">Onboarding</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/passkey/struct.PasskeyClient.html#method.connect_with_passkey">API docs</a>
</h2>

The recommended flow depends on the platform. Mobile uses a single-call unified flow backed by {{#name PasskeyClient.connect_with_passkey}}. Web uses two buttons (Sign In and Create Account) and lets the user pick. For explicit control over each path, call {{#name PasskeyClient.sign_in}} and {{#name PasskeyClient.register}} directly.

### Unified flow (mobile)

One "Use Passkey" button backed by {{#name PasskeyClient.connect_with_passkey}}: silent sign-in for a returning user, automatic fall-through to registration on a fresh device. The response's `registered_credential` field doubles as the path discriminator: `Some` (with the new credential metadata) on the register path, `None` on the sign-in path.

Internally the silent attempt pins `preferImmediatelyAvailableCredentials = true` so the OS fast-fails (no UI, sub-300ms on iOS / Android) when no local credential exists; only {{#enum PrfProviderError::CredentialNotFound}} flips to register, all other errors (`Cancel`, `Timeout`, `Configuration`) propagate unchanged.

{{#tabs passkey:connect-with-passkey}}

### Two-button flow (web)

`connect_with_passkey` is not surfaced on the WASM target. The unified flow needs a silent "no credential here" signal so it can fall through to register, and WebAuthn deliberately collapses that case into the same `NotAllowedError` as a user cancel for privacy reasons. There is no reliable way on web for the SDK to tell the two apart.

The recommended UX on web is two buttons: a **Sign In** button calling `signIn` and a separate **Create Account** button calling `register`. Let the user pick the right one instead of trying to auto-detect. See [Direct sign-in / register](#direct-sign-in-register) for the call shapes.

<h3 id="direct-sign-in-register">Direct sign-in / register</h3>

For finer control, call {{#name PasskeyClient.sign_in}} and {{#name PasskeyClient.register}} directly. Use this path when separating Sign-In and Create-Account flows, or when adding a new label on a returning user without going through `connect_with_passkey`.

Sign in to an existing credential:

{{#tabs passkey:sign-in}}

Register a fresh credential:

{{#tabs passkey:register-passkey}}

Pass `wallet.seed` to {{#name connect}} in either case.

## Managing labels

Most apps brand a single label and never call these directly. Listing and publishing labels matters when your app supports multiple wallets per passkey identity.

<h3 id="listing-labels">
    <a class="header" href="#listing-labels">Listing</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/passkey/struct.PasskeyLabels.html#method.list">API docs</a>
</h3>

Discover labels associated to the passkey using Nostr. {{#name PasskeyClient.sign_in}} already lists labels in discovery mode (when no `label` is specified), so a separate {{#name PasskeyLabels.list}} call (via {{#name PasskeyClient.labels}}) is only needed when re-fetching the label set after sign-in.

{{#tabs passkey:list-labels}}

<h3 id="storing-a-label">
    <a class="header" href="#storing-a-label">Storing</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/passkey/struct.PasskeyLabels.html#method.store">API docs</a>
</h3>

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

Two recovery paths are common enough to warrant runnable examples.

Flip to sign-in when register hits an existing credential (`AlreadyExists`):

{{#tabs passkey:recover-already-exists}}

Show a sticky retry UI when the OS biometric timeout fires (`Timeout`):

{{#tabs passkey:handle-timeout}}

For the full mapping (including the iOS sub-300ms fast-fail nuance that bundles "no-credential" under `UserCancelled`), see the [UX guide](./uxguide_passkey.md).

<a id="advanced"></a>

## Advanced

Customization points most apps never need. {{#name createPasskeyClient}} handles the typical onboarding flow on every platform with a built-in `PasskeyProvider`. Reach for the topics here when:

- You need a custom `PrfProvider` (CLI YubiKey, FIDO2, air-gapped backup file).
- You're integrating Python, Go, or C# (no built-in `PasskeyProvider` ships for those bindings).
- You want fine-grained `PasskeyProvider` options (`credentialRegistry`, custom `userName`, etc.).
- You need the lower-level domain-association diagnostic separate from {{#name PasskeyClient.check_availability}}.

### Built-in PasskeyProvider options

The convenience factory only takes `rpId` and `rpName`. The underlying `PasskeyProvider` constructor accepts a few more knobs:

| Option | Default | Description |
|--------|---------|-------------|
| {{#name rp_id}} | **required** | Relying Party ID. Pass your app's domain, or `PasskeyProvider.BREEZ_RP_ID` (= `keys.breez.technology`) if your app is Breez-registered. Changing this means existing passkeys produce a different seed; see [migration considerations](https://github.com/breez/passkey-login/blob/main/SDK%20implementation.md#passkey-migration-considerations). |
| {{#name rp_name}} | **required** | Display name shown to the user in the OS passkey picker and credential-management UIs when choosing a credential. Only used at credential registration; changing it does not affect existing credentials. |
| {{#name user_name}} | {{#name rp_name}} | User name stored with the credential (registration only). On iOS this is the only field surfaced in the iCloud Keychain / passkey picker: the platform's `ASAuthorizationPlatformPublicKeyCredentialProvider` API has no `displayName` slot. Pass the per-credential friendly label here if you want users to see it. |
| {{#name user_display_name}} | {{#name user_name}} | Primary label shown in passkey picker on platforms that surface a separate display name (Android via WebAuthn JSON `user.displayName`, web via WebAuthn L3). iOS ignores. |
| {{#name credential_registry}} | none | Opt-in app-side store of known credential IDs. See [CredentialRegistry](#credentialregistry). |

<div class="warning">
<h4>C# / Go / Python limitation</h4>

The SDK does not ship a built-in `PasskeyProvider` for C#, Go, or Python (no native passkey API to wrap on those targets). Integrators on those bindings implement their own `PrfProvider` and pass it to `PasskeyClient::new(...)` directly. See [Custom PrfProvider](#custom-prfprovider).
</div>

### Built-in behaviours

The native built-in providers (iOS / Android / Flutter / RN) handle several platform quirks internally so consumers don't need workarounds:

- **Bulk PRF (single OS prompt for N salts).** {{#name derive_seeds}} uses the WebAuthn dual-salt extension (`saltInput1` + `saltInput2` on iOS, `prfFirst` + `prfSecond` on Android) when the authenticator supports it, falling back to per-salt assertions otherwise. The SDK's {{#name PasskeyClient.sign_in}} and {{#name PasskeyClient.register}} both go through this path so a master + label derivation costs **one** prompt where supported.
- **Post-create grace (800ms).** After a successful {{#name create_passkey}}, the next derive call holds briefly so the OS finishes indexing the new credential before the immediate post-register assertion. Without this, on Apple Passwords the dual-salt assertion can drop `prf.second` and force a fallback prompt; on Google Password Manager the credential can be briefly invisible to the picker.
- **Fast-fail on no-credential.** Assertions set `preferImmediatelyAvailableCredentials` (iOS) / `preferImmediatelyAvailableCredentials=true` (Android) so a missing credential surfaces as `CredentialNotFound` immediately rather than the cross-device "use another device" hybrid sheet.
- **Opt-in CredentialRegistry auto-merge.** Supplying a `CredentialRegistry` unions its IDs into `allowCredentialIds` before assertion and into `excludeCredentialIds` before registration, and auto-adds the asserted / created credential ID back. See [CredentialRegistry](#credentialregistry) for reference impls.

These run by default. To override (e.g. for a custom credential-management UI), pass an explicit `allowCredentialIds`.

<a id="custom-prfprovider"></a>

### Custom PrfProvider

If the built-in `PasskeyProvider` does not satisfy your requirements (e.g., you need a hardware security key, a FIDO2/CTAP2 transport, an air-gapped backup file, or a custom authenticator), implement the `PrfProvider` interface directly. The Breez CLI ships [YubiKey](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/yubikey_prf.rs), [FIDO2](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/fido2_prf.rs), and [file-based](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/file_prf.rs) implementations as references.

{{#tabs passkey:implement-prf-provider}}

<a id="credentialregistry"></a>

### CredentialRegistry

The SDK ships only the contract (a `CredentialRegistry` interface on each platform). Bring your own implementation: Keychain on iOS, Block Store + SharedPreferences on Android, `localStorage` on web, or a custom backend. Registry calls are best-effort with a 3 second timeout: failures fire `onRegistryError` (when set) and the WebAuthn ceremony proceeds. Pre-tracking credentials get seeded on first assertion so subsequent registrations correctly hit the platform-level "already exists" guard via `excludeCredentialIds`.

#### iOS Keychain (with iCloud Keychain sync)

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
    rpId: "<your-rp-domain>",
    rpName: "Your App",
    credentialRegistry: KeychainCredentialRegistry()
)
```

#### Android Block Store + SharedPreferences fallback

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
    rpId = "<your-rp-domain>",
    rpName = "Your App",
    credentialRegistry = BlockStoreCredentialRegistry(activity.applicationContext),
)
```

#### Web localStorage

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
    rpId: '<your-rp-domain>',
    rpName: 'Your App',
    credentialRegistry: new LocalStorageCredentialRegistry(),
});
```

#### Using credentials() with a registry

Once a registry is wired in, the SDK auto-merges its contents into `excludeCredentialIds` on register and `allowCredentialIds` on sign-in. You can also inspect and mutate the stored set directly via the {{#name PasskeyClient.credentials}} sub-object, typically to back a settings page that lists registered passkeys on this device with per-row remove.

{{#tabs passkey:with-credential-registry}}

#### When (not) to call `credentials().clear()`

`clear()` wipes the app's local bookkeeping only. Existing credentials stay on the OS / cloud authenticator and the user can still sign in with them.

**Don't call `clear()` on a normal sign-out.** With the registry empty, the next `register()` sends an empty `excludeCredentialIds`. On some authenticators that mints a **sibling credential**: same RP, different credential ID, **different PRF output → different seed → different wallet**. If the user picks the sibling at next sign-in, they reach an unfamiliar wallet.

Dedup behavior on an empty `excludeCredentialIds`:

| Authenticator | Behavior |
|---|---|
| iOS / iCloud Keychain | Mints sibling silently |
| Hardware keys (YubiKey etc.) | Mints sibling silently |
| Google Password Manager | Usually warns or dedupes |

Reserve `clear()` for explicit factory-reset flows where orphan credentials are acceptable.

#### CredentialRegistry error semantics

- Every registry call is bounded by a 3 second timeout. Slow backends never block the WebAuthn ceremony.
- Failures and timeouts are logged (Rust `tracing::warn`, Swift `os_log`, Kotlin `Log.w`, JS `console.warn`) and surfaced via the per-provider `onRegistryError` callback when set. `read` defaults to an empty list; `add`/`remove`/`clear` fire-and-forget.
- Web/RN/Flutter perform a constructor-time conformance check. A registry object missing one of `read` / `add` / `remove` / `clear` throws on construction so misconfiguration surfaces at startup. iOS / Kotlin / Rust enforce conformance at compile time.
- When a `CredentialNotFound` would propagate AND no `allowCredentialIds` AND no `credentialRegistry` were supplied, the SDK appends a help URL to the error message pointing here.

### Domain association diagnostic

{{#name check_domain_association}} performs an Apple-app-site-association probe (iOS) or Digital Asset Links lookup (Android) against the configured RP and returns a typed {{#name DomainAssociation}}: `Associated`, `NotAssociated { source, reason }`, or `Skipped { reason }`. Use it to surface configuration mistakes (entitlement missing, AASA file not deployed) before a WebAuthn ceremony fails opaquely. Most hosts reach this through {{#name PasskeyClient.check_availability}}, which folds it together with `is_supported` into one call; use the diagnostic directly when you need only the association signal:

{{#tabs passkey:domain-association}}

### Capacitor plugins

For Capacitor apps, the JS layer talks to the iOS Swift / Android Kotlin native plugin through Capacitor's bridge. The SDK ships a TypeScript-only contract sub-export so plugin authors can keep their `definitions.ts` in lockstep with the canonical native shape:

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
