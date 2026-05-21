# Passkey Login

Passkey Login lets users access their wallet with biometrics (fingerprint, face scan, or device PIN) instead of writing down and safeguarding a seed phrase. The SDK uses the <a target="_blank" href="https://w3c.github.io/webauthn/#prf-extension">WebAuthn PRF extension</a> to deterministically derive wallet keys from a passkey. Keys are never stored; they're regenerated on demand each time the user authenticates. The protocol also supports multiple wallets, each derived from a different label, with labels discoverable via Nostr relays.

For the full technical specification, see the <a target="_blank" href="https://github.com/breez/passkey-login/blob/main/spec.md">Passkey Login spec</a>. For UX recommendations, see the [UX guide](./uxguide_passkey.md).

## Integration outline

1. [Setup](./passkey_setup.md): pick an RP and host the per-platform configuration files.
2. [Initialization](./passkey_initializing.md): construct a {{#name PasskeyClient}} and gate UI on availability.
3. [Onboarding](./passkey_onboarding.md): run sign-in / register / unified flow; handle recoverable errors.
4. [Advanced](./passkey_advanced.md): drop down from `createPasskeyClient` for custom providers, registries, label management, and platform-specific diagnostics.

## Supported specs

- [Seedless Restore](https://github.com/breez/seedless-restore): Passkey-based wallet derivation and discovery
- [Nostr](https://github.com/nostr-protocol/nostr): Relay-based event protocol for label storage
- [NIP-42](https://github.com/nostr-protocol/nips/blob/master/42.md): Authentication of clients to relays
- [NIP-65](https://github.com/nostr-protocol/nips/blob/master/65.md): Relay List Metadata
