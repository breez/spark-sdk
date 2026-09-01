# Using an External Signer

The External Signer feature allows you to provide custom signing logic for the SDK rather than relying on the SDK's internal key management. This is useful when you want to:

- Keep keys in a secured environment
- Implement custom key derivation logic
- Integrate with existing wallet infrastructure

## Using the Default External Signers

The external signer interface is split into two parts: an `ExternalBreezSigner` for SDK-layer signing (LNURL-auth, sync, message signing, ECIES) and an `ExternalSparkSigner` for the Spark wallet flows (transfers, claims, FROST signing, deposits). The SDK also ships a Turnkey-backed implementation that keeps the keys in a secure enclave; see [Using Turnkey](turnkey.md).

The SDK provides a convenient factory function `default_external_signers` that creates both signers from a mnemonic:

```rust
fn create_signers() -> Result<ExternalSigners, SdkError> {
    let mnemonic = "<mnemonic words>".to_string();
    let network = Network::Mainnet;

    let signers = default_external_signers(
        mnemonic,
        None, // passphrase
        network,
        Some(0), // account number
    )?;

    Ok(signers)
}
```



Provide both signers to the `connect_with_signer` method instead of the regular `connect` method:

```rust
async fn connect_example(signers: ExternalSigners) -> Result<BreezSdk, SdkError> {
    // Create the config
    let mut config = default_config(Network::Mainnet);
    config.api_key = Some("<breez api key>".to_string());

    // Connect using the external signers
    let sdk = connect_with_signer(ConnectWithSignerRequest {
        config,
        breez_signer: signers.breez_signer,
        spark_signer: signers.spark_signer,
        storage_dir: "./.data".to_string(),
    })
    .await?;

    Ok(sdk)
}
```



**Developer note**

When using an external signer, you don't provide a seed directly to the SDK. Instead, the signer handles all cryptographic operations internally.

## Advanced Setup with Sdk Builder

To compose an external signer along with the options in [customizing the SDK](./customizing.md) (custom storage backends, a shared SDK context, an account number), build the SDK with `new_with_signer` instead. It takes the same two signers and returns an `SdkBuilder` you chain the customization methods on before calling `build`:

```rust
async fn build_with_signer(signers: ExternalSigners) -> Result<BreezSdk, SdkError> {
    let mut config = default_config(Network::Mainnet);
    config.api_key = Some("<breez api key>".to_string());

    let builder = SdkBuilder::new_with_signer(config, signers.breez_signer, signers.spark_signer);
    // let builder = builder.with_storage_backend(<your storage backend>);
    // let builder = builder.with_shared_context(<your shared context>);
    let sdk = builder.build().await?;

    Ok(sdk)
}
```



For a signer that provides signing only (see [Signers Without Local ECIES/HMAC Support](#signers-without-local-ecieshmac-support)), use `new_with_signing_only_signer` the same way:

```rust
async fn build_with_signing_only_signer(
    config: Config,
    signers: SigningOnlyExternalSigners,
) -> Result<BreezSdk, SdkError> {
    let builder = SdkBuilder::new_with_signing_only_signer(
        config,
        signers.breez_signer,
        signers.spark_signer,
    );
    let sdk = builder.build().await?;

    Ok(sdk)
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

Some external signers can't perform the SDK's local ECIES/HMAC operations (for example, a policy-restricted enclave that won't release key material). For these, implement `ExternalSigningSigner` instead of `ExternalBreezSigner`, then connect with `connect_with_signing_only_signer`. With such a signer:

- **LNURL-auth** returns an error when called.
- **Real-time sync** must be disabled: leave [`real_time_sync_server_url`](./config.md#real-time-sync-server-url) unset, or the build fails.
- **Cross-chain** must be disabled: leave [`cross_chain_config`](./config.md#send-usdc-usdt) unset, or the build fails.
