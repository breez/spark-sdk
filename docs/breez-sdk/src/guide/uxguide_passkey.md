## Passkey login

### UX principles

- **Be transparent about the trust model.** Users should know up front that the passkey is the wallet, not just a convenience layer.
- **Make passkey login a choice on supported devices, not a forced default.** Users who prefer mnemonic onboarding should be able to opt out without friction.

### Recommended onboarding flow

The recommended UX differs by platform because web and mobile authenticators expose different error semantics.

**Mobile (iOS 18+, Android 9+):** Single primary CTA ("Use Passkey") backed by {{#name PasskeyClient.connect_with_passkey}}. The SDK runs a silent sign-in attempt first (`preferImmediatelyAvailableCredentials = true`); a returning user gets one biometric prompt, a fresh-device user fast-fails with no UI shown and the SDK transparently falls through to register. Only {{#enum PrfProviderError::CredentialNotFound}} triggers the fall-through. `Cancel`, `Timeout`, and `Configuration` errors propagate unchanged: on a real user cancel (≥ 300ms), show a sticky retry screen and do NOT auto-fall-through to register.

| Path | OS prompts on mobile |
|---|---|
| Returning user (silent sign-in succeeds) | **1** prompt (single dual-salt assertion derives master + label) |
| New user (silent sign-in fast-fails → register) | **2** prompts (1 create + 1 dual-salt assertion) |

The SDK's bulk-PRF path runs the create + label-store + assertion as the two prompts above; a naive sequence would cost three.

**Web:** Two CTAs ("Sign In" and "Create Account"), letting the user pick the right one. WebAuthn deliberately collapses "no credential found" and "user cancelled" into the same `NotAllowedError` for privacy reasons, so the SDK cannot reliably auto-fall-through on web the way it can on mobile. The unified `connect_with_passkey` is therefore not surfaced on the WASM target: call {{#name PasskeyClient.sign_in}} and {{#name PasskeyClient.register}} as separate entry points and trust the user to know whether they're returning or new.

### Adding a new label to an existing identity

For a user who already has a passkey and wants to create another wallet under a new label:

1. {{#name PasskeyClient.sign_in}} with the new label. This derives master + new-label seeds in one assertion AND seeds the SDK's identity cache.
2. {{#name PasskeyLabels.store}} (via {{#name PasskeyClient.labels}}) to publish the new label to Nostr. This uses the cached identity for free: no additional prompt.

Total: **1** OS prompt. The reverse order (`labels().store` before `sign_in`) costs 2 prompts because each call would derive the master salt independently.

### Credential ID and user-handle management

`createPasskey()` returns a `RegisteredCredential` carrying:

- `credentialId` - the opaque ID the authenticator assigned to the new credential.
- `userId` - the WebAuthn `user.id` (user handle). The SDK generates a fresh random 16-byte value per call and surfaces it here for host-side correlation. Hosts cannot supply one: reusing a value across creates on the same `rpId` silently overwrites the prior credential on some authenticators (Apple Passwords), destroying the PRF secret the wallet derives from. The SDK locks this down by always randomizing.
- `aaguid` - 16-byte authenticator provider identifier parsed from attestation. `null` when the platform doesn't surface enough authenticator data.
- `backupEligible` - the BE flag indicating whether the credential can sync across devices. `null` when not extractable.

Store the credential ID locally (for example in `localStorage`, `SharedPreferences`, or `UserDefaults`) so you can:

1. **Prevent duplicate credentials.** On subsequent calls to `createPasskey()`, pass the stored IDs via the `excludeCredentialIds` parameter. When the authenticator already holds one of these credentials, the platform will reject the registration with a "credential already registered" error instead of silently creating a duplicate.
2. **Correlate credentials server-side.** Persist `userId` alongside any server record you keep for the credential. The SDK never sends this value anywhere; hosts that maintain a backend account model use it to match a WebAuthn assertion's user handle back to their record.
3. **Track which credentials belong to this app instance.** The credential ID is an opaque byte array assigned by the authenticator. It is not sensitive, but it is only useful on the device where the credential was created.
4. **Display provider name + sync state in account-management UI.** AAGUID identifies the credential provider (iCloud Keychain, Google Password Manager, 1Password, hardware key, etc.) when looked up against a known-AAGUID database such as [passkeydeveloper/passkey-authenticator-aaguids](https://github.com/passkeydeveloper/passkey-authenticator-aaguids). `backupEligible` indicates whether the credential will sync to the user's other devices. Both AAGUID and BE are unverified attestation: use as display hints only, never for trust decisions.

```typescript
// Web example
const { credentialId, userId, aaguid, backupEligible } = await prfProvider.createPasskey();
// Store for later use
localStorage.setItem('passkeyCredentialId', btoa(String.fromCharCode(...credentialId)));
localStorage.setItem('passkeyUserId', btoa(String.fromCharCode(...userId)));

// On next registration attempt, exclude existing credentials
const existingId = localStorage.getItem('passkeyCredentialId');
const exclude = existingId
  ? [Uint8Array.from(atob(existingId), c => c.charCodeAt(0))]
  : [];
const newCredential = await prfProvider.createPasskey({ excludeCredentialIds: exclude });
```

### Recovery paths

The canonical branch point is `error.kind()`: it collapses the nine `PrfProviderError` variants into seven actionable [`ErrorKind`](https://breez.github.io/spark-sdk/breez_sdk_spark/passkey/enum.ErrorKind.html) values. Match on `ErrorKind` instead of the individual variants:

| `ErrorKind` | What it means | Recommended UX |
|---|---|---|
| `NoCredential` | No matching passkey on this device (includes the iOS sub-300ms fast-fail that the platform reports as `UserCancelled`) | Fall through to {{#name PasskeyClient.register}} automatically |
| `Cancel` (≥ 300ms, < 55s elapsed) | User dismissed the OS prompt | Sticky error screen with explicit "Try Again" + "Go Back". **Do not** auto-retry. |
| `AlreadyExists` | OS rejected create because a credential already exists in the user's password manager (`matchedExcludedCredential` on iOS, `InvalidStateError` on Android) | Switch path to {{#name PasskeyClient.sign_in}} so the OS picker surfaces the existing cred |
| `Configuration` | Associated Domains entitlement missing or AASA misconfigured | Developer-facing error; surface {{#name check_domain_association}}'s `NotAssociated` reason text |
| `PrfUnsupported` | Device or authenticator doesn't support the PRF extension | Fall back to mnemonic onboarding, gated on {{#name PasskeyClient.check_availability}} at startup |
| `Timeout` (≥ 55s elapsed) | OS biometric inactivity timeout | Sticky retry with timeout-specific copy ("Sign-in timed out: the system stopped waiting for biometrics") |
| `Internal` | Network / library / generic failure | Generic "try again later" UI; log details for diagnostics |

The SDK does the sub-300ms / 300ms-55s / 55s+ classification internally on iOS: hosts that branch on `error.kind()` see the distilled `NoCredential` / `Cancel` / `Timeout` outcomes directly, no host-side stopwatch needed.

### Guidelines

1. **Gate passkey UI on availability.** Call {{#name PasskeyClient.check_availability}} at startup and fall back to mnemonic onboarding on unsupported devices (Android < 9, iOS < 18, browsers without WebAuthn PRF). The same call surfaces domain-association failures, so a single check covers both "device can't" and "config is broken".
2. **Match the CTA layout to the platform.** On mobile, the recommended UX is a single primary CTA backed by {{#name PasskeyClient.connect_with_passkey}}: silent sign-in for returning users, automatic fall-through to register for new ones. On web, present two CTAs ("Sign In" and "Create Account") and let the user pick: WebAuthn's privacy-preserving error collapse makes auto-detection unreliable. Don't gate registration behind a separate "I understand" review screen on either platform: the OS already shows its own consent UI.
3. **Cache the derived seed within a session.** The SDK caches the Nostr identity inside the {{#name PasskeyClient}} instance for the duration of a sign-in / register call, so subsequent {{#name PasskeyLabels.list}} / {{#name PasskeyLabels.store}} calls (via {{#name PasskeyClient.labels}}) don't re-prompt. For longer-lived caching (e.g. across a router rebuild on app launch), store the seed in your own in-memory cache and pass it to {{#name connect}} when reconnecting; do not persist to disk.
4. **Never persist the derived mnemonic.** Re-derive from the passkey and label on each session. Persisting the mnemonic would bypass the OS authentication prompt.
5. **Allow manual mnemonic backup.** Offer a user-initiated "Show recovery phrase" path that derives the mnemonic on demand via {{#name PasskeyClient.sign_in}}. This gives users a recovery option if they lose access to their passkey.
6. **Branch on `error.kind()`, not on individual variants.** `ErrorKind` (7 values) is the canonical surface; `PrfProviderError` (9 variants) is for diagnostic detail. The SDK already classifies the iOS sub-300ms-vs-55s+ wall-clock ambiguity internally so `error.kind()` distills it to `NoCredential` / `Cancel` / `Timeout`: no host-side stopwatch needed.
7. **Don't auto-retry on dismissed prompts.** The SDK guarantees it never re-fires the OS prompt without an explicit retry from the host. Build your state machine the same way: dismiss → sticky error screen with Try Again button → user-driven retry.
