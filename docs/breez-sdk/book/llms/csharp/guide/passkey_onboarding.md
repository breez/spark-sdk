# Onboarding

Initialize the `PasskeyClient`, then run the onboarding flow that fits your platform.

## Initialization

`PasskeyClient` is the entry point for every passkey wallet operation. Construct one per app session and reuse it.

On web, iOS, Android, Flutter, and React Native it wires the built-in `PasskeyProvider` for you, defaulting to the Breez shared RP (`keys.breez.technology`): a Breez-registered app needs only its Breez API key. Set `ProviderOptions` on the config to use your own RP or customize the picker identity. On other platforms, or for a custom PRF backend (hardware key, file-backed), implement `PrfProvider` and inject it:

```csharp
var prfProvider = new CustomPrfProvider();
return new PasskeyClient(prfProvider, "<breez api key>", null);
```



**Parameters:**

| Parameter | Default | Description |
|---|---|---|
| `BreezApiKey` | **required** | Your Breez API key, used to authenticate to the Breez relay for label storage. |
| `DefaultLabel` | `"Default"` | Wallet label used when `PasskeyClient.Register` / `PasskeyClient.SignIn` receive none. Set on `PasskeyConfig`. |

Configure the built-in provider through `ProviderOptions` on `PasskeyConfig` (a `PasskeyProviderOptions`):

| Field | Default | Description |
|---|---|---|
| `RpId` | Breez shared RP | Relying Party ID: your app's domain, or unset for the Breez shared RP (`keys.breez.technology`) if your app is Breez-registered. Changing it later strands existing credentials. |
| `RpName` | `"Breez"` | Display name for your app, shown in some authenticator UIs. |
| `UserName` | `RpName` | Account identifier the OS sign-in picker shows beneath the display name, e.g. `john@doe.com`. Set a stable per-user value to keep each registration a distinct entry. |
| `UserDisplayName` | `UserName` | Human-friendly name the picker shows most prominently, e.g. `John Doe`. |

For platform-specific provider options (iOS `URLSession` / presentation anchor, Android `Activity`, web `authenticatorAttachment`) or a custom PRF backend, build the provider yourself and inject it. See [PRF providers](./passkey_prf_providers.md).

### Checking passkey availability

Call `PasskeyClient.CheckAvailability` before showing the passkey button. One call covers device support and your domain config, so you can hide the option on unsupported devices (older Android / iOS) or surface a configuration error (missing entitlement, undeployed AASA) before the user runs into an opaque WebAuthn failure.

```csharp
switch (await passkey.CheckAvailability())
{
    case PasskeyAvailability.Available:
        break;
    case PasskeyAvailability.PrfUnsupported:
        break;
    case PasskeyAvailability.NotAssociated notAssociated:
        Console.WriteLine($"Domain association failed (source={notAssociated.source}): " +
                          $"{notAssociated.reason}");
        break;
    case PasskeyAvailability.Skipped:
        break;
}
```



## Choosing a flow

The right flow depends on the platform:

- **iOS / Android** use a single-call unified flow backed by `PasskeyClient.ConnectWithPasskey`.
- **Web** uses the same unified flow where the browser supports immediate mediation, and two buttons ("Create a new passkey" / "Sign in with a passkey") otherwise.

For explicit control over each path, call `PasskeyClient.SignIn` and `PasskeyClient.Register` directly.

### Unified flow (iOS / Android)

One "Use Passkey" button: a silent sign-in for returning users, with automatic fall-through to registration on a fresh device.

The response's `Credential` field carries whichever credential signed in or was registered. See [Credential metadata](./passkey_credential_metadata.md) for using it.

Call it without a label to support multiple wallets per passkey: `Labels` then holds the returning user's full set (the response wallet is the default label). Show a picker when it has more than one entry and `PasskeyClient.SignIn` to the chosen label.

```csharp
// Single-CTA onboarding: silent sign-in for a returning user,
// fall-through to register on a fresh device.
var response = await passkey.ConnectWithPasskey(
    new ConnectWithPasskeyRequest()
);

if (response.labels.Length > 1)
{
    // Returning multi-wallet user: let them pick a label, then
    // SignIn to the chosen wallet.
}

var config = BreezSdkSparkMethods.DefaultConfig(network: Network.Mainnet);
var sdk = await BreezSdkSparkMethods.Connect(new ConnectRequest(
    config: config,
    seed: response.wallet.seed,
    storageDir: "./.data"
));
```



