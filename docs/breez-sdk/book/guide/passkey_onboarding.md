# Onboarding

Initialize the `PasskeyClient`, then run the onboarding flow that fits your platform.

## Initialization

`PasskeyClient` is the entry point for every passkey wallet operation. Construct one per app session and reuse it.

On web, iOS, Android, Flutter, and React Native it wires the built-in `PasskeyProvider` for you, defaulting to the Breez shared RP (`keys.breez.technology`): a Breez-registered app needs only its Breez API key. Set `provider_options` on the config to use your own RP or customize the picker identity. On other platforms, or for a custom PRF backend (hardware key, file-backed), implement `PrfProvider` and inject it:

### Rust

```rust
let prf_provider = Arc::new(CustomPrfProvider);
PasskeyClient::new(prf_provider, Some("<breez api key>".to_string()), None)
```

### Swift

```swift
let passkey = PasskeyClient(
    breezApiKey: "<breez api key>",
    config: PasskeyConfig(
        providerOptions: PasskeyProviderOptions(rpId: "<your-rp-domain>", rpName: "Your App")
    )
)
```

### Kotlin

```kotlin
val passkey = PasskeyClient(
    breezApiKey = "<breez api key>",
    activityProvider = { activity },
    config = PasskeyConfig(
        providerOptions = PasskeyProviderOptions(rpId = "<your-rp-domain>", rpName = "Your App"),
    ),
)
```

### C#

```csharp
var prfProvider = new CustomPrfProvider();
return new PasskeyClient(prfProvider, "<breez api key>", null);
```

### Javascript (Wasm)

```typescript
const passkey = new PasskeyClient('<breez api key>', {
  providerOptions: { rpId: '<your-rp-domain>', rpName: 'Your App' }
})
```

### React Native

```typescript
const passkey = new PasskeyClient(
  '<breez api key>',
  PasskeyConfig.create({
    providerOptions: PasskeyProviderOptions.create({ rpId: '<your-rp-domain>', rpName: 'Your App' })
  })
)
```

### Flutter

```dart
final passkey = PasskeyClient(
  breezApiKey: '<breez api key>',
  config: PasskeyConfig(
    providerOptions: PasskeyProviderOptions(rpId: '<your-rp-domain>', rpName: 'Your App'),
  ),
);
```

### Python

```python
prf_provider = CustomPrfProvider()
passkey = PasskeyClient(prf_provider, "<breez api key>", None)
```

### Go

