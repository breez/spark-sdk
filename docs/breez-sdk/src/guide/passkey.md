# Passkey login

Passkey login lets users access their wallet using biometrics (fingerprint or face recognition) or a device PIN, eliminating the need to write down and safeguard a seed phrase.

No keys or seed phrases are stored. The SDK uses the <a target="_blank" href="https://w3c.github.io/webauthn/#prf-extension">WebAuthn PRF extension</a> to deterministically derive a seed phrase from the user's passkey on-demand, and regenerates it on each sign-in.

One passkey can be associated with multiple wallets, each under its own label, discoverable across the user's devices through Nostr relays.

- **[Setup](passkey_setup.md)** - host the Web / Android / iOS configuration files that tie passkeys to your app.
- **[Onboarding](passkey_onboarding.md)** - initialize the client and wire up the sign-in, register, and unified onboarding flows.
- **[Credential metadata](passkey_credential_metadata.md)** - pin a returning user, prevent duplicate registrations, and show the authenticator and sync status.
- **[Managing labels](passkey_labels.md)** - derive multiple wallets from one passkey and discover them through Nostr.
- **[PRF providers](passkey_prf_providers.md)** - use the built-in platform provider, or implement a custom one (hardware key, FIDO2, file-backed).

## Further reading

- **[UX guidelines](uxguide_passkey.md)** - recommended onboarding UX, prompt counts, and error-recovery patterns.

For the full technical specification, see the <a target="_blank" href="https://github.com/breez/passkey-login/blob/main/spec.md">Passkey Login spec</a>.
