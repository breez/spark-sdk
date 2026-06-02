## Passkey login

### UX principles

- **Be transparent about the trust model.** Users should know up front that the passkey is the wallet, not just a convenience layer.
- **Make passkey login a choice on supported devices, not a forced default.** Users who prefer mnemonic onboarding should be able to opt out without friction.

### Guidelines

1. **Gate passkey UI on availability.** Call {{#name PasskeyClient.check_availability}} at startup and fall back to mnemonic onboarding on unsupported devices (Android < 9, iOS < 18, browsers without WebAuthn PRF). The same call surfaces domain-association failures, so a single check covers both "device can't" and "config is broken".
2. **Match the CTA layout to the platform.** On iOS and Android, the recommended UX is a single primary CTA backed by {{#name PasskeyClient.connect_with_passkey}}: silent sign-in for returning users, automatic fall-through to register for new ones. On web, present two CTAs ("Sign In" and "Create Account") and let the user pick: WebAuthn's privacy-preserving error collapse makes auto-detection unreliable. Don't gate registration behind a separate "I understand" review screen on either platform: the OS already shows its own consent UI.
3. **Cache the derived seed within a session.** A sign-in / register call primes the Nostr identity cache on the {{#name PasskeyClient}} instance, so subsequent {{#name PasskeyLabels.list}} / {{#name PasskeyLabels.store}} calls (via {{#name PasskeyClient.labels}}) reuse it without re-prompting. For longer-lived caching (e.g. across a router rebuild on app launch), store the seed in your own in-memory cache and pass it to {{#name connect}} when reconnecting; do not persist to disk.
4. **Never persist the derived mnemonic.** Re-derive from the passkey and label on each session. Persisting the mnemonic would bypass the OS authentication prompt.
5. **Allow manual mnemonic backup.** Offer a user-initiated "Show recovery phrase" path that derives the mnemonic on demand via {{#name PasskeyClient.sign_in}}. This gives users a recovery option if they lose access to their passkey.
6. **Match on {{#name PrfProviderError}} variants.** They are the cross-language error surface. Rust callers can branch on the collapsed `error.kind()` / `ErrorKind` instead; `kind()` is Rust-only.
7. **Don't auto-retry on dismissed prompts.** The SDK never re-fires the OS prompt without an explicit retry from the host. Build your state machine the same way: a dismissed prompt leads to a sticky error screen with a Try Again button, and only a user tap retries.

### Onboarding flow

Browsers and native authenticators expose different error semantics, so the recommended UX differs by platform.

**iOS 18+ / Android 9+:** one "Use Passkey" button backed by {{#name PasskeyClient.connect_with_passkey}}. It tries a silent sign-in first: a returning user gets a single biometric prompt; a new user fast-fails with no UI and the SDK falls through to register. On a real cancel, show a sticky retry and do not auto-register.

| Path | OS prompts |
|---|---|
| Returning user | **1** (one assertion derives master + label) |
| New user | **2** (1 create, 1 assertion) |

**Web:** two buttons, "Sign In" and "Create Account". WebAuthn reports "no credential" and "user cancelled" identically, so the SDK can't auto-detect which the user wants; let them choose. {{#name PasskeyClient.connect_with_passkey}} is not surfaced on the WASM target.

See [Onboarding](./passkey_onboarding.md) for the call shapes.

### Adding a wallet under a new label

For a user who already has a passkey and wants another wallet:

1. {{#name PasskeyClient.sign_in}} with the new label. One assertion derives the master and new-label seeds and primes the identity cache.
2. {{#name PasskeyLabels.store}} (via {{#name PasskeyClient.labels}}) to publish the label to Nostr. This reuses the cached identity, so it adds no prompt.

Total: **1** OS prompt. Storing the label first would cost 2, because each call would derive the master salt independently.

### Credential metadata

Every flow returns a {{#name PasskeyCredential}}: the credential ID on every path, plus the user handle, AAGUID, and backup flag on registration. Persist them so you can keep a returning user on the same wallet, block duplicate registrations on one device, and show which authenticator holds the passkey and whether it syncs. AAGUID and the backup flag are unverified attestation: treat them as display hints, never as trust signals.

See [Credential metadata](./passkey_credential_metadata.md) for the fields, where to store them, and the per-use-case code.

### Recovery paths

Every passkey failure normalizes to a {{#name PrfProviderError}} variant. Match on the variant to choose the UX:

| Variant | What it means | Recommended UX |
|---|---|---|
| {{#enum PrfProviderError::CredentialNotFound}} | No matching passkey on this device | Fall through to {{#name PasskeyClient.register}} automatically |
| {{#enum PrfProviderError::UserCancelled}} | User dismissed the prompt | Sticky screen with "Try Again" and "Go Back". **Do not** auto-retry |
| {{#enum PrfProviderError::CredentialAlreadyExists}} | Create hit a credential already on the device | Switch to {{#name PasskeyClient.sign_in}} so the OS picker surfaces it |
| {{#enum PrfProviderError::Configuration}} | Entitlement or AASA misconfigured | Developer-facing error; surface {{#name check_domain_association}}'s {{#enum PasskeyAvailability::NotAssociated}} reason |
| {{#enum PrfProviderError::PrfNotSupported}} | Device or authenticator lacks the PRF extension | Fall back to mnemonic onboarding |
| {{#enum PrfProviderError::UserTimedOut}} | OS biometric inactivity timeout | Sticky retry with timeout-specific copy |
| {{#enum PrfProviderError::Generic}} | Network / library / generic failure | Generic "try again later"; log for diagnostics |

On iOS the SDK resolves the platform's wall-clock ambiguity (a generic failure could be a missing credential, a cancel, or a timeout) into these variants for you, so you don't need a host-side stopwatch.
