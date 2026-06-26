## Passkey login

### UX principles

- **Be transparent about the trust model.** Users should know up front that the passkey is the wallet, not just a convenience layer.
- **Make passkey login a choice on supported devices, not a forced default.** Users who prefer mnemonic onboarding should be able to opt out without friction.

### Guidelines

1. **Gate passkey UI on availability.** Call {{#name PasskeyClient.check_availability}} at startup and fall back to mnemonic onboarding on unsupported devices. The same call surfaces domain-association failures, so one check covers both "device can't" and "config is broken".
2. **Match the CTA layout to the platform.** iOS and Android: a single primary CTA backed by {{#name PasskeyClient.connect_with_passkey}} (silent sign-in for returning users, fall-through to register for new ones). Web: the same single CTA where {{#name immediate_mediation_supported}} is set on {{#name PasskeyClient.check_availability}}, otherwise two CTAs ("Create a new passkey" / "Sign in with a passkey"), since without immediate mediation WebAuthn can't tell "no credential" from "cancel".
3. **Don't add your own consent screen.** The OS shows its own consent UI, so don't gate registration behind a separate "I understand" review step.
4. **Cache the derived seed within a session.** A sign-in or register call avoids re-prompting for later {{#name PasskeyLabels.list}} / {{#name PasskeyLabels.store}} calls in the same session. For reuse across an app relaunch, keep the seed in your own in-memory cache and pass it to {{#name connect}}; never persist it to disk.
5. **Never persist the derived mnemonic.** Re-derive it from the passkey and label on each session. Persisting it would bypass the OS authentication prompt.
6. **Allow manual mnemonic backup.** Offer a user-initiated "Show recovery phrase" path that derives the mnemonic on demand via {{#name PasskeyClient.sign_in}}, so users keep a recovery option if they lose the passkey.
7. **Match on {{#name PrfProviderError}} variants.** They are the cross-language error surface. Rust callers can branch on the collapsed `error.kind()` instead.
8. **Don't auto-retry on dismissed prompts.** The SDK never re-fires the OS prompt on its own. A dismissed prompt should lead to a sticky error screen with a "Try Again" button; only a user tap retries.

### Onboarding flow

Browsers and native authenticators expose different error semantics, so the recommended UX differs by platform.

**iOS 18+ / Android 9+:** one "Use Passkey" button backed by {{#name PasskeyClient.connect_with_passkey}}. It tries a silent sign-in first: a returning user gets a single biometric prompt; a new user fast-fails with no UI and the SDK falls through to register. On a real cancel, show a sticky retry and do not auto-register.

| Path | OS prompts |
|---|---|
| Returning user | **1** (one assertion derives master + label) |
| New user | **2** (1 create, 1 assertion) |

**Web:** where the browser supports immediate mediation ({{#name immediate_mediation_supported}} on {{#name PasskeyClient.check_availability}}), the same single "Use Passkey" button backed by {{#name PasskeyClient.connect_with_passkey}}. Otherwise two buttons, "Create a new passkey" and "Sign in with a passkey": without immediate mediation WebAuthn reports "no credential" and "user cancelled" identically, so the SDK can't auto-detect which the user wants.

See [Onboarding](./passkey_onboarding.md) for the call shapes.

### Adding a wallet under a new label

For a user who already has a passkey and wants another wallet:

1. {{#name PasskeyClient.sign_in}} with the new label. One assertion derives the master and new-label seeds.
2. {{#name PasskeyLabels.store}} to publish the label to Nostr. This reuses the cached identity, so it adds no prompt.

Total: **1** OS prompt. Storing the label first would cost 2, since each call would derive the master salt independently.

### Credential metadata

Every flow returns a {{#name PasskeyCredential}}: the credential ID on every path, plus the user handle, AAGUID, and backup flag on registration. Persist them to keep a returning user on the same wallet, block duplicate registrations on one device, and show which authenticator holds the passkey and whether it syncs. AAGUID and the backup flag are unverified: treat them as display hints, never trust signals.

See [Credential metadata](./passkey_credential_metadata.md) for the fields, where to store them, and the per-use-case code.

### Recovery paths

Every passkey failure normalizes to a {{#name PrfProviderError}} variant. See [Onboarding error recovery](./passkey_onboarding.md#error-recovery) for the variant-to-action table; the guidelines above cover the UX rules.

On iOS, the SDK disambiguates the platform's generic failure (missing credential, cancel, or timeout) into these variants for you, so you don't need a host-side timer.
