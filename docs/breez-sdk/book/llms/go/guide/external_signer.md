# Using an External Signer

The External Signer feature allows you to provide custom signing logic for the SDK rather than relying on the SDK's internal key management. This is useful when you want to:

- Keep keys in a secured environment
- Implement custom key derivation logic
- Integrate with existing wallet infrastructure

## Using the Default External Signers

The external signer interface is split into two parts: an `ExternalBreezSigner` for SDK-layer signing (LNURL-auth, sync, message signing, ECIES) and an `ExternalSparkSigner` for the Spark wallet flows (transfers, claims, FROST signing, deposits). The SDK also ships a Turnkey-backed implementation that keeps the keys in a secure enclave; see [Using Turnkey](turnkey.md).

The SDK provides a convenient factory function `DefaultExternalSigners` that creates both signers from a mnemonic:

```go
func createSigners() (breez_sdk_spark.ExternalSigners, error) {
	mnemonic := "<mnemonic words>"
	network := breez_sdk_spark.NetworkMainnet
	var accountNumber uint32 = 0

	signers, err := breez_sdk_spark.DefaultExternalSigners(
		mnemonic,
		nil, // passphrase
		network,
		&accountNumber,
	)
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return breez_sdk_spark.ExternalSigners{}, err
	}

	return signers, nil
}
```



Provide both signers to the `ConnectWithSigner` method instead of the regular `Connect` method:

```go
func connectWithSigner(
	signers breez_sdk_spark.ExternalSigners,
) (*breez_sdk_spark.BreezSdk, error) {
	// Create the config
	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
	apiKey := "<breez api key>"
	config.ApiKey = &apiKey

	// Connect using the external signers
	sdk, err := breez_sdk_spark.ConnectWithSigner(breez_sdk_spark.ConnectWithSignerRequest{
		Config:      config,
		BreezSigner: signers.BreezSigner,
		SparkSigner: signers.SparkSigner,
		StorageDir:  "./.data",
	})
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return nil, err
	}

	return sdk, nil
}
```



**Developer note**

When using an external signer, you don't provide a seed directly to the SDK. Instead, the signer handles all cryptographic operations internally.

## Advanced Setup with Sdk Builder

To compose an external signer along with the options in [customizing the SDK](./customizing.md) (custom storage backends, a shared SDK context, an account number), build the SDK with `NewWithSigner` instead. It takes the same two signers and returns an `SdkBuilder` you chain the customization methods on before calling `Build`:

```go
func buildWithSigner(
	signers breez_sdk_spark.ExternalSigners,
) (*breez_sdk_spark.BreezSdk, error) {
	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
	apiKey := "<breez api key>"
	config.ApiKey = &apiKey

	builder := breez_sdk_spark.SdkBuilderNewWithSigner(config, signers.BreezSigner, signers.SparkSigner)
	// builder.WithStorageBackend(<your storage backend>)
	// builder.WithSharedContext(<your shared context>)
	sdk, err := builder.Build()
	if err != nil {
		return nil, err
	}

	return sdk, nil
}
```



For a signer that provides signing only (see [Signers Without Local ECIES/HMAC Support](#signers-without-local-ecieshmac-support)), use `NewWithSigningOnlySigner` the same way:

```go
func buildWithSigningOnlySigner(
	config breez_sdk_spark.Config,
	signers breez_sdk_spark.SigningOnlyExternalSigners,
) (*breez_sdk_spark.BreezSdk, error) {
	builder := breez_sdk_spark.SdkBuilderNewWithSigningOnlySigner(config, signers.BreezSigner, signers.SparkSigner)
	sdk, err := builder.Build()
	if err != nil {
		return nil, err
	}

	return sdk, nil
}
```



## Implementing a Custom Signer

If you need full control over the signing process, you can implement the [ExternalBreezSigner](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/core/src/signer/external.rs) and [ExternalSparkSigner](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/core/src/signer/external_spark.rs) interfaces in your application. These interfaces define all the cryptographic operations the SDK needs.

The default implementations of the two interfaces, [DefaultExternalSigner](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/core/src/signer/default_external.rs) and [DefaultExternalSparkSigner](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/core/src/signer/default_external_spark.rs), can be used as a reference for what's expected.

**Developer note**

Implementing a custom signer requires deep understanding of Bitcoin cryptography. The default signer implementations provide a solid reference for what's expected.

Most applications should use the default external signers factory function rather than implementing their own.

**Flutter Limitation**

External signers are not supported in Flutter due to limitations with passing trait objects through the flutter_rust_bridge FFI. Flutter applications should use the standard `connect` method with mnemonic-based key management.

### Signers Without Local ECIES/HMAC Support

Some external signers can't perform the SDK's local ECIES/HMAC operations (for example, a policy-restricted enclave that won't release key material). For these, implement `ExternalSigningSigner` instead of `ExternalBreezSigner`, then connect with `ConnectWithSigningOnlySigner`. With such a signer:

- **LNURL-auth** returns an error when called.
- **Real-time sync** must be disabled: leave [`RealTimeSyncServerUrl`](./config.md#real-time-sync-server-url) unset, or the build fails.
- **Cross-chain** must be disabled: leave [`CrossChainConfig`](./config.md#send-usdc-usdt) unset, or the build fails.
