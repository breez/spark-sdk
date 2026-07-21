# Onboarding

Initialize the {{#name PasskeyClient}}, then run the onboarding flow that fits your platform.

## Initialization

{{#name PasskeyClient}} is the entry point for every passkey wallet operation. Construct one per app session and reuse it.

On web, iOS, Android, Flutter, and React Native it wires the built-in {{#name PasskeyProvider}} for you, defaulting to the Breez shared RP (`keys.breez.technology`): a Breez-registered app needs only its Breez API key. Set {{#name provider_options}} on the config to use your own RP or customize the picker identity. On other platforms, or for a custom PRF backend (hardware key, file-backed), implement {{#name PrfProvider}} and inject it:

{{#tabs passkey:setup-client}}

**Parameters:**

| Parameter | Default | Description |
|---|---|---|
| {{#name breez_api_key}} | **required** | Your Breez API key, used to authenticate to the Breez relay for label storage. |
| {{#name default_label}} | `"Default"` | Wallet label used when {{#name PasskeyClient.register}} / {{#name PasskeyClient.sign_in}} receive none. Set on {{#name passkey_config}}. |

Configure the built-in provider through {{#name provider_options}} on {{#name passkey_config}} (a {{#name PasskeyProviderOptions}}):

| Field | Default | Description |
|---|---|---|
| {{#name rp_id}} | Breez shared RP | Relying Party ID: your app's domain, or unset for the Breez shared RP (`keys.breez.technology`) if your app is Breez-registered. Changing it later strands existing credentials. |
| {{#name rp_name}} | `"Breez"` | Display name for your app, shown in some authenticator UIs. |
| {{#name user_name}} | {{#name rp_name}} | Account identifier the OS sign-in picker shows beneath the display name, e.g. `john@doe.com`. Set a stable per-user value to keep each registration a distinct entry. |
| {{#name user_display_name}} | {{#name user_name}} | Human-friendly name the picker shows most prominently, e.g. `John Doe`. |

For platform-specific provider options (iOS `URLSession` / presentation anchor, Android `Activity`, web `authenticatorAttachment`) or a custom PRF backend, build the provider yourself and inject it. See [PRF providers](./passkey_prf_providers.md).

### Checking passkey availability

Call {{#name PasskeyClient.check_availability}} before showing the passkey button. One call covers device support and your domain config, so you can hide the option on unsupported devices (older Android / iOS) or surface a configuration error (missing entitlement, undeployed AASA) before the user runs into an opaque WebAuthn failure.

{{#tabs passkey:check-availability}}

## Choosing a flow

The right flow depends on the platform:

- **iOS / Android** use a single-call unified flow backed by {{#name PasskeyClient.connect_with_passkey}}.
- **Web** uses two buttons ("Create a new passkey" and "Sign in with a passkey") and lets the user pick.

For explicit control over each path, call {{#name PasskeyClient.sign_in}} and {{#name PasskeyClient.register}} directly.

### Unified flow (iOS / Android)

One "Use Passkey" button: a silent sign-in for returning users, with automatic fall-through to registration on a fresh device.

The response's {{#name credential}} field carries whichever credential signed in or was registered. See [Credential metadata](./passkey_credential_metadata.md) for using it.

{{#tabs passkey:connect-with-passkey}}

### Two-button flow (Web)

On web, present two buttons: **Create a new passkey** (calls {{#name PasskeyClient.register}}) and **Sign in with a passkey** (calls {{#name PasskeyClient.sign_in}}).

{{#name PasskeyClient.connect_with_passkey}} is not available on the WASM target. WebAuthn reports "no credential" and "user cancelled" identically, so the SDK can't auto-detect the flow. Let the user choose.

### Sign in and register

Call {{#name PasskeyClient.sign_in}} and {{#name PasskeyClient.register}} directly for explicit control: the two web buttons, separate create-a-passkey and sign-in screens, or adding a new label for a returning user. Pass `wallet.seed` to {{#name connect}} in either case.

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
| {{#enum PrfProviderError::UserCancelled}} | User dismissed the OS prompt | Sticky retry UI with "Try Again". |
| {{#enum PrfProviderError::CredentialNotFound}} | No matching credential on this device | Fall through to {{#name PasskeyClient.register}}. |
| {{#enum PrfProviderError::CredentialAlreadyExists}} | Register hit a credential in {{#name exclude_credentials}} | Flip to {{#name PasskeyClient.sign_in}}; the OS picker surfaces the existing credential. |
| {{#enum PrfProviderError::UserTimedOut}} | OS biometric inactivity timeout, distinct from a cancel | Sticky retry with timeout-specific copy. **Do not** auto-retry. |
| {{#enum PrfProviderError::PrfNotSupported}} | Authenticator lacks the PRF extension | Fall back to mnemonic onboarding. |
| {{#enum PrfProviderError::Configuration}} | Entitlement missing, AASA stale, or assetlinks malformed | Developer-facing error; surface the {{#enum PasskeyAvailability::NotAssociated}} reason. |
| {{#enum PrfProviderError::Generic}} | Network or generic failure | Generic "try again later" UI. |

Web exposes typed exception classes (`PasskeyAlreadyExistsError`, `PasskeyTimedOutError`, `PasskeyCredentialNotFoundError`) for `instanceof` matching. Rust callers can branch on the collapsed `error.kind()` instead of every variant.

Two recovery paths are common enough to show in full.

Flip to sign-in when register hits an existing credential:

{{#tabs passkey:recover-already-exists}}

Show a sticky retry when the biometric timeout fires:

{{#tabs passkey:handle-timeout}}

See the [UX guide](./uxguide_login.md) for the recommended recovery UX.

## Supported specs

- [Seedless Restore](https://github.com/breez/seedless-restore): passkey-based wallet derivation and discovery
- [Nostr](https://github.com/nostr-protocol/nostr): relay-based event protocol for label storage
- [NIP-42](https://github.com/nostr-protocol/nips/blob/master/42.md): authentication of clients to relays
- [NIP-65](https://github.com/nostr-protocol/nips/blob/master/65.md): relay list metadata
