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

```swift
let userSettings = try await sdk.getUserSettings()
print("User settings: \(userSettings)")
```



## Updating the user settings

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.update_user_settings

Every field of `UpdateUserSettingsRequest` is optional, and a field left unset is not changed. Settings that hold a value use an enum to distinguish assigning one from clearing it: `SparkMasterIdentityPublicKey.set` and `StableBalanceActiveLabel.set` assign, `SparkMasterIdentityPublicKey.unset` and `StableBalanceActiveLabel.unset` clear.

```swift
try await sdk.updateUserSettings(
    request: UpdateUserSettingsRequest(
        sparkPrivateModeEnabled: true,
        stableBalanceActiveLabel: nil,
        sparkMasterIdentityPublicKey: .set(publicKey: "<hex encoded public key>")
    ))
```
