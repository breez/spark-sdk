## Passkey login

### UX principles

- **Be transparent about the trust model.** Users should know up front that the passkey is the wallet, not just a convenience layer.
- **Make passkey login a choice on supported devices, not a forced default.** Users who prefer mnemonic onboarding should be able to opt out without friction.

### Example onboarding flow

The steps below describe one possible shape for a passkey onboarding flow that supports multiple labels per user. Adapt to your app's structure; apps that use a single hardcoded label can skip the label picker steps.

1. **Detect existing passkey**: Call {{#name Passkey.list_labels}}. This triggers one passkey authentication prompt.
   - **Labels found** → show a label picker (returning user).
   - **No labels but passkey exists** → show a label picker with a sensible default pre-filled.
   - **User cancelled / credential not found** → treat as "no existing passkey detected" and offer to create one.
2. **Warn new users**: Before registering a new passkey, show a screen explaining that the passkey is how the wallet is accessed and that deleting it without a backup may permanently lose access. Require explicit acknowledgment.
3. **Create passkey**: Call {{#name create_passkey}} to register the credential. Keeping this separate from derivation lets the user review before the OS prompts for credential creation.
4. **Store label**: Call {{#name Passkey.store_label}} to publish the chosen label to Nostr so it can be discovered on other devices or after reinstall.
5. **Derive and connect**: Call {{#name Passkey.get_wallet}} with the chosen label and pass the returned seed directly to {{#name connect}}. Reusing the seed across both calls avoids a second passkey prompt.

Each step should own its own error handling. A failure partway through should retry only the failed step, not the entire flow.

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

### Guidelines

1. **Gate passkey UI on availability.** Check {{#name is_prf_available}} at startup and fall back to mnemonic onboarding on unsupported devices (Android < 9, iOS < 18, browsers without WebAuthn PRF).
2. **Reuse the derived seed within an action.** Calling {{#name Passkey.get_wallet}} and then {{#name connect}} derives the seed twice and prompts the user twice. Pass the seed returned from `get_wallet` directly into `connect` instead of re-deriving.
3. **Cache the user-selected label across sessions.** Store the chosen label locally (e.g., `localStorage` on web, `SharedPreferences` on Android, `UserDefaults` on iOS, `FlutterSecureStorage` on Flutter). On subsequent launches, skip the label picker and go straight to passkey authentication.
4. **Never persist the derived mnemonic.** Re-derive from the passkey and label on each session. Persisting the mnemonic would bypass the OS authentication prompt.
5. **Allow manual mnemonic backup.** Offer a user-initiated "Show recovery phrase" path that derives the mnemonic on demand via {{#name Passkey.get_wallet}}. This gives users a recovery option if they lose access to their passkey.
6. **Handle cancellation by returning to the originating screen.** When {{#enum PasskeyPrfError::UserCancelled}} is raised, return the user to the screen where they triggered the action and let them re-initiate. Don't auto-retry, since automatic retry creates a prompt loop.
7. **Treat "no credential" as an opportunity, not an error.** `noCredential` / `credentialNotFound` errors mean no passkey exists for the RP ID yet. Offer registration rather than showing an error.
8. **Fall back to manual label entry.** When {{#name Passkey.list_labels}} returns an empty list (e.g., relays unreachable, events pruned), let the user type the label manually instead of blocking the flow.
