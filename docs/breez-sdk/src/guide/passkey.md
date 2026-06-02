# Passkey login

Passkey Login lets users access their wallet with biometrics (fingerprint, face scan, or device PIN) instead of writing down and safeguarding a seed phrase. The SDK uses the <a target="_blank" href="https://w3c.github.io/webauthn/#prf-extension">WebAuthn PRF extension</a> to deterministically derive wallet keys from a passkey. Keys are never stored; they're regenerated on demand each time the user authenticates. The protocol also supports multiple wallets, each derived from a different label, with labels discoverable via Nostr relays.

- **[Setup](passkey_setup.md)** - host the Web / Android / iOS configuration files that tie passkeys to your app
- **[Onboarding](passkey_onboarding.md)** - initialize the client and wire up the register / sign-in / unified onboarding flows
- **[Credential metadata](passkey_credential_metadata.md)** - pin a returning user, prevent duplicate registrations, and show the authenticator and sync status
- **[Managing labels](passkey_labels.md)** - derive multiple wallets from a single passkey and discover them through Nostr
- **[PRF providers](passkey_prf_providers.md)** - use the built-in platform provider, or implement a custom one (hardware key, FIDO2, file-backed)

## Further reading

- **[UX guidelines](uxguide_passkey.md)** - recommended onboarding UX, prompt counts, and error-recovery patterns

For the full technical specification, see the <a target="_blank" href="https://github.com/breez/passkey-login/blob/main/spec.md">Passkey Login spec</a>.
