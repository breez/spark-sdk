# Onboarding

Initialize the {{#name PasskeyClient}}, then run the onboarding flow that fits your platform.

## Initialization

{{#name PasskeyClient}} is the entry point for every passkey-derived wallet operation. It composes a {{#name PrfProvider}} (the platform-specific bridge to WebAuthn / Credential Manager / AuthenticationServices) with the SDK's internal Nostr-backed label store, then exposes register / sign-in / connect / labels to your app code. Construct one per app session and reuse it.

Build the platform's {{#name PasskeyProvider}}, then pass it to {{#name PasskeyClient}} along with your Breez API key:

{{#tabs passkey:setup-client}}

**Parameters:**

- {{#name breez_api_key}}: **Required.** Your Breez API key, used to authenticate to the Breez relay (<a target="_blank" href="https://github.com/nostr-protocol/nips/blob/master/42.md">NIP-42</a>) for label storage.
- {{#name rp_id}}: **Required.** Relying Party ID. Pass your app's domain, or `PasskeyProvider.BREEZ_RP_ID` if your app is Breez-registered. Changing it later strands existing credentials.
- {{#name rp_name}}: **Required.** Maps to the WebAuthn `rp.name`, which current OS prompts still mandate (deprecated in WebAuthn L3 but enforced everywhere). Surfaces in some authenticators' management UIs (Apple Passwords, Google Password Manager); platform UIs increasingly ignore it. Set at registration only; changing it does not affect existing credentials.
- {{#name user_name}}: **Optional.** Maps to the WebAuthn `user.name`. Treated as the user's unique identifier for the credential and shown in the OS account picker during sign-in. Pass a stable per-user value if each registration should surface as a distinct entry (Apple's Passwords app, in particular, dedupes credentials by `(rpId, user.name)`). Defaults to {{#name rp_name}}. Registration-only.
- {{#name user_display_name}}: **Optional.** Maps to the WebAuthn `user.displayName`. The user-friendly label the OS / browser MAY (but is not required to) show in the picker; behavior varies by platform. Defaults to {{#name user_name}}. Registration-only.
- {{#name passkey_config}}: **Optional.** Carries {{#name default_label}}, the label used when {{#name PasskeyClient.register}} / {{#name PasskeyClient.sign_in}} receive no label. Falls back to the SDK's internal `"Default"` when unset.

The SDK also implements <a target="_blank" href="https://github.com/nostr-protocol/nips/blob/master/65.md">NIP-65</a> to discover and publish to additional public relays for redundancy.

For custom PRF providers (CLI YubiKey, FIDO2, file-backed) or non-default {{#name PasskeyProvider}} options, see [PRF providers](./passkey_prf_providers.md).

### Checking passkey availability

Call {{#name PasskeyClient.check_availability}} before surfacing the passkey button. It probes OS support, PRF capability, and your domain association in one shot, so you can hide the option on incompatible devices (older Android / iOS) or surface a configuration error (missing entitlement, undeployed AASA) before the user runs into an opaque WebAuthn failure:

{{#tabs passkey:check-availability}}

## Choosing a flow

The recommended flow depends on the platform. iOS and Android use a single-call unified flow backed by {{#name PasskeyClient.connect_with_passkey}}. Web (browser) uses two buttons (Sign In and Create Account) and lets the user pick. For explicit control over each path, call {{#name PasskeyClient.sign_in}} and {{#name PasskeyClient.register}} directly.

### Unified flow (iOS / Android)

One "Use Passkey" button backed by {{#name PasskeyClient.connect_with_passkey}}: silent sign-in for a returning user, automatic fall-through to registration on a fresh device. The response's {{#name credential}} field carries whichever credential signed in or was registered; the register path also fills the attestation fields ({{#name aaguid}}, {{#name backup_eligible}}). See [Credential metadata](./passkey_credential_metadata.md) for using these.

Internally the silent attempt pins {{#name prefer_immediately_available_credentials}} to true so the OS fast-fails (no UI, sub-300ms on iOS / Android) when no local credential exists; only {{#enum PrfProviderError::CredentialNotFound}} flips to register, all other errors (`Cancel`, `Timeout`, `Configuration`) propagate unchanged.

{{#tabs passkey:connect-with-passkey}}

### Two-button flow (Web)

`connectWithPasskey` is not surfaced on the WASM target. The unified flow needs a silent "no credential here" signal so it can fall through to register, and WebAuthn deliberately collapses that case into the same `NotAllowedError` as a user cancel for privacy reasons. There is no reliable way on web for the SDK to tell the two apart.

The recommended UX on web is two buttons: a **Sign In** button calling `signIn` and a separate **Create Account** button calling `register`. Let the user pick the right one instead of trying to auto-detect. See [Sign in and register](#sign-in-and-register) for the call shapes.

### Sign in and register

For finer control, call {{#name PasskeyClient.sign_in}} and {{#name PasskeyClient.register}} directly: on web (the two buttons above), when separating Sign-In and Create-Account flows, or when adding a new label on a returning user without going through {{#name PasskeyClient.connect_with_passkey}}. Pass `wallet.seed` to {{#name connect}} in either case.

#### Sign in

Sign in to an existing credential:

{{#tabs passkey:sign-in}}

#### Register

Register a fresh credential:

{{#tabs passkey:register-passkey}}

## Error recovery

Every passkey failure normalizes to a {{#name PrfProviderError}} variant. Match on the variant to drive recovery:

| Variant | What it means | Recommended action |
|---|---|---|
| {{#enum PrfProviderError::UserCancelled}} | User dismissed the OS prompt (≥ 300ms, < 55s) | Sticky retry UI with "Try Again" |
| {{#enum PrfProviderError::CredentialNotFound}} | No matching credential on this device (includes iOS sub-300ms fast-fail) | Fall through to {{#name PasskeyClient.register}} |
| {{#enum PrfProviderError::CredentialAlreadyExists}} | Register hit a credential in {{#name exclude_credentials}} | Flip to {{#name PasskeyClient.sign_in}}; the OS picker surfaces the existing credential |
| {{#enum PrfProviderError::UserTimedOut}} | OS biometric inactivity timeout (≥ 55s): distinct from a user-cancel | Sticky retry with timeout-specific copy. **Do not** auto-retry |
| {{#enum PrfProviderError::PrfNotSupported}} | Authenticator doesn't implement the PRF extension | Fall back to mnemonic onboarding |
| {{#enum PrfProviderError::Configuration}} | Entitlement missing, AASA stale, or assetlinks malformed | Developer-facing error; surface {{#name check_domain_association}}'s {{#enum PasskeyAvailability::NotAssociated}} reason |
| {{#enum PrfProviderError::Generic}} | Network / library / generic failure | Generic "try again later" UI |

Rust callers can branch on the collapsed `error.kind()` / [`ErrorKind`](https://breez.github.io/spark-sdk/breez_sdk_spark/passkey/enum.ErrorKind.html) instead of matching every variant; `kind()` is Rust-only. Web exposes typed exception classes (`PasskeyAlreadyExistsError`, `PasskeyTimedOutError`, `PasskeyCredentialNotFoundError`) for `instanceof` matching.

Two recovery paths are common enough to warrant runnable examples.

Flip to sign-in when register hits an existing credential (`AlreadyExists`):

{{#tabs passkey:recover-already-exists}}

Show a sticky retry UI when the OS biometric timeout fires (`Timeout`):

{{#tabs passkey:handle-timeout}}

For the full mapping (including the iOS sub-300ms fast-fail nuance that bundles "no-credential" under `UserCancelled`), see the [UX guide](./uxguide_passkey.md).

## Supported specs

- [Seedless Restore](https://github.com/breez/seedless-restore): Passkey-based wallet derivation and discovery
- [Nostr](https://github.com/nostr-protocol/nostr): Relay-based event protocol for label storage
- [NIP-42](https://github.com/nostr-protocol/nips/blob/master/42.md): Authentication of clients to relays
- [NIP-65](https://github.com/nostr-protocol/nips/blob/master/65.md): Relay List Metadata
