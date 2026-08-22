# Onboarding

Initialize the `PasskeyClient`, then run the onboarding flow that fits your platform.

## Initialization

`PasskeyClient` is the entry point for every passkey wallet operation. Construct one per app session and reuse it.

On web, iOS, Android, Flutter, and React Native it wires the built-in `PasskeyProvider` for you, defaulting to the Breez shared RP (`keys.breez.technology`): a Breez-registered app needs only its Breez API key. Set `provider_options` on the config to use your own RP or customize the picker identity. On other platforms, or for a custom PRF backend (hardware key, file-backed), implement `PrfProvider` and inject it:

```rust
let prf_provider = Arc::new(CustomPrfProvider);
PasskeyClient::new(prf_provider, Some("<breez api key>".to_string()), None)
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

```rust
match passkey.check_availability().await? {
    PasskeyAvailability::Available => {
        // Passkey supported: proceed with connect_with_passkey. On web,
        // call PasskeyClient::supports_immediate_mediation to pick
        // single- vs two-button onboarding (native is always single).
    }
    PasskeyAvailability::PrfUnsupported => {
        // Fall back to mnemonic flow.
    }
    PasskeyAvailability::NotAssociated { source, reason } => {
        eprintln!("Domain association failed (source={source}): {reason}");
    }
    PasskeyAvailability::Skipped { reason: _ } => {
        // No verification source on this platform; proceed normally.
    }
}
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

```rust
// Single-CTA onboarding: silent sign-in, fall through to register.
// Without a label, a returning user's wallets are discovered in
// `response.labels` (the response wallet is the default); a new user
// gets a freshly registered default wallet.
let response = passkey
    .connect_with_passkey(ConnectWithPasskeyRequest::default())
    .await?;
if response.labels.len() > 1 {
    // Multiple wallets: let the user pick, then sign_in to the chosen label.
}

let config = default_config(Network::Mainnet);
let sdk = connect(ConnectRequest {
    config,
    seed: response.wallet.seed,
    storage_dir: "./.data".to_string(),
})
.await?;
```



### Web flow

`PasskeyClient.connect_with_passkey` works on web too, **where the browser supports immediate mediation** (recent Chromium). Check `PasskeyClient.supports_immediate_mediation` and use the same single-button unified flow.

Where it isn't supported (Safari, Firefox, older browsers), present two buttons: **Create a new passkey** (calls `PasskeyClient.register`) and **Sign in with a passkey** (calls `PasskeyClient.sign_in`). Without immediate mediation, WebAuthn reports "no credential" and "user cancelled" identically, so the SDK can't auto-detect the flow.

### Sign in and register

Call `PasskeyClient.sign_in` and `PasskeyClient.register` directly for explicit control: the two web buttons, separate create-a-passkey and sign-in screens, or adding a new label for a returning user. Pass `wallet.seed` to `connect` in either case.

#### Sign in

Sign in to an existing credential:

```rust
// Returning-user-only sign-in. No fall-through to register.
Ok(passkey
    .sign_in(SignInRequest {
        label: Some("personal".to_string()),
        ..Default::default()
    })
    .await?)
```



#### Register

Register a fresh credential:

```rust
let response = passkey
    .register(RegisterRequest {
        label: Some("personal".to_string()),
        ..Default::default()
    })
    .await?;

let config = default_config(Network::Mainnet);
let sdk = connect(ConnectRequest {
    config,
    seed: response.wallet.seed,
    storage_dir: "./.data".to_string(),
})
.await?;
```



## Error recovery

Most passkey failures normalize to a `PrfProviderError` variant. Match on the variant to drive recovery:

| Variant | What it means | Recommended action |
|---|---|---|
| `PrfProviderError::UserCancelled` | User dismissed the OS prompt | Sticky retry UI with "Try Again". |
| `PrfProviderError::CredentialNotFound` | No matching credential on this device | Fall through to `PasskeyClient.register`. |
| `PrfProviderError::CredentialAlreadyExists` | Register hit a credential in `exclude_credentials` | Flip to `PasskeyClient.sign_in`; the OS picker surfaces the existing credential. |
| `PrfProviderError::UserTimedOut` | OS biometric inactivity timeout, distinct from a cancel | Sticky retry with timeout-specific copy. **Do not** auto-retry. |
| `PrfProviderError::PrfNotSupported` | Authenticator lacks the PRF extension | Fall back to mnemonic onboarding. |
| `PrfProviderError::Configuration` | Entitlement missing, AASA stale, or assetlinks malformed | Developer-facing error; surface the `PasskeyAvailability::NotAssociated` reason. |
| `PrfProviderError::Generic` | Network or generic failure | Generic "try again later" UI. |

Those rows cover `PasskeyClient.sign_in`, and
`PasskeyClient.register` up to the point the credential is created.
Once it exists, a failure from the authenticator arrives as the variant below
instead: a cancel, a timeout or an unsupported authenticator during the
derive no longer surfaces as its own `PrfProviderError`.

`PasskeyClient.register` has one failure of its own that is **not** a
`PrfProviderError`:

| Variant | What it means | Recommended action |
|---|---|---|
| `PasskeyError::CreatedButNotDerived` | The passkey was created, then the authenticator failed the derive that followed | Sign in pinned to the `credential_id` on the error. **Do not** register again. |

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

```rust
match passkey
    .register(RegisterRequest {
        label: Some("personal".to_string()),
        exclude_credentials: Some(vec![
            // app-persisted credential IDs from prior registrations
        ]),
    })
    .await
{
    Ok(response) => Ok(response.wallet),
    Err(e) if e.kind() == ErrorKind::AlreadyExists => {
        // A matching credential already exists; sign in to it instead.
        let response = passkey
            .sign_in(SignInRequest {
                label: Some("personal".to_string()),
                ..Default::default()
            })
            .await?;
        Ok(response.wallet)
    }
    Err(e) => Err(e.into()),
}
```



Show a sticky retry when the biometric timeout fires:

```rust
// Biometric inactivity timeout, distinct from a user cancel.
match passkey
    .sign_in(SignInRequest {
        label: Some("personal".to_string()),
        ..Default::default()
    })
    .await
{
    Ok(response) => Ok(response),
    Err(e) if e.kind() == ErrorKind::Timeout => {
        // Show a retry UI. Do NOT auto-retry without user input.
        println!("Sign-in timed out: show \"Try Again\" UI.");
        Err(e.into())
    }
    Err(e) => Err(e.into()),
}
```



See the [UX guide](./uxguide_login.md) for the recommended recovery UX.

## Supported specs

- [Seedless Restore](https://github.com/breez/seedless-restore): passkey-based wallet derivation and discovery
- [Nostr](https://github.com/nostr-protocol/nostr): relay-based event protocol for label storage
- [NIP-42](https://github.com/nostr-protocol/nips/blob/master/42.md): authentication of clients to relays
- [NIP-65](https://github.com/nostr-protocol/nips/blob/master/65.md): relay list metadata
