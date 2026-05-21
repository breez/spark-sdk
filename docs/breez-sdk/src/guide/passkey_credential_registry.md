# CredentialRegistry

The SDK ships only the contract (a `CredentialRegistry` interface on each platform). Bring your own implementation: Keychain on iOS, Block Store + SharedPreferences on Android, `localStorage` on web, or a custom backend. Registry calls are best-effort with a 3 second timeout: failures fire `onRegistryError` (when set) and the WebAuthn ceremony proceeds. Pre-tracking credentials get seeded on first assertion so subsequent registrations correctly hit the platform-level "already exists" guard via `excludeCredentialIds`.

## iOS Keychain (with iCloud Keychain sync)

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

## Android Block Store + SharedPreferences fallback

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

## Web localStorage

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

## Using credentials() with a registry

Once a registry is wired in, the SDK auto-merges its contents into `excludeCredentialIds` on register and `allowCredentialIds` on sign-in. You can also inspect and mutate the stored set directly via the {{#name PasskeyClient.credentials}} sub-object, typically to back a settings page that lists registered passkeys on this device with per-row remove.

{{#tabs passkey:with-credential-registry}}

## When (not) to call `credentials().clear()`

`clear()` wipes the app's local bookkeeping only. Existing credentials stay on the OS / cloud authenticator and the user can still sign in with them.

**Don't call `clear()` on a normal sign-out.** With the registry empty, the next `register()` sends an empty `excludeCredentialIds`. On some authenticators that mints a **sibling credential**: same RP, different credential ID, **different PRF output → different seed → different wallet**. If the user picks the sibling at next sign-in, they reach an unfamiliar wallet.

Dedup behavior on an empty `excludeCredentialIds`:

| Authenticator | Behavior |
|---|---|
| iOS / iCloud Keychain | Mints sibling silently |
| Hardware keys (YubiKey etc.) | Mints sibling silently |
| Google Password Manager | Usually warns or dedupes |

Reserve `clear()` for explicit factory-reset flows where orphan credentials are acceptable.

## Error semantics

- Every registry call is bounded by a 3 second timeout. Slow backends never block the WebAuthn ceremony.
- Failures and timeouts are logged (Rust `tracing::warn`, Swift `os_log`, Kotlin `Log.w`, JS `console.warn`) and surfaced via the per-provider `onRegistryError` callback when set. `read` defaults to an empty list; `add`/`remove`/`clear` fire-and-forget.
- Web/RN/Flutter perform a constructor-time conformance check. A registry object missing one of `read` / `add` / `remove` / `clear` throws on construction so misconfiguration surfaces at startup. iOS / Kotlin / Rust enforce conformance at compile time.
- When a `CredentialNotFound` would propagate AND no `allowCredentialIds` AND no `credentialRegistry` were supplied, the SDK appends a help URL to the error message pointing here.
