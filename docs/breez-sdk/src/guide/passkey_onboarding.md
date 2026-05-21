# Onboarding

The recommended flow depends on the platform. Mobile uses a single-call unified flow backed by {{#name PasskeyClient.connect_with_passkey}}. Web uses two buttons (Sign In and Create Account) and lets the user pick. For explicit control over each path, call {{#name PasskeyClient.sign_in}} and {{#name PasskeyClient.register}} directly.

## Unified flow (mobile)

One "Use Passkey" button backed by {{#name PasskeyClient.connect_with_passkey}}: silent sign-in for a returning user, automatic fall-through to registration on a fresh device. The response's `registered_credential` field doubles as the path discriminator: `Some` (with the new credential metadata) on the register path, `None` on the sign-in path.

Internally the silent attempt pins `preferImmediatelyAvailableCredentials = true` so the OS fast-fails (no UI, sub-300ms on iOS / Android) when no local credential exists; only {{#enum PrfProviderError::CredentialNotFound}} flips to register, all other errors (`Cancel`, `Timeout`, `Configuration`) propagate unchanged.

{{#tabs passkey:connect-with-passkey}}

## Two-button flow (web)

`connect_with_passkey` is not surfaced on the WASM target. The unified flow needs a silent "no credential here" signal so it can fall through to register, and WebAuthn deliberately collapses that case into the same `NotAllowedError` as a user cancel for privacy reasons. There is no reliable way on web for the SDK to tell the two apart.

The recommended UX on web is two buttons: a **Sign In** button calling `signIn` and a separate **Create Account** button calling `register`. Let the user pick the right one instead of trying to auto-detect. See [Direct sign-in / register](#direct-sign-in--register) for the call shapes.

## Direct sign-in / register

For finer control, call {{#name PasskeyClient.sign_in}} and {{#name PasskeyClient.register}} directly. Use this path when separating Sign-In and Create-Account flows, or when adding a new label on a returning user without going through `connect_with_passkey`.

Sign in to an existing credential:

{{#tabs passkey:sign-in}}

Register a fresh credential:

{{#tabs passkey:register-passkey}}

Pass `wallet.seed` to {{#name connect}} in either case.

## Error recovery

The SDK collapses every passkey failure into seven actionable [`ErrorKind`](https://breez.github.io/spark-sdk/breez_sdk_spark/passkey/enum.ErrorKind.html) values: branching on `error.kind()` is the canonical recovery pattern:

| `ErrorKind` | What it means | Recommended action |
|---|---|---|
| `Cancel` | User dismissed the OS prompt (≥ 300ms, < 55s) | Sticky retry UI with "Try Again" |
| `NoCredential` | No matching credential on this device (includes iOS sub-300ms fast-fail) | Fall through to {{#name PasskeyClient.register}} |
| `AlreadyExists` | Register hit a credential in `excludeCredentialIds` | Flip to {{#name PasskeyClient.sign_in}}; the OS picker surfaces the existing credential |
| `Timeout` | OS biometric inactivity timeout (≥ 55s): distinct from a user-cancel | Sticky retry with timeout-specific copy. **Do not** auto-retry |
| `PrfUnsupported` | Authenticator doesn't implement the PRF extension | Fall back to mnemonic onboarding |
| `Configuration` | Entitlement missing, AASA stale, or assetlinks malformed | Developer-facing error; surface {{#name check_domain_association}}'s `NotAssociated` reason |
| `Internal` | Network / library / generic failure | Generic "try again later" UI |

Web also exposes typed exception classes (`PasskeyAlreadyExistsError`, `PasskeyTimedOutError`, `PasskeyCredentialNotFoundError`) as an alternative to `error.kind()` for `instanceof` matching.

Two recovery paths are common enough to warrant runnable examples.

Flip to sign-in when register hits an existing credential (`AlreadyExists`):

{{#tabs passkey:recover-already-exists}}

Show a sticky retry UI when the OS biometric timeout fires (`Timeout`):

{{#tabs passkey:handle-timeout}}

For the full mapping (including the iOS sub-300ms fast-fail nuance that bundles "no-credential" under `UserCancelled`), see the [UX guide](./uxguide_passkey.md).
