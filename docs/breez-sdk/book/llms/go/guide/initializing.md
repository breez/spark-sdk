# Initializing the SDK

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.connect

## Basic Initialization

The easiest way to initialize the SDK is with the `Connect` method. This method requires:

- The network, mnemonic, and Breez API key you intend to use
- A storage directory path where the SDK can manage its data

**Developer note**

For WASM Web, SDK storage is managed using IndexedDB.

The storage is used to persist the SDK’s state. If you run multiple SDK instances, each must have its own unique storage directory.

Once connected, you’re ready to start interacting with the SDK.

```go
// Construct the seed using a mnemonic, entropy or passkey
mnemonic := "<mnemonic words>"
var seed breez_sdk_spark.Seed = breez_sdk_spark.SeedMnemonic{
	Mnemonic:   mnemonic,
	Passphrase: nil,
}

// Create the default config
apiKey := "<breez api key>"
config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
config.ApiKey = &apiKey

connectRequest := breez_sdk_spark.ConnectRequest{
	Config:     config,
	Seed:       seed,
	StorageDir: "./.data",
}

// Connect to the SDK using the simplified connect method
sdk, err := breez_sdk_spark.Connect(connectRequest)

return sdk, err
```



**Developer note**

On some platforms (e.g., Android, iOS), you must use an application-specific writable directory within the app's sandbox for SDK storage.

### Connecting with a Passkey

Instead of managing mnemonics directly, you can use passkeys to derive wallet seeds deterministically. This eliminates the need for mnemonic backup and provides a seamless authentication experience using biometrics or device PIN.

See [Connecting with a Passkey](passkey.md) for the full setup guide including PRF provider implementation, platform configuration, and label management.

## Advanced Initialization

If you're building a multi-tenant server deployment, start with [Server mode](server_mode.md) for the recommended profile, lifecycle pattern, and shared-infrastructure wiring.

For advanced use cases where you need more control, you can configure the SDK using the Builder pattern. With the SDK Builder you can define:

- [Storage](customizing.md#with-storage) to manage stored data
- [Bitcoin Chain Service](customizing.md#with-chain-service) to provide network data
- [LNURL Client](customizing.md#with-lnurl-client) to make REST requests
- [Fiat Service](customizing.md#with-fiat-service) to provide Fiat currencies and exchange rates 
- Change the [Account Number](customizing.md#with-account-number) to derive an independent wallet from the same seed
- [Payment Observer](customizing.md#with-payment-observer) to be notified before payments occur

See [Customizing the SDK](customizing.md) for examples of this advanced initialization pattern.

## Disconnecting

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.disconnect

When you’re done using the SDK, call the disconnect method to release any resources in use.

This is particularly useful if you need to re-instantiate the SDK, such as when changing the mnemonic or updating configuration.

```go
func Disconnect(sdk *breez_sdk_spark.BreezSdk) {
	sdk.Disconnect()
}
```