```go
prfProvider := &CustomPrfProvider{}
apiKey := "<breez api key>"
return breez_sdk_spark.NewPasskeyClient(prfProvider, &apiKey, nil)
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

#### Rust

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

#### Swift

```swift
switch try await passkey.checkAvailability() {
case .available:
    // Show passkey as primary option.
    break
case .prfUnsupported:
    // Fall back to mnemonic flow.
    break
case .notAssociated(let source, let reason):
    print("Domain association failed (source=\(source)): \(reason)")
case .skipped:
    // No verification source on this platform; proceed normally.
    break
}
```

#### Kotlin

```kotlin
when (val availability = passkey.checkAvailability()) {
    is PasskeyAvailability.Available -> Unit
    is PasskeyAvailability.PrfUnsupported -> Unit
    is PasskeyAvailability.NotAssociated -> {
        // Log.e("Breez", "Domain association failed
        // (source=${availability.source}): ${availability.reason}")
    }
    is PasskeyAvailability.Skipped -> Unit
}
```

#### C#

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

#### Javascript (Wasm)

```typescript
const availability = await passkey.checkAvailability()
switch (availability.type) {
  case 'available':
    // Show passkey as primary option.
    break
  case 'prfUnsupported':
    // Fall back to mnemonic flow.
    break
  case 'notAssociated':
    console.error(
      `Domain association failed (source=${availability.source}): ${availability.reason}`
    )
    break
  case 'skipped':
    // No verification source on this platform; proceed normally.
    break
}
```

#### React Native

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

#### Flutter

```dart
final availability = await passkey.checkAvailability();
if (availability is PasskeyAvailability_Available) {
  // Show passkey as primary option.
} else if (availability is PasskeyAvailability_PrfUnsupported) {
  // Fall back to mnemonic flow.
} else if (availability is PasskeyAvailability_NotAssociated) {
  print("Domain association failed (source=${availability.source}): ${availability.reason}");
} else if (availability is PasskeyAvailability_Skipped) {
  // No verification source on this platform; proceed normally.
}
```

#### Python

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

#### Go

```go
availability, err := passkey.CheckAvailability()
if err != nil {
	return
}
switch r := availability.(type) {
case breez_sdk_spark.PasskeyAvailabilityAvailable:
	// Show passkey as primary option.
	_ = r
case breez_sdk_spark.PasskeyAvailabilityPrfUnsupported:
	// Fall back to mnemonic flow.
	_ = r
case breez_sdk_spark.PasskeyAvailabilityNotAssociated:
	log.Printf("Domain association failed (source=%s): %s", r.Source, r.Reason)
case breez_sdk_spark.PasskeyAvailabilitySkipped:
	// No verification source on this platform; proceed normally.
	_ = r
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

#### Rust

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

#### Swift

```swift
// Single-CTA onboarding: silent sign-in, fall through to register.
var config = defaultConfig(network: .mainnet)
config.apiKey = "<breez api key>"

let response = try await passkey.connectWithPasskey(
    request: ConnectWithPasskeyRequest()
)

if response.labels.count > 1 {
    // Returning multi-wallet user: let them pick a label, then sign in to it.
    // let chosen = promptForLabel(response.labels)
    // return try await passkey.signIn(request: SignInRequest(label: chosen))
}

let sdk = try await connect(
    request: ConnectRequest(
        config: config,
        seed: response.wallet.seed,
        storageDir: "./.data"
    ))
```

#### Kotlin

```kotlin
// Single-CTA onboarding: silent sign-in, fall through to register.
val config = defaultConfig(Network.MAINNET).apply { apiKey = "<breez api key>" }
val response = passkey.connectWithPasskey(ConnectWithPasskeyRequest())

if (response.labels.size > 1) {
    // Returning multi-wallet user: let them pick a label and sign in to it.
    // val chosen = promptForLabel(response.labels)
    // return connect(ConnectRequest(config, passkey.signIn(SignInRequest(label = chosen)).wallet.seed, "./.data"))
}

val sdk = connect(ConnectRequest(config, response.wallet.seed, "./.data"))
```

#### C#

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

#### Javascript (Wasm)

```typescript
// Single-button flow. On web it works only where the browser supports
// immediate mediation; supportsImmediateMediation() reports it. Otherwise
// use the two-button flow (register / signIn).
const availability = await passkey.checkAvailability()
if (availability.type !== 'available' || !(await passkey.supportsImmediateMediation())) {
  throw new Error('Use the two-button flow (register / signIn) on this browser')
}
// No label: a returning user's wallets are discovered (response.labels,
// with wallet being the default); a new user gets a freshly registered
// default wallet.
const response = await passkey.connectWithPasskey({})
if (response.labels.length > 1) {
  // Multiple wallets: let the user pick, then signIn to the chosen label.
}

const config = defaultConfig('mainnet')
const sdk = await connect({ config, seed: response.wallet.seed, storageDir: './.data' })
```

#### React Native

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

#### Flutter

```dart
// Single-CTA onboarding: silent sign-in, fall through to register.
final config = defaultConfig(network: Network.mainnet)
    .copyWith(apiKey: '<breez api key>');
final response = await passkey.connectWithPasskey(
  request: ConnectWithPasskeyRequest(),
);

if (response.labels.length > 1) {
  // Returning multi-wallet user: let them pick a label, then sign in to it.
  // final chosen = await showWalletPicker(response.labels);
  // return (await passkey.signIn(request: SignInRequest(label: chosen))).wallet;
}

final sdk = await connect(
    request: ConnectRequest(
        config: config, seed: response.wallet.seed, storageDir: "./.data"));
```

#### Python

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

#### Go

```go
// Silent sign-in for a returning user, fall-through to register on a fresh device.
// No label: derives the default wallet and discovers this passkey's full label set.
response, err := passkey.ConnectWithPasskey(breez_sdk_spark.ConnectWithPasskeyRequest{})
if err != nil {
	return nil, err
}

if len(response.Labels) > 1 {
	// Returning multi-wallet user: let them pick a label and SignIn to it.
	// passkey.SignIn(breez_sdk_spark.SignInRequest{Label: &chosenLabel})
}

config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
sdk, err := breez_sdk_spark.Connect(breez_sdk_spark.ConnectRequest{
	Config:     config,
	Seed:       response.Wallet.Seed,
	StorageDir: "./.data",
})
if err != nil {
	return nil, err
}
```



### Web flow

`PasskeyClient.connect_with_passkey` works on web too, **where the browser supports immediate mediation** (recent Chromium). Check `PasskeyClient.supports_immediate_mediation` and use the same single-button unified flow.

Where it isn't supported (Safari, Firefox, older browsers), present two buttons: **Create a new passkey** (calls `PasskeyClient.register`) and **Sign in with a passkey** (calls `PasskeyClient.sign_in`). Without immediate mediation, WebAuthn reports "no credential" and "user cancelled" identically, so the SDK can't auto-detect the flow.

### Sign in and register

Call `PasskeyClient.sign_in` and `PasskeyClient.register` directly for explicit control: the two web buttons, separate create-a-passkey and sign-in screens, or adding a new label for a returning user. Pass `wallet.seed` to `connect` in either case.

#### Sign in

Sign in to an existing credential:

##### Rust

```rust
// Returning-user-only sign-in. No fall-through to register.
Ok(passkey
    .sign_in(SignInRequest {
        label: Some("personal".to_string()),
        ..Default::default()
    })
    .await?)
```

##### Swift

```swift
// Returning-user sign-in. No fall-through to register.
return try await passkey.signIn(request: SignInRequest(label: "personal"))
```

##### Kotlin

```kotlin
// Returning-user sign-in. No fall-through to register.
return passkey.signIn(SignInRequest(label = "personal"))
```

##### Javascript (Wasm)

```typescript
// Returning-user sign-in. No fall-through to register.
return await passkey.signIn({ label: 'personal' })
```

##### React Native

```typescript
// Returning-user sign-in. No fall-through to register.
return await passkey.signIn({
  label: 'personal',
  allowCredentials: undefined,
  preferImmediatelyAvailableCredentials: undefined
})
```

##### Flutter

```dart
// Returning-user sign-in. No fall-through to register.
return await passkey.signIn(request: SignInRequest(label: 'personal'));
```



#### Register

Register a fresh credential:

##### Rust

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

##### Swift

```swift
var config = defaultConfig(network: .mainnet)
config.apiKey = "<breez api key>"

let response = try await passkey.register(
    request: RegisterRequest(label: "personal")
)

let sdk = try await connect(
    request: ConnectRequest(
        config: config,
        seed: response.wallet.seed,
        storageDir: "./.data"
    ))
```

##### Kotlin

```kotlin
val config = defaultConfig(Network.MAINNET).apply { apiKey = "<breez api key>" }
val response = passkey.register(RegisterRequest(label = "personal"))

val sdk = connect(ConnectRequest(config, response.wallet.seed, "./.data"))
```

##### C#

```csharp
var response = await passkey.Register(new RegisterRequest(label: "personal"));

var config = BreezSdkSparkMethods.DefaultConfig(network: Network.Mainnet);
var sdk = await BreezSdkSparkMethods.Connect(new ConnectRequest(
    config: config,
    seed: response.wallet.seed,
    storageDir: "./.data"
));
```

##### Javascript (Wasm)

```typescript
const response = await passkey.register({ label: 'personal' })

const config = defaultConfig('mainnet')
const sdk = await connect({ config, seed: response.wallet.seed, storageDir: './.data' })
```

##### React Native

```typescript
const config = { ...defaultConfig(Network.Mainnet), apiKey: '<breez api key>' }
const response = await passkey.register({ label: 'personal', excludeCredentials: undefined })

const sdk = await connect({ config, seed: response.wallet.seed, storageDir: './.data' })
```

##### Flutter

```dart
final config = defaultConfig(network: Network.mainnet)
    .copyWith(apiKey: '<breez api key>');
final response = await passkey.register(
  request: RegisterRequest(label: 'personal'),
);

final sdk = await connect(
    request: ConnectRequest(
        config: config, seed: response.wallet.seed, storageDir: "./.data"));
```

##### Python

```python
response = await passkey.register(RegisterRequest(label="personal"))

config = default_config(network=Network.MAINNET)
sdk = await connect(
    ConnectRequest(config=config, seed=response.wallet.seed, storage_dir="./.data")
)
```

##### Go

```go
label := "personal"
response, err := passkey.Register(breez_sdk_spark.RegisterRequest{Label: &label})
if err != nil {
	return nil, err
}

config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
sdk, err := breez_sdk_spark.Connect(breez_sdk_spark.ConnectRequest{
	Config:     config,
	Seed:       response.Wallet.Seed,
	StorageDir: "./.data",
})
if err != nil {
	return nil, err
}
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

### Rust

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

### Swift

```swift
do {
    let response = try await passkey.register(
        request: RegisterRequest(
            label: "personal",
            excludeCredentials: [
                // app-persisted credential IDs from prior registrations
            ]
        )
    )
    return response.wallet
} catch PrfProviderError.CredentialAlreadyExists {
    // A matching credential already exists; sign in instead.
    let response = try await passkey.signIn(
        request: SignInRequest(label: "personal")
    )
    return response.wallet
}
```

### Kotlin

```kotlin
return try {
    val response = passkey.register(
        RegisterRequest(
            label = "personal",
            // app-persisted credential IDs from prior registrations
            excludeCredentials = emptyList(),
        )
    )
    response.wallet
} catch (e: PrfProviderException.CredentialAlreadyExists) {
    // A matching credential already exists; sign in to it instead.
    val response = passkey.signIn(SignInRequest(label = "personal"))
    response.wallet
}
```

### C#

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

### Javascript (Wasm)

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
  if (error instanceof PasskeyAlreadyExistsError) {
    // A matching credential already exists; sign in to it instead.
    const response = await passkey.signIn({ label: 'personal' })
    return response.wallet
  }
  throw error
}
```

### React Native

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

### Flutter

```dart
try {
  final response = await passkey.register(
    request: RegisterRequest(
      label: 'personal',
      excludeCredentials: [
        // app-persisted credential IDs from prior registrations
      ],
    ),
  );
  return response.wallet;
} on PasskeyPrfException catch (e) {
  if (e.code != 'credentialAlreadyExists') rethrow;
  // A matching credential already exists; sign in instead.
  final response = await passkey.signIn(
    request: SignInRequest(label: 'personal'),
  );
  return response.wallet;
}
```

### Python

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

### Go

```go
label := "personal"
registerResponse, err := passkey.Register(breez_sdk_spark.RegisterRequest{
	Label:              &label,
	ExcludeCredentials: &[][]byte{
		// app-persisted credential IDs from prior registrations
	},
})
if err == nil {
	return &registerResponse.Wallet, nil
}

if !errors.Is(err, breez_sdk_spark.ErrPrfProviderErrorCredentialAlreadyExists) {
	return nil, err
}

// A matching credential already exists; sign in to it instead.
signInResponse, err := passkey.SignIn(breez_sdk_spark.SignInRequest{Label: &label})
if err != nil {
	return nil, err
}
return &signInResponse.Wallet, nil
```



Show a sticky retry when the biometric timeout fires:

### Rust

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

### Swift

```swift
do {
    return try await passkey.signIn(
        request: SignInRequest(label: "personal")
    )
} catch PrfProviderError.UserTimedOut {
    // Show a retry UI. Do NOT auto-retry without user input.
    print("Sign-in timed out: show \"Try Again\" UI.")
    throw PrfProviderError.UserTimedOut
}
```

### Kotlin

```kotlin
return try {
    passkey.signIn(SignInRequest(label = "personal"))
} catch (e: PrfProviderException.UserTimedOut) {
    // Show a retry UI. Do NOT auto-retry without user input.
    // Log.v("Breez", "Sign-in timed out: show \"Try Again\" UI.")
    throw e
}
```

### C#

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

### Javascript (Wasm)

```typescript
// Biometric inactivity timeout, distinct from a user cancel.
try {
  const response = await passkey.signIn({ label: 'personal' })
  return response
} catch (error) {
  if (error instanceof PasskeyTimedOutError) {
    // Show a retry UI. Do NOT auto-retry without user input.
    console.log('Sign-in timed out: show "Try Again" UI.')
  }
  throw error
}
```

### React Native

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

### Flutter

```dart
// Timeout is distinct from a cancel: surface a re-prompt UI.
try {
  return await passkey.signIn(
    request: SignInRequest(label: 'personal'),
  );
} on PasskeyPrfException catch (e) {
  if (e.code == 'userTimedOut') {
    // Show a retry UI. Do NOT auto-retry without user input.
    print("Sign-in timed out: show \"Try Again\" UI.");
  }
  rethrow;
}
```

### Python

```python
# Biometric inactivity timeout, distinct from a user cancel.
try:
    return await passkey.sign_in(SignInRequest(label="personal"))
except PrfProviderError.UserTimedOut:
    # Show a retry UI. Do NOT auto-retry without user input.
    print("Sign-in timed out: show \"Try Again\" UI.")
    raise
```

### Go

```go
// Biometric inactivity timeout, distinct from a user cancel.
label := "personal"
response, err := passkey.SignIn(breez_sdk_spark.SignInRequest{Label: &label})
if err != nil {
	if errors.Is(err, breez_sdk_spark.ErrPrfProviderErrorUserTimedOut) {
		// Show a retry UI. Do NOT auto-retry without user input.
		log.Print("Sign-in timed out: show \"Try Again\" UI.")
	}
	return nil, err
}
return &response, nil
```



See the [UX guide](./uxguide_login.md) for the recommended recovery UX.

## Supported specs

- [Seedless Restore](https://github.com/breez/seedless-restore): passkey-based wallet derivation and discovery
- [Nostr](https://github.com/nostr-protocol/nostr): relay-based event protocol for label storage
- [NIP-42](https://github.com/nostr-protocol/nips/blob/master/42.md): authentication of clients to relays
- [NIP-65](https://github.com/nostr-protocol/nips/blob/master/65.md): relay list metadata

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
