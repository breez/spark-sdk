## Passkey login

### UX principles

- **Be transparent about the trust model.** Users should know up front that the passkey is the wallet, not just a convenience layer.
- **Make passkey login a choice on supported devices, not a forced default.** Users who prefer mnemonic onboarding should be able to opt out without friction.

### Recommended onboarding flow: silent sign-in → fall through to create

The simplest user-facing UX is a single primary CTA ("Use Passkey") that does the right thing regardless of whether the user is new or returning:

1. Call {{#name PasskeyClient.sign_in}} with `label = None` (discovery mode) or your default label (`Some("Default")`). On iOS+Android, the platform's `preferImmediatelyAvailableCredentials` flag means this is a **silent assertion**: a returning user gets one biometric prompt and is signed in; a fresh-device user with no credential fast-fails (no UI shown) with {{#enum PrfProviderError::CredentialNotFound}} in under 300ms.
2. Catch {{#enum PrfProviderError::CredentialNotFound}} (and on iOS also `UserCancelled` returned in under ~300ms — Apple's API conflates the two when no UI was shown). Call {{#name PasskeyClient.register}} to create + derive.
3. On {{#enum PrfProviderError::UserCancelled}} taking longer than ~300ms, the user actively dismissed the prompt — show a sticky retry screen, do NOT auto-fall-through to register.

Total OS prompt count for the user-visible flow:

| Path | iOS / Android |
|---|---|
| Returning user (silent sign-in succeeds) | **1** prompt (single dual-salt assertion derives master + label) |
| New user (silent sign-in fast-fails → register) | **2** prompts (1 create + 1 dual-salt assertion) |

The SDK's bulk-PRF path collapses what used to be three separate ceremonies (create → store label → assert) into the two prompts above.

### Adding a new label to an existing identity

For a user who already has a passkey and wants to create another wallet under a new label:

1. {{#name PasskeyClient.sign_in}} with the new label. This derives master + new-label seeds in one assertion AND seeds the SDK's identity cache.
2. {{#name PasskeyClient.store_label}} to publish the new label to Nostr. This uses the cached identity for free — no additional prompt.

Total: **1** OS prompt. The earlier order (`store_label` then `sign_in`) cost 2 prompts because each call derived the master salt independently.

### Credential ID management

`createPasskey()` returns a `RegisteredCredential` carrying the credential ID plus authenticator metadata parsed from the attestation: `aaguid` (16-byte provider identifier) and `backupEligible` (BE flag indicating the credential can sync across devices). Both metadata fields are `null` when the platform doesn't surface enough authenticator data. Store the credential ID locally (for example in `localStorage`, `SharedPreferences`, or `UserDefaults`) so you can:

1. **Prevent duplicate credentials.** On subsequent calls to `createPasskey()`, pass the stored IDs via the `excludeCredentialIds` parameter. When the authenticator already holds one of these credentials, the platform will reject the registration with a "credential already registered" error instead of silently creating a duplicate.
2. **Track which credentials belong to this app instance.** The credential ID is an opaque byte array assigned by the authenticator. It is not sensitive, but it is only useful on the device where the credential was created.
3. **Display provider name + sync state in account-management UI.** AAGUID identifies the credential provider (iCloud Keychain, Google Password Manager, 1Password, hardware key, etc.) when looked up against a known-AAGUID database such as [passkeydeveloper/passkey-authenticator-aaguids](https://github.com/passkeydeveloper/passkey-authenticator-aaguids). `backupEligible` indicates whether the credential will sync to the user's other devices. Both AAGUID and BE are unverified attestation: use as display hints only, never for trust decisions.

```typescript
// Web example
const { credentialId, aaguid, backupEligible } = await prfProvider.createPasskey();
// Store for later use
localStorage.setItem('passkeyCredentialId', btoa(String.fromCharCode(...credentialId)));

// On next registration attempt, exclude existing credentials
const existingId = localStorage.getItem('passkeyCredentialId');
const exclude = existingId
  ? [Uint8Array.from(atob(existingId), c => c.charCodeAt(0))]
  : [];
const newCredential = await prfProvider.createPasskey(exclude);
```

### Recovery paths

Map each error variant to a user-facing recovery affordance. The SDK never auto-retries — every recovery is host-driven.

| Error | What it means | Recommended UX |
|---|---|---|
| {{#enum PrfProviderError::CredentialNotFound}} (or iOS fast-fail `UserCancelled` < 300ms) | No matching passkey on this device | Fall through to {{#name PasskeyClient.register}} automatically |
| {{#enum PrfProviderError::UserCancelled}} (>= 300ms elapsed) | User dismissed the OS prompt | Sticky error screen with explicit "Try Again" + "Go Back". **Do not** auto-retry. |
| {{#enum PrfProviderError::CredentialAlreadyExists}} | OS rejected create because a credential already exists in the user's password manager (`matchedExcludedCredential` on iOS, `InvalidStateError` on Android) | Switch path to {{#name PasskeyClient.sign_in}} so the OS picker surfaces the existing cred |
| {{#enum PrfProviderError::Configuration}} | Associated Domains entitlement missing or AASA misconfigured | Surface a developer-facing error using {{#name check_domain_association}}'s `NotAssociated` reason text |
| {{#enum PrfProviderError::PrfNotSupported}} | Device or authenticator doesn't support the PRF extension | Fall back to mnemonic onboarding, gated on {{#name is_available}} at startup |
| Cancel-shaped error after >= 55s elapsed | OS biometric inactivity timeout | Sticky retry with timeout-specific copy ("Sign-in timed out — the system stopped waiting for biometrics") |

### Guidelines

1. **Gate passkey UI on availability.** Check {{#name is_available}} at startup and fall back to mnemonic onboarding on unsupported devices (Android < 9, iOS < 18, browsers without WebAuthn PRF).
2. **Use the silent-sign-in-fallback-create flow.** Don't gate registration behind a separate "I understand" review screen for the typical user — the OS already shows its own consent UI. {{#name PasskeyClient.sign_in}} returns the wallet directly; pass `wallet.seed` straight to {{#name connect}}.
3. **Cache the derived seed within a session.** The SDK caches the Nostr identity inside the {{#name PasskeyClient}} instance for the duration of a sign-in / register call, so subsequent {{#name PasskeyClient.list_labels}} / {{#name PasskeyClient.store_label}} calls don't re-prompt. For longer-lived caching (e.g. across a router rebuild on app launch), store the seed in your own in-memory cache and pass it to {{#name connect}} when reconnecting; do not persist to disk.
4. **Never persist the derived mnemonic.** Re-derive from the passkey and label on each session. Persisting the mnemonic would bypass the OS authentication prompt.
5. **Allow manual mnemonic backup.** Offer a user-initiated "Show recovery phrase" path that derives the mnemonic on demand via {{#name PasskeyClient.sign_in}}. This gives users a recovery option if they lose access to their passkey.
6. **Distinguish fast-fail cancel from real cancel using elapsed time.** iOS conflates "no credential available" and "user dismissed the prompt" — both come back as `UserCancelled`. The only signal is wall-clock time: under ~300ms means no UI was shown (treat as no-cred, fall through to register); 300ms–55s means the user actually dismissed (sticky retry); 55s+ means OS biometric inactivity timeout. Handled internally by the SDK on Android via `NoCredentialException`; on iOS the host should time the call.
7. **Don't auto-retry on dismissed prompts.** The SDK guarantees it never re-fires the OS prompt without an explicit retry from the host. Build your state machine the same way: dismiss → sticky error screen with Try Again button → user-driven retry.
