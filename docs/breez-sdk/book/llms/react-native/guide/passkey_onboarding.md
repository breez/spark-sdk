# Onboarding

Initialize the `PasskeyClient`, then run the onboarding flow that fits your platform.

## Initialization

`PasskeyClient` is the entry point for every passkey wallet operation. Construct one per app session and reuse it.

On web, iOS, Android, Flutter, and React Native it wires the built-in `PasskeyProvider` for you, defaulting to the Breez shared RP (`keys.breez.technology`): a Breez-registered app needs only its Breez API key. Set `providerOptions` on the config to use your own RP or customize the picker identity. On other platforms, or for a custom PRF backend (hardware key, file-backed), implement `PrfProvider` and inject it:

```typescript
const passkey = new PasskeyClient(
  '<breez api key>',
  PasskeyConfig.create({
    providerOptions: PasskeyProviderOptions.create({ rpId: '<your-rp-domain>', rpName: 'Your App' })
  })
)
```



**Parameters:**

| Parameter | Default | Description |
|---|---|---|
| `breezApiKey` | **required** | Your Breez API key, used to authenticate to the Breez relay for label storage. |
| `defaultLabel` | `"Default"` | Wallet label used when `PasskeyClient.register` / `PasskeyClient.signIn` receive none. Set on `passkeyConfig`. |

Configure the built-in provider through `providerOptions` on `passkeyConfig` (a `PasskeyProviderOptions`):

| Field | Default | Description |
|---|---|---|
| `rpId` | Breez shared RP | Relying Party ID: your app's domain, or unset for the Breez shared RP (`keys.breez.technology`) if your app is Breez-registered. Changing it later strands existing credentials. |
| `rpName` | `"Breez"` | Display name for your app, shown in some authenticator UIs. |
| `userName` | `rpName` | Account identifier the OS sign-in picker shows beneath the display name, e.g. `john@doe.com`. Set a stable per-user value to keep each registration a distinct entry. |
| `userDisplayName` | `userName` | Human-friendly name the picker shows most prominently, e.g. `John Doe`. |

For platform-specific provider options (iOS `URLSession` / presentation anchor, Android `Activity`, web `authenticatorAttachment`) or a custom PRF backend, build the provider yourself and inject it. See [PRF providers](./passkey_prf_providers.md).

### Checking passkey availability

Call `PasskeyClient.checkAvailability` before showing the passkey button. One call covers device support and your domain config, so you can hide the option on unsupported devices (older Android / iOS) or surface a configuration error (missing entitlement, undeployed AASA) before the user runs into an opaque WebAuthn failure.

```typescript
const availability = await passkey.checkAvailability()
switch (availability.tag) {
  case PasskeyAvailability_Tags.Available:
    // Show passkey as primary option.
    break
  case PasskeyAvailability_Tags.PrfUnsupported:
    // Fall back to mnemonic flow.
    break
  case PasskeyAvailability_Tags.NotAssociated:
    console.error(
      `Domain association failed (source=${availability.inner.source}): ` +
      `${availability.inner.reason}`
    )
    break
  case PasskeyAvailability_Tags.Skipped:
    // No verification source on this platform; proceed normally.
    break
}
```



## Choosing a flow

The right flow depends on the platform:

- **iOS / Android** use a single-call unified flow backed by `PasskeyClient.connectWithPasskey`.
- **Web** uses the same unified flow where the browser supports immediate mediation, and two buttons ("Create a new passkey" / "Sign in with a passkey") otherwise.

For explicit control over each path, call `PasskeyClient.signIn` and `PasskeyClient.register` directly.

### Unified flow (iOS / Android)

One "Use Passkey" button: a silent sign-in for returning users, with automatic fall-through to registration on a fresh device.

The response's `credential` field carries whichever credential signed in or was registered. See [Credential metadata](./passkey_credential_metadata.md) for using it.

Call it without a label to support multiple wallets per passkey: `labels` then holds the returning user's full set (the response wallet is the default label). Show a picker when it has more than one entry and `PasskeyClient.signIn` to the chosen label.

```typescript
// Silent sign-in, fall through to register. No label: a returning user's
// wallets come back in `response.labels` (the default is signed in).
const config = { ...defaultConfig(Network.Mainnet), apiKey: '<breez api key>' }
const response = await passkey.connectWithPasskey({
  label: undefined,
  allowCredentials: undefined,
  excludeCredentials: undefined
})

if (response.labels.length > 1) {
  // Returning multi-wallet user: let them pick a label and sign in to it.
  // const chosen = await pickLabel(response.labels)
  // return await passkey.signIn({ label: chosen, allowCredentials: undefined, preferImmediatelyAvailableCredentials: undefined })
}

const sdk = await connect({ config, seed: response.wallet.seed, storageDir: './.data' })
```



### Web flow

`PasskeyClient.connectWithPasskey` works on web too, **where the browser supports immediate mediation** (recent Chromium). Check `PasskeyClient.supportsImmediateMediation` and use the same single-button unified flow.

Where it isn't supported (Safari, Firefox, older browsers), present two buttons: **Create a new passkey** (calls `PasskeyClient.register`) and **Sign in with a passkey** (calls `PasskeyClient.signIn`). Without immediate mediation, WebAuthn reports "no credential" and "user cancelled" identically, so the SDK can't auto-detect the flow.

