# Onboarding

Initialize the `PasskeyClient`, then run the onboarding flow that fits your platform.

## Initialization

`PasskeyClient` is the entry point for every passkey wallet operation. Construct one per app session and reuse it.

On web, iOS, Android, Flutter, and React Native it wires the built-in `PasskeyProvider` for you, defaulting to the Breez shared RP (`keys.breez.technology`): a Breez-registered app needs only its Breez API key. Set `provider_options` on the config to use your own RP or customize the picker identity. On other platforms, or for a custom PRF backend (hardware key, file-backed), implement `PrfProvider` and inject it:

```python
prf_provider = CustomPrfProvider()
passkey = PasskeyClient(prf_provider, "<breez api key>", None)
```



**Parameters:**

| Parameter | Default | Description |
|---|---|---|
| `breez_api_key` | **required** | Your Breez API key, used to authenticate to the Breez relay for label storage. |
| `default_label` | `"Default"` | Wallet label used when `PasskeyClient.register` / `PasskeyClient.sign_in` receive none. Set on `passkey_config`. |

Configure the built-in provider through `provider_options` on `passkey_config` (a `PasskeyProviderOptions`):

| Field | Default | Description |
|---|---|---|
| `rp_id` | Breez shared RP | Relying Party ID: your app's domain, or unset for the Breez shared RP (`keys.breez.technology`) if your app is Breez-registered. Changing it later strands existing credentials. |
| `rp_name` | `"Breez"` | Display name for your app, shown in some authenticator UIs. |
| `user_name` | `rp_name` | Account identifier the OS sign-in picker shows beneath the display name, e.g. `john@doe.com`. Set a stable per-user value to keep each registration a distinct entry. |
| `user_display_name` | `user_name` | Human-friendly name the picker shows most prominently, e.g. `John Doe`. |

For platform-specific provider options (iOS `URLSession` / presentation anchor, Android `Activity`, web `authenticatorAttachment`) or a custom PRF backend, build the provider yourself and inject it. See [PRF providers](./passkey_prf_providers.md).

### Checking passkey availability

Call `PasskeyClient.check_availability` before showing the passkey button. One call covers device support and your domain config, so you can hide the option on unsupported devices (older Android / iOS) or surface a configuration error (missing entitlement, undeployed AASA) before the user runs into an opaque WebAuthn failure.

```python
availability = await passkey.check_availability()
if isinstance(availability, PasskeyAvailability.AVAILABLE):
    # Show passkey as primary option.
    pass
elif isinstance(availability, PasskeyAvailability.PRF_UNSUPPORTED):
    # Fall back to mnemonic flow.
    pass
elif isinstance(availability, PasskeyAvailability.NOT_ASSOCIATED):
    print(f"Domain association failed (source={availability.source}): {availability.reason}")
elif isinstance(availability, PasskeyAvailability.SKIPPED):
    # No verification source on this platform; proceed normally.
    pass
```



## Choosing a flow

The right flow depends on the platform:

- **iOS / Android** use a single-call unified flow backed by `PasskeyClient.connect_with_passkey`.
- **Web** uses the same unified flow where the browser supports immediate mediation, and two buttons ("Create a new passkey" / "Sign in with a passkey") otherwise.

For explicit control over each path, call `PasskeyClient.sign_in` and `PasskeyClient.register` directly.

### Unified flow (iOS / Android)

One "Use Passkey" button: a silent sign-in for returning users, with automatic fall-through to registration on a fresh device.

The response's `credential` field carries whichever credential signed in or was registered. See [Credential metadata](./passkey_credential_metadata.md) for using it.

Call it without a label to support multiple wallets per passkey: `labels` then holds the returning user's full set (the response wallet is the default label). Show a picker when it has more than one entry and `PasskeyClient.sign_in` to the chosen label.

```python
# Silent sign-in for a returning user, fall-through to register on a fresh device.
# No label: derive the default wallet and discover this passkey's label set.
response = await passkey.connect_with_passkey(ConnectWithPasskeyRequest())

if len(response.labels) > 1:
    # Returning multi-wallet user: let them pick a label and sign in to it.
    # chosen = ...  # prompt the user with response.labels
    # response = await passkey.sign_in(SignInRequest(label=chosen))
    pass

config = default_config(network=Network.MAINNET)
sdk = await connect(
    ConnectRequest(config=config, seed=response.wallet.seed, storage_dir="./.data")
)
```



