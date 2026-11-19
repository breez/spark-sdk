# Credential metadata

Every passkey flow returns the credential it created or signed in with. Its IDs and attestation hints let you pin a returning user to the same wallet, prevent duplicate registrations, correlate with your backend, and show which authenticator holds the passkey.

`register`, `sign_in`, and `connect_with_passkey` each return a `credential` field. The built-in platform providers always populate it; only PRF backends that don't surface one (CLI / file-backed / hardware) leave it unset.

## Fields

`PasskeyCredential` carries:

| Field | Available | Use it for |
|---|---|---|
| `credential_id` | always | [Pinning a returning user](#pin-a-returning-user-to-the-same-wallet) and [preventing duplicate registrations](#prevent-duplicate-registrations). |
| `user_id` | registration | [Correlating with your backend](#correlate-the-credential-with-your-backend). |
| `aaguid` | registration | [Showing the authenticator](#show-the-authenticator-and-sync-status). Unverified. |
| `backup_eligible` | registration | [Showing the sync status](#show-the-authenticator-and-sync-status). |

## Using the fields

Each of these is optional. The basic register and sign-in flows need none of them: reach for one only when your app wants that behavior.

### Pin a returning user to the same wallet

Each credential derives its own wallet seed, so a returning user must sign in with the same credential to re-open the same wallet.

Persist `credential_id` after registration and pass it as `allow_credentials` on `sign_in`. The OS then offers only that credential. Omit `allow_credentials` and the OS picks any matching credential for your RP.

#### Rust

```rust
let response = passkey
    .register(RegisterRequest {
        label: Some("personal".to_string()),
        ..Default::default()
    })
    .await?;

if let Some(credential) = &response.credential {
    // Persist to reopen the same wallet on sign-in
    println!("{:?}", credential.credential_id);
    // Authenticator model (display hint, unverified)
    println!("{:?}", credential.aaguid);
    // Whether the passkey syncs across devices
    println!("{:?}", credential.backup_eligible);
}

// Pin the stored credential ID so the OS can't substitute a sibling,
// which would derive a different wallet.
let sign_in_response = passkey
    .sign_in(SignInRequest {
        label: Some("personal".to_string()),
        allow_credentials: Some(vec![/* stored credential_id bytes */]),
        ..Default::default()
    })
    .await?;
// Pass to connect() to open the wallet
println!("{:?}", sign_in_response.wallet.seed);
// Label this wallet was derived from
println!("{}", sign_in_response.wallet.label);
// This passkey's labels (populated on discovery sign-in)
println!("{:?}", sign_in_response.labels);
// Credential signed in with (credential_id only)
println!("{:?}", sign_in_response.credential);
```

#### Swift

```swift
let response = try await passkey.register(
    request: RegisterRequest(label: "personal")
)

if let credential = response.credential {
    // Persist to reopen the same wallet on sign-in
    print(credential.credentialId)
    // Authenticator model (display hint, unverified)
    print(credential.aaguid)
    // Whether the passkey syncs across devices
    print(credential.backupEligible)
}

// Pin the stored credential ID so the OS can't substitute a sibling
// credential, which would derive a different wallet.
let signInResponse = try await passkey.signIn(
    request: SignInRequest(
        label: "personal",
        allowCredentials: [
            // stored credentialId bytes
        ]
    )
)
// Pass to connect() to open the wallet
print(signInResponse.wallet.seed)
// Label this wallet was derived from
print(signInResponse.wallet.label)
// This passkey's labels (populated on discovery sign-in)
print(signInResponse.labels)
// Credential signed in with (credential_id only)
print(signInResponse.credential)
```

#### Kotlin

```kotlin
val response = passkey.register(RegisterRequest(label = "personal"))

response.credential?.let { credential ->
    // Log.v("Breez", "${credential.credentialId}") // Persist to reopen the same wallet on sign-in
    // Log.v("Breez", "${credential.aaguid}") // Authenticator model (display hint, unverified)
    // Log.v("Breez", "${credential.backupEligible}") // Whether the passkey syncs across devices
}

// Pin the stored credential ID so the OS can't substitute a sibling
// credential, which would derive a different wallet.
val signInResponse = passkey.signIn(
    SignInRequest(
        label = "personal",
        // stored credentialId bytes
        allowCredentials = emptyList(),
    )
)
// Log.v("Breez", "${signInResponse.wallet.seed}") // Pass to connect() to open the wallet
// Log.v("Breez", "${signInResponse.wallet.label}") // Label this wallet was derived from
// Log.v("Breez", "${signInResponse.labels}")
// This passkey's labels (populated on discovery sign-in)
// Log.v("Breez", "${signInResponse.credential}") // Credential signed in with (credential_id only)
```

#### C#

```csharp
var response = await passkey.Register(new RegisterRequest(label: "personal"));

if (response.credential is not null)
{
    // Persist to reopen the same wallet on sign-in
    Console.WriteLine(response.credential.credentialId);
    // Authenticator model (display hint, unverified)
    Console.WriteLine(response.credential.aaguid);
    // Whether the passkey syncs across devices
    Console.WriteLine(response.credential.backupEligible);
}

// Pin the stored credential ID so the OS can't substitute a sibling
// credential, which would derive a different wallet.
var signInResponse = await passkey.SignIn(new SignInRequest(
    label: "personal",
    allowCredentials: new byte[][]
    {
        // stored credentialId bytes
    }
));
// Pass to connect() to open the wallet
Console.WriteLine(signInResponse.wallet.seed);
// Label this wallet was derived from
Console.WriteLine(signInResponse.wallet.label);
// This passkey's labels (populated on discovery sign-in)
Console.WriteLine(signInResponse.labels);
// Credential signed in with (credential_id only)
Console.WriteLine(signInResponse.credential);
```

#### Javascript (Wasm)

```typescript
const response = await passkey.register({ label: 'personal' })

if (response.credential != null) {
  // Persist to reopen the same wallet on sign-in
  console.log(response.credential.credentialId)
  // Authenticator model (display hint, unverified)
  console.log(response.credential.aaguid)
  // Whether the passkey syncs across devices
  console.log(response.credential.backupEligible)
}

// Pin the stored credential ID so the OS can't substitute a sibling
// credential, which would derive a different wallet.
const signInResponse = await passkey.signIn({
  label: 'personal',
  allowCredentials: [/* stored credentialId bytes */]
})
// Pass to connect() to open the wallet
console.log(signInResponse.wallet.seed)
// Label this wallet was derived from
console.log(signInResponse.wallet.label)
// This passkey's labels (populated on discovery sign-in)
console.log(signInResponse.labels)
// Credential signed in with (credential_id only)
console.log(signInResponse.credential)
```

#### React Native

```typescript
const response = await passkey.register({ label: 'personal', excludeCredentials: undefined })

if (response.credential !== undefined) {
  // Persist to reopen the same wallet on sign-in
  console.log(response.credential.credentialId)
  // Authenticator model (display hint, unverified)
  console.log(response.credential.aaguid)
  // Whether the passkey syncs across devices
  console.log(response.credential.backupEligible)
}

// Pin the stored credential ID so the OS can't substitute a sibling
// credential, which would derive a different wallet.
const signInResponse = await passkey.signIn({
  label: 'personal',
  allowCredentials: [/* stored credentialId bytes */],
  preferImmediatelyAvailableCredentials: undefined
})
// Pass to connect() to open the wallet
console.log(signInResponse.wallet.seed)
// Label this wallet was derived from
console.log(signInResponse.wallet.label)
// This passkey's labels (populated on discovery sign-in)
console.log(signInResponse.labels)
// Credential signed in with (credential_id only)
console.log(signInResponse.credential)
```

#### Flutter

```dart
final response = await passkey.register(
  request: RegisterRequest(label: 'personal'),
);

final credential = response.credential;
if (credential != null) {
  // Persist to reopen the same wallet on sign-in
  print(credential.credentialId);
  // Authenticator model (display hint, unverified)
  print(credential.aaguid);
  // Whether the passkey syncs across devices
  print(credential.backupEligible);
}

// Pin the stored credential ID so the OS can't substitute a sibling.
final signInResponse = await passkey.signIn(
  request: SignInRequest(label: 'personal', allowCredentials: const [
    // stored credentialId bytes
  ]),
);
// Pass to connect() to open the wallet
print(signInResponse.wallet.seed);
// Label this wallet was derived from
print(signInResponse.wallet.label);
// This passkey's labels (populated on discovery sign-in)
print(signInResponse.labels);
// Credential signed in with (credential_id only)
print(signInResponse.credential);
```

#### Python

```python
response = await passkey.register(RegisterRequest(label="personal"))

if response.credential is not None:
    # Persist to reopen the same wallet on sign-in
    print(response.credential.credential_id)
    # Authenticator model (display hint, unverified)
    print(response.credential.aaguid)
    # Whether the passkey syncs across devices
    print(response.credential.backup_eligible)

# Pin the stored credential ID so the OS can't substitute a sibling
# credential, which would derive a different wallet.
sign_in_response = await passkey.sign_in(
    SignInRequest(
        label="personal",
        allow_credentials=[
            # stored credential_id bytes
        ],
    )
)
# Pass to connect() to open the wallet
print(sign_in_response.wallet.seed)
# Label this wallet was derived from
print(sign_in_response.wallet.label)
# This passkey's labels (populated on discovery sign-in)
print(sign_in_response.labels)
# Credential signed in with (credential_id only)
print(sign_in_response.credential)
```

#### Go

```go
label := "personal"
response, err := passkey.Register(breez_sdk_spark.RegisterRequest{Label: &label})
if err != nil {
	return err
}

if response.Credential != nil {
	// Persist to reopen the same wallet on sign-in
	log.Println(response.Credential.CredentialId)
	// Authenticator model (display hint, unverified)
	log.Println(response.Credential.Aaguid)
	// Whether the passkey syncs across devices
	log.Println(response.Credential.BackupEligible)
}

// Pin the stored credential ID so the OS can't substitute a sibling
// credential, which would derive a different wallet.
signInResponse, err := passkey.SignIn(breez_sdk_spark.SignInRequest{
	Label:            &label,
	AllowCredentials: &[][]byte{
		// stored CredentialId bytes
	},
})
if err != nil {
	return err
}
// Pass to connect() to open the wallet
log.Println(signInResponse.Wallet.Seed)
// Label this wallet was derived from
log.Println(signInResponse.Wallet.Label)
// This passkey's labels (populated on discovery sign-in)
log.Println(signInResponse.Labels)
// Credential signed in with (credential_id only)
log.Println(signInResponse.Credential)
```



### Prevent duplicate registrations

Pass the user's already-registered credential IDs as `exclude_credentials` on `register`. When one is already on the device, the OS refuses to create a second and raises `PrfProviderError::CredentialAlreadyExists`: route that to `sign_in` so the picker surfaces the existing credential.

#### Rust

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

#### Swift

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

#### Kotlin

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

#### C#

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

#### Javascript (Wasm)

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

#### React Native

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

#### Flutter

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

#### Python

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

#### Go

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



### Correlate the credential with your backend

If your backend ties passkeys to your own user accounts, `user_id` is a stable identifier set at registration that links the two. The SDK surfaces it locally and never transmits it. Persist it with your user record, then match it on later sign-ins to tell which user is signing in.

This enables account-level controls the passkey layer can't enforce on its own:

- Cap how many passkeys (and wallets) one account may register.
- Revoke a lost credential server-side.
- List a user's registered devices in their settings.

### Show the authenticator and sync status

`aaguid` identifies the authenticator that created the passkey (Apple Passwords, Google Password Manager, a hardware key). Look it up in the community [AAGUID database](https://github.com/passkeydeveloper/passkey-authenticator-aaguids) for a name and icon. `backup_eligible` tells you whether the passkey syncs across the user's devices.

> **Note:** `aaguid` and `backup_eligible` are unverified and self-reported by the authenticator. Use them as display hints, never as a trust signal.

## Persisting the values

The use cases above require these values to be persisted across app launches. `credential_id` is returned on every authentication response, while `aaguid`, `backup_eligible`, and `user_id` are only returned during registration and should be stored at that time.

Use synced storage such as iCloud Keychain (iOS), Block Store (Android), or your own synced backend. Local-only storage is insufficient because it is lost on app reinstall and cannot be accessed from another device.

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
