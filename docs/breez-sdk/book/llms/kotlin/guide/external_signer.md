# Using an External Signer

The External Signer feature allows you to provide custom signing logic for the SDK rather than relying on the SDK's internal key management. This is useful when you want to:

- Keep keys in a secured environment
- Implement custom key derivation logic
- Integrate with existing wallet infrastructure

## Using the Default External Signers

The external signer interface is split into two parts: an `ExternalBreezSigner` for SDK-layer signing (LNURL-auth, sync, message signing, ECIES) and an `ExternalSparkSigner` for the Spark wallet flows (transfers, claims, FROST signing, deposits). The SDK also ships a Turnkey-backed implementation that keeps the keys in a secure enclave; see [Using Turnkey](turnkey.md).

The SDK provides a convenient factory function `defaultExternalSigners` that creates both signers from a mnemonic:

```kotlin
fun createSigners(): breez_sdk_spark.ExternalSigners {
    val mnemonic = "<mnemonic words>"
    val network = Network.MAINNET
    val accountNumber = 0U

    val signers = defaultExternalSigners(
        mnemonic = mnemonic,
        passphrase = null,
        network = network,
        accountNumber = accountNumber
    )

    return signers
}
```



Provide both signers to the `connectWithSigner` method instead of the regular `connect` method:

```kotlin
suspend fun connectWithSigner(signers: breez_sdk_spark.ExternalSigners) {
    // Create the config
    val config = defaultConfig(Network.MAINNET)
    config.apiKey = "<breez api key>"

    try {
        // Connect using the external signers
        val sdk = connectWithSigner(ConnectWithSignerRequest(
            config = config,
            breezSigner = signers.breezSigner,
            sparkSigner = signers.sparkSigner,
            storageDir = "./.data"
        ))
    } catch (e: Exception) {
        // handle error
    }
}
```



**Developer note**

When using an external signer, you don't provide a seed directly to the SDK. Instead, the signer handles all cryptographic operations internally.

## Advanced Setup with Sdk Builder

To compose an external signer along with the options in [customizing the SDK](./customizing.md) (custom storage backends, a shared SDK context, an account number), build the SDK with `newWithSigner` instead. It takes the same two signers and returns an `SdkBuilder` you chain the customization methods on before calling `build`:

```kotlin
suspend fun buildWithSigner(signers: breez_sdk_spark.ExternalSigners) {
    // Create the config
    val config = defaultConfig(Network.MAINNET)
    config.apiKey = "<breez api key>"

    try {
        val builder = SdkBuilder.newWithSigner(
            config,
            signers.breezSigner,
            signers.sparkSigner
        )
        // builder.withStorageBackend(<your storage backend>)
        // builder.withSharedContext(<your shared context>)
        val sdk = builder.build()
    } catch (e: Exception) {
        // handle error
    }
}
```



For a signer that provides signing only (see [Signers Without Local ECIES/HMAC Support](#signers-without-local-ecieshmac-support)), use `newWithSigningOnlySigner` the same way:

```kotlin
suspend fun buildWithSigningOnlySigner(
    config: breez_sdk_spark.Config,
    signers: breez_sdk_spark.SigningOnlyExternalSigners
) {
    try {
        val builder = SdkBuilder.newWithSigningOnlySigner(
            config,
            signers.breezSigner,
            signers.sparkSigner
        )
        val sdk = builder.build()
    } catch (e: Exception) {
        // handle error
    }
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

Some external signers can't perform the SDK's local ECIES/HMAC operations (for example, a policy-restricted enclave that won't release key material). For these, implement `ExternalSigningSigner` instead of `ExternalBreezSigner`, then connect with `connectWithSigningOnlySigner`. With such a signer:

- **LNURL-auth** returns an error when called.
- **Real-time sync** must be disabled: leave [`realTimeSyncServerUrl`](./config.md#real-time-sync-server-url) unset, or the build fails.
- **Cross-chain** must be disabled: leave [`crossChainConfig`](./config.md#send-usdc-usdt) unset, or the build fails.