### Web flow

`PasskeyClient.connect_with_passkey` works on web too, **where the browser supports immediate mediation** (recent Chromium). Check `PasskeyClient.supports_immediate_mediation` and use the same single-button unified flow.

Where it isn't supported (Safari, Firefox, older browsers), present two buttons: **Create a new passkey** (calls `PasskeyClient.register`) and **Sign in with a passkey** (calls `PasskeyClient.sign_in`). Without immediate mediation, WebAuthn reports "no credential" and "user cancelled" identically, so the SDK can't auto-detect the flow.

### Sign in and register

Call `PasskeyClient.sign_in` and `PasskeyClient.register` directly for explicit control: the two web buttons, separate create-a-passkey and sign-in screens, or adding a new label for a returning user. Pass `wallet.seed` to `connect` in either case.

#### Sign in

Sign in to an existing credential:



#### Register

Register a fresh credential:

```python
response = await passkey.register(RegisterRequest(label="personal"))

config = default_config(network=Network.MAINNET)
sdk = await connect(
    ConnectRequest(config=config, seed=response.wallet.seed, storage_dir="./.data")
)
```



## Error recovery

Most passkey failures normalize to a `PrfProviderError` variant. Match on the variant to drive recovery:

| Variant | What it means | Recommended action |
|---|---|---|
| `PrfProviderError.USER_CANCELLED` | User dismissed the OS prompt | Sticky retry UI with "Try Again". |
| `PrfProviderError.CREDENTIAL_NOT_FOUND` | No matching credential on this device | Fall through to `PasskeyClient.register`. |
| `PrfProviderError.CREDENTIAL_ALREADY_EXISTS` | Register hit a credential in `exclude_credentials` | Flip to `PasskeyClient.sign_in`; the OS picker surfaces the existing credential. |
| `PrfProviderError.USER_TIMED_OUT` | OS biometric inactivity timeout, distinct from a cancel | Sticky retry with timeout-specific copy. **Do not** auto-retry. |
| `PrfProviderError.PRF_NOT_SUPPORTED` | Authenticator lacks the PRF extension | Fall back to mnemonic onboarding. |
| `PrfProviderError.CONFIGURATION` | Entitlement missing, AASA stale, or assetlinks malformed | Developer-facing error; surface the `PasskeyAvailability.NOT_ASSOCIATED` reason. |
| `PrfProviderError.GENERIC` | Network or generic failure | Generic "try again later" UI. |

Those rows cover `PasskeyClient.sign_in`, and
`PasskeyClient.register` up to the point the credential is created.
Once it exists, a failure from the authenticator arrives as the variant below
instead: a cancel, a timeout or an unsupported authenticator during the
derive no longer surfaces as its own `PrfProviderError`.

`PasskeyClient.register` has one failure of its own that is **not** a
`PrfProviderError`:

| Variant | What it means | Recommended action |
|---|---|---|
| `PasskeyError.CREATED_BUT_NOT_DERIVED` | The passkey was created, then the authenticator failed the derive that followed | Sign in pinned to the `credential_id` on the error. **Do not** register again. |

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

```python
try:
    await passkey.register(
        RegisterRequest(
            label="personal",
            exclude_credentials=[
                # app-persisted credential IDs from prior registrations
            ],
        )
    )
except PrfProviderError.CredentialAlreadyExists:
    # A matching credential already exists; sign in to it instead.
    response = await passkey.sign_in(SignInRequest(label="personal"))
    return response.wallet
```



Show a sticky retry when the biometric timeout fires:

```python
# Biometric inactivity timeout, distinct from a user cancel.
try:
    return await passkey.sign_in(SignInRequest(label="personal"))
except PrfProviderError.UserTimedOut:
    # Show a retry UI. Do NOT auto-retry without user input.
    print("Sign-in timed out: show \"Try Again\" UI.")
    raise
```



See the [UX guide](./uxguide_login.md) for the recommended recovery UX.

## Supported specs

- [Seedless Restore](https://github.com/breez/seedless-restore): passkey-based wallet derivation and discovery
- [Nostr](https://github.com/nostr-protocol/nostr): relay-based event protocol for label storage
- [NIP-42](https://github.com/nostr-protocol/nips/blob/master/42.md): authentication of clients to relays
- [NIP-65](https://github.com/nostr-protocol/nips/blob/master/65.md): relay list metadata
