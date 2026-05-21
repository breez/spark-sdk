# CredentialRegistry

`CredentialRegistry` is an optional app-side store of credential IDs your app has registered. The SDK uses it on register (to populate `excludeCredentialIds` so the OS refuses to create a duplicate) and on sign-in (to populate `allowCredentialIds` so the OS surfaces the right credential in the picker). You implement it yourself, backed by whatever local storage fits your platform; the SDK does not ship a default.

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

The SDK calls `read` before any sign-in / register ceremony to populate `allowCredentialIds` / `excludeCredentialIds`, and calls `add` after a successful ceremony to record the new credential ID. `remove` and `clear` are app-driven (typically wired to a settings page; see [When (not) to call `credentials().clear()`](#when-not-to-call-credentialsclear) for the destructive-semantics caveat).

Credential IDs are public per WebAuthn spec, so no encryption is required at rest.

## Platform suggestions

- **iOS**: Wrap `Security.framework` generic-password items with `kSecAttrSynchronizable = true` so the registry replicates across the user's iCloud-signed-in devices and survives reinstall. One generic-password item per RP, with a JSON-encoded array of base64 credential IDs as the value.
- **Android**: Use <a target="_blank" href="https://developers.google.com/identity/blockstore">Block Store</a> (Google-account-synced, ~16 KB per app) as the primary store, with `SharedPreferences` as a local fallback for offline / not-signed-in cases. Read unions both sources; writes go to both. Available via the `play-services-auth-blockstore` artifact.
- **Web**: `window.localStorage` is the simplest option: JSON-encoded base64url credential IDs per RP. Cleared on site-data clear and not synced across devices. For larger capacity or cross-tab sync, IndexedDB works the same shape.

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

## Error handling

- Every registry call is bounded by a 3 second timeout. Slow backends never block the WebAuthn ceremony.
- Failures and timeouts are logged (Rust `tracing::warn`, Swift `os_log`, Kotlin `Log.w`, JS `console.warn`) and surfaced via the per-provider `onRegistryError` callback when set. `read` defaults to an empty list; `add`/`remove`/`clear` fire-and-forget.
- Web/RN/Flutter perform a constructor-time conformance check. A registry object missing one of `read` / `add` / `remove` / `clear` throws on construction so misconfiguration surfaces at startup. iOS / Kotlin / Rust enforce conformance at compile time.
- When a `CredentialNotFound` would propagate AND no `allowCredentialIds` AND no `credentialRegistry` were supplied, the SDK appends a help URL to the error message pointing here.
