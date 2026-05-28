# CredentialRegistry

`CredentialRegistry` is an optional app-side store of credential IDs your app has registered. The SDK uses it on register (to populate `excludeCredentials` so the OS refuses to create a duplicate) and on sign-in (to populate `allowCredentials` so the OS surfaces the right credential in the picker). You implement it yourself, backed by whatever local storage fits your platform; the SDK does not ship a default.

## Interface

Four async methods, keyed by `rpId`:

```typescript
interface CredentialRegistry {
  read(rpId: string): Promise<Uint8Array[]>;
  add(rpId: string, credentialId: Uint8Array): Promise<void>;
  remove(rpId: string, credentialId: Uint8Array): Promise<void>;
  clear(rpId: string): Promise<void>;
}
```

`read` and `add` are SDK-driven: the SDK calls `read` before every sign-in and register so the OS sees the right `allowCredentials` / `excludeCredentials`, and calls `add` after a successful register to record the new ID. `remove` and `clear` are app-driven: wire them to a settings page where users manage registered passkeys. See [When (not) to call `credentials().clear()`](#when-not-to-call-credentialsclear) before exposing `clear`.

Credential IDs are public per WebAuthn spec, so no encryption is required at rest.

## Platform suggestions

- **iOS**: Wrap `Security.framework` generic-password items with `kSecAttrSynchronizable = true` so the registry replicates across the user's iCloud-signed-in devices and survives reinstall. One generic-password item per RP, with a JSON-encoded array of base64 credential IDs as the value.
- **Android**: Use <a target="_blank" href="https://developers.google.com/identity/blockstore">Block Store</a> (Google-account-synced, ~16 KB per app) as the primary store, with `SharedPreferences` as a local fallback for offline / not-signed-in cases. Read unions both sources; writes go to both. Available via the `play-services-auth-blockstore` artifact.
- **Web**: `window.localStorage` is the simplest option: JSON-encoded base64url credential IDs per RP. Cleared on site-data clear and not synced across devices. For larger capacity or cross-tab sync, IndexedDB works the same shape.

## Using credentials() with a registry

With a registry wired in, the SDK auto-populates `excludeCredentials` on register and `allowCredentials` on sign-in. {{#name PasskeyClient.credentials}} lets you read and modify the stored set yourself, typically to back a settings page that lists registered passkeys on this device with per-row remove.

{{#tabs passkey:with-credential-registry}}

## When (not) to call `credentials().clear()`

`clear()` wipes only the app's local list of credential IDs. The passkey itself stays on the OS or cloud authenticator, so the user can still sign in with it. `clear()` is not a logout.

**Don't call `clear()` on a normal sign-out.** With an empty registry, the next `register()` sends an empty `excludeCredentials`. Some authenticators treat that as "no duplicates known" and mint a **sibling credential**: same RP, different credential ID. The new credential ID produces a different PRF output, which derives a different seed, which lands the user in a different wallet at next sign-in.

Dedup behaviour with an empty `excludeCredentials`:

| Authenticator | Behaviour |
|---|---|
| iOS / iCloud Keychain | Mints a sibling silently |
| Hardware keys (YubiKey etc.) | Mints a sibling silently |
| Google Password Manager | Usually warns or dedupes |

Reserve `clear()` for explicit factory-reset flows where orphan credentials are acceptable.

## Error handling

- Every registry call is bounded by a 3 second timeout. Slow backends never block the WebAuthn ceremony.
- Failures and timeouts are logged and surfaced via the `onRegistryError` callback when set. A failed `read` falls back to an empty list; failed writes are dropped without blocking the ceremony.
- An incomplete registry (missing one of `read`, `add`, `remove`, `clear`) is rejected as early as your platform allows: either at app startup or at build time.
- If sign-in fails with no matching credential and you have not supplied `allowCredentials` or a registry to narrow the lookup, the SDK links back to this page in the error message.