### Web flow

`PasskeyClient.ConnectWithPasskey` works on web too, **where the browser supports immediate mediation** (recent Chromium). Check `PasskeyClient.SupportsImmediateMediation` and use the same single-button unified flow.

Where it isn't supported (Safari, Firefox, older browsers), present two buttons: **Create a new passkey** (calls `PasskeyClient.Register`) and **Sign in with a passkey** (calls `PasskeyClient.SignIn`). Without immediate mediation, WebAuthn reports "no credential" and "user cancelled" identically, so the SDK can't auto-detect the flow.

### Sign in and register

Call `PasskeyClient.SignIn` and `PasskeyClient.Register` directly for explicit control: the two web buttons, separate create-a-passkey and sign-in screens, or adding a new label for a returning user. Pass `wallet.seed` to `Connect` in either case.

#### Sign in

Sign in to an existing credential:



#### Register

Register a fresh credential:

```csharp
var response = await passkey.Register(new RegisterRequest(label: "personal"));

var config = BreezSdkSparkMethods.DefaultConfig(network: Network.Mainnet);
var sdk = await BreezSdkSparkMethods.Connect(new ConnectRequest(
    config: config,
    seed: response.wallet.seed,
    storageDir: "./.data"
));
```



## Error recovery

Most passkey failures normalize to a `PrfProviderError` variant. Match on the variant to drive recovery:

| Variant | What it means | Recommended action |
|---|---|---|
| `PrfProviderError.UserCancelled` | User dismissed the OS prompt | Sticky retry UI with "Try Again". |
| `PrfProviderError.CredentialNotFound` | No matching credential on this device | Fall through to `PasskeyClient.Register`. |
| `PrfProviderError.CredentialAlreadyExists` | Register hit a credential in `ExcludeCredentials` | Flip to `PasskeyClient.SignIn`; the OS picker surfaces the existing credential. |
| `PrfProviderError.UserTimedOut` | OS biometric inactivity timeout, distinct from a cancel | Sticky retry with timeout-specific copy. **Do not** auto-retry. |
| `PrfProviderError.PrfNotSupported` | Authenticator lacks the PRF extension | Fall back to mnemonic onboarding. |
| `PrfProviderError.Configuration` | Entitlement missing, AASA stale, or assetlinks malformed | Developer-facing error; surface the `PasskeyAvailability.NotAssociated` reason. |
| `PrfProviderError.Generic` | Network or generic failure | Generic "try again later" UI. |

Those rows cover `PasskeyClient.SignIn`, and
`PasskeyClient.Register` up to the point the credential is created.
Once it exists, a failure from the authenticator arrives as the variant below
instead: a cancel, a timeout or an unsupported authenticator during the
derive no longer surfaces as its own `PrfProviderError`.

`PasskeyClient.Register` has one failure of its own that is **not** a
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

```csharp
try
{
    var response = await passkey.Register(new RegisterRequest(
        label: "personal",
        excludeCredentials: new byte[][]
        {
            // app-persisted credential IDs from prior registrations
        }
    ));
    return response.wallet;
}
catch (PrfProviderException.CredentialAlreadyExists)
{
    var response = await passkey.SignIn(new SignInRequest(label: "personal"));
    return response.wallet;
}
```



Show a sticky retry when the biometric timeout fires:

```csharp
try
{
    return await passkey.SignIn(new SignInRequest(label: "personal"));
}
catch (PrfProviderException.UserTimedOut)
{
    Console.WriteLine("Sign-in timed out: show \"Try Again\" UI.");
    throw;
}
```



See the [UX guide](./uxguide_login.md) for the recommended recovery UX.

## Supported specs

- [Seedless Restore](https://github.com/breez/seedless-restore): passkey-based wallet derivation and discovery
- [Nostr](https://github.com/nostr-protocol/nostr): relay-based event protocol for label storage
- [NIP-42](https://github.com/nostr-protocol/nips/blob/master/42.md): authentication of clients to relays
- [NIP-65](https://github.com/nostr-protocol/nips/blob/master/65.md): relay list metadata
