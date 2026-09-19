# User settings

The SDK exposes a set of user settings that are shared across all SDK instances, even from different partners.

## Available user settings

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.UserSettings.html

The following user settings are available:

- **Spark private mode**: Spark supports opt-in wallet privacy. When enabled, the wallet's Bitcoin payments and balance will not be accessible through public indexers like [Sparkscan](https://sparkscan.io). The SDK enables this by default for new wallets, and we highly recommend keeping it enabled. However, some applications may require the wallet to be visible to the public.

> **Note:** Spark private mode only applies to Bitcoin payments. Token payments are not affected by the private mode and will still be publicly available.

- **Stable balance active label**: Controls which stable token is active for automatic Bitcoin-to-token conversion. Set to a label from your [stable balance configuration](./config.md#stable-balance-configuration) to activate, or unset to deactivate. See the [Stable balance](./stable_balance.md) guide for details.

- **Spark master identity public key**: A second public key that Spark accepts as a reader of the wallet while private mode is enabled. It enables watch-only views of a private wallet: designate a key you control, and a Spark client authenticating with the corresponding private key can query the wallet's Bitcoin balance and payment history. The master identity is read-only, so making payments still requires the wallet's own keys. The same public key can be designated across many wallets.

## Getting the current user settings

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_user_settings

### Rust

```rust
let user_settings = sdk.get_user_settings().await?;
info!("User settings: {:?}", user_settings);
```

### Swift

```swift
let userSettings = try await sdk.getUserSettings()
print("User settings: \(userSettings)")
```

### Kotlin

```kotlin
try {
    val userSettings = sdk.getUserSettings()
    println("User settings: $userSettings")
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
var userSettings = await sdk.GetUserSettings();

Console.WriteLine($"User settings: {userSettings}");
```

### Javascript (Wasm)

```typescript
const userSettings = await sdk.getUserSettings()
console.log(`User settings: ${JSON.stringify(userSettings)}`)
```

### React Native

```typescript
const userSettings = await sdk.getUserSettings()
console.log(`User settings: ${JSON.stringify(userSettings)}`)
```

### Flutter

```dart
final userSettings = await sdk.getUserSettings();
print('User settings: $userSettings');
```

### Python

```python
try:
    user_settings = await sdk.get_user_settings()

    print(f"User settings: {user_settings}")
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
userSettings, err := sdk.GetUserSettings()

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}

log.Printf("User settings: %v", userSettings)
```



## Updating the user settings

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.update_user_settings

Every field of `UpdateUserSettingsRequest` is optional, and a field left unset is not changed. Settings that hold a value use an enum to distinguish assigning one from clearing it: `SparkMasterIdentityPublicKey::Set` and `StableBalanceActiveLabel::Set` assign, `SparkMasterIdentityPublicKey::Unset` and `StableBalanceActiveLabel::Unset` clear.

### Rust

```rust
sdk.update_user_settings(UpdateUserSettingsRequest {
    spark_private_mode_enabled: Some(true),
    stable_balance_active_label: None,
    spark_master_identity_public_key: Some(SparkMasterIdentityPublicKey::Set {
        public_key: "<hex encoded public key>".to_string(),
    }),
})
.await?;
```

### Swift

```swift
try await sdk.updateUserSettings(
    request: UpdateUserSettingsRequest(
        sparkPrivateModeEnabled: true,
        stableBalanceActiveLabel: nil,
        sparkMasterIdentityPublicKey: .set(publicKey: "<hex encoded public key>")
    ))
```

### Kotlin

```kotlin
try {
    sdk.updateUserSettings(UpdateUserSettingsRequest(
        sparkPrivateModeEnabled = true,
        stableBalanceActiveLabel = null,
        sparkMasterIdentityPublicKey = SparkMasterIdentityPublicKey.Set(
            publicKey = "<hex encoded public key>"
        )
    ))
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
await sdk.UpdateUserSettings(
    request: new UpdateUserSettingsRequest(
        sparkPrivateModeEnabled: true,
        stableBalanceActiveLabel: null,
        sparkMasterIdentityPublicKey: new SparkMasterIdentityPublicKey.Set(
            publicKey: "<hex encoded public key>"
        )
    )
);
```

### Javascript (Wasm)

```typescript
await sdk.updateUserSettings({
  sparkPrivateModeEnabled: true,
  stableBalanceActiveLabel: undefined,
  sparkMasterIdentityPublicKey: {
    type: 'set',
    publicKey: '<hex encoded public key>'
  }
})
```

### React Native

```typescript
await sdk.updateUserSettings({
  sparkPrivateModeEnabled: true,
  stableBalanceActiveLabel: undefined,
  sparkMasterIdentityPublicKey: new SparkMasterIdentityPublicKey.Set({
    publicKey: '<hex encoded public key>'
  })
})
```

### Flutter

```dart
await sdk.updateUserSettings(
    request: UpdateUserSettingsRequest(
        sparkPrivateModeEnabled: true,
        stableBalanceActiveLabel: null,
        sparkMasterIdentityPublicKey: SparkMasterIdentityPublicKey_Set(
            publicKey: "<hex encoded public key>")));
```

### Python

```python
try:
    await sdk.update_user_settings(
        request=UpdateUserSettingsRequest(
            spark_private_mode_enabled=True,
            stable_balance_active_label=None,
            spark_master_identity_public_key=SparkMasterIdentityPublicKey.SET(
                public_key="<hex encoded public key>"
            )
        )
    )
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
sparkPrivateModeEnabled := true
masterIdentityPublicKey := breez_sdk_spark.SparkMasterIdentityPublicKey(
	breez_sdk_spark.SparkMasterIdentityPublicKeySet{
		PublicKey: "<hex encoded public key>",
	},
)
err := sdk.UpdateUserSettings(breez_sdk_spark.UpdateUserSettingsRequest{
	SparkPrivateModeEnabled:      &sparkPrivateModeEnabled,
	StableBalanceActiveLabel:     nil,
	SparkMasterIdentityPublicKey: &masterIdentityPublicKey,
})

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}
```



---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