### Sign in and register

Call `PasskeyClient.signIn` and `PasskeyClient.register` directly for explicit control: the two web buttons, separate create-a-passkey and sign-in screens, or adding a new label for a returning user. Pass `wallet.seed` to `connect` in either case.

#### Sign in

Sign in to an existing credential:

```typescript
// Returning-user sign-in. No fall-through to register.
return await passkey.signIn({
  label: 'personal',
  allowCredentials: undefined,
  preferImmediatelyAvailableCredentials: undefined
})
```



#### Register

Register a fresh credential:

```typescript
const config = { ...defaultConfig(Network.Mainnet), apiKey: '<breez api key>' }
const response = await passkey.register({ label: 'personal', excludeCredentials: undefined })

const sdk = await connect({ config, seed: response.wallet.seed, storageDir: './.data' })
```



## Error recovery

Most passkey failures normalize to a `PrfProviderError` variant. Match on the variant to drive recovery:

| Variant | What it means | Recommended action |
|---|---|---|
| `PrfProviderError.UserCancelled` | User dismissed the OS prompt | Sticky retry UI with "Try Again". |
| `PrfProviderError.CredentialNotFound` | No matching credential on this device | Fall through to `PasskeyClient.register`. |
| `PrfProviderError.CredentialAlreadyExists` | Register hit a credential in `excludeCredentials` | Flip to `PasskeyClient.signIn`; the OS picker surfaces the existing credential. |
| `PrfProviderError.UserTimedOut` | OS biometric inactivity timeout, distinct from a cancel | Sticky retry with timeout-specific copy. **Do not** auto-retry. |
| `PrfProviderError.PrfNotSupported` | Authenticator lacks the PRF extension | Fall back to mnemonic onboarding. |
| `PrfProviderError.Configuration` | Entitlement missing, AASA stale, or assetlinks malformed | Developer-facing error; surface the `PasskeyAvailability.NotAssociated` reason. |
| `PrfProviderError.Generic` | Network or generic failure | Generic "try again later" UI. |

Those rows cover `PasskeyClient.signIn`, and
`PasskeyClient.register` up to the point the credential is created.
Once it exists, a failure from the authenticator arrives as the variant below
instead: a cancel, a timeout or an unsupported authenticator during the
derive no longer surfaces as its own `PrfProviderError`.

`PasskeyClient.register` has one failure of its own that is **not** a
`PrfProviderError`:

| Variant | What it means | Recommended action |
|---|---|---|
| `PasskeyError.CreatedButNotDerived` | The passkey was created, then the authenticator failed the derive that followed | Sign in pinned to the `credential_id` on the error. **Do not** register again. |

Handle it explicitly. The passkey exists on the device from that point on, so
registering again leaves the first one behind owning a wallet nothing points
to. A catch-all `else` branch will not fail to compile: it routes this into
your generic "try again" path, which is usually a retry that registers a
second passkey. It carries the underlying
`PrfProviderError` as `source`, so unwrap once and reuse the arms
above. Failures that are not the authenticator's (mnemonic, key derivation,
invalid PRF output) keep their own variant and are not wrapped.

Web exposes typed exception classes (`PasskeyAlreadyExistsError`, `PasskeyTimedOutError`, `PasskeyCredentialNotFoundError`) for `instanceof` matching. Rust callers can branch on the collapsed `error.kind()` instead of every variant.

Two recovery paths are common enough to show in full.

Flip to sign-in when register hits an existing credential:

```typescript
try {
  const response = await passkey.register({
    label: 'personal',
    excludeCredentials: [
      // app-persisted credential IDs from prior registrations
    ]
  })
  return response.wallet
} catch (error) {
  if (error instanceof PasskeyPrfException && error.code === 'credentialAlreadyExists') {
    // A matching credential already exists; sign in to it instead.
    const response = await passkey.signIn({
      label: 'personal',
      allowCredentials: undefined,
      preferImmediatelyAvailableCredentials: undefined
    })
    return response.wallet
  }
  throw error
}
```



Show a sticky retry when the biometric timeout fires:

```typescript
// Biometric inactivity timeout, distinct from a user cancel.
try {
  const response = await passkey.signIn({
    label: 'personal',
    allowCredentials: undefined,
    preferImmediatelyAvailableCredentials: undefined
  })
  return response
} catch (error) {
  if (error instanceof PasskeyPrfException && error.code === 'userTimedOut') {
    // Show a retry UI. Do NOT auto-retry without user input.
    console.log('Sign-in timed out: show "Try Again" UI.')
  }
  throw error
}
```



See the [UX guide](./uxguide_login.md) for the recommended recovery UX.

## Supported specs

- [Seedless Restore](https://github.com/breez/seedless-restore): passkey-based wallet derivation and discovery
- [Nostr](https://github.com/nostr-protocol/nostr): relay-based event protocol for label storage
- [NIP-42](https://github.com/nostr-protocol/nips/blob/master/42.md): authentication of clients to relays
- [NIP-65](https://github.com/nostr-protocol/nips/blob/master/65.md): relay list metadata
