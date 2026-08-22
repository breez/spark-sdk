# Using an External Signer

The External Signer feature allows you to provide custom signing logic for the SDK rather than relying on the SDK's internal key management. This is useful when you want to:

- Keep keys in a secured environment
- Implement custom key derivation logic
- Integrate with existing wallet infrastructure

## Using the Default External Signers

The external signer interface is split into two parts: an `ExternalBreezSigner` for SDK-layer signing (LNURL-auth, sync, message signing, ECIES) and an `ExternalSparkSigner` for the Spark wallet flows (transfers, claims, FROST signing, deposits). The SDK also ships a Turnkey-backed implementation that keeps the keys in a secure enclave; see [Using Turnkey](turnkey.md).

The SDK provides a convenient factory function `default_external_signers` that creates both signers from a mnemonic:

```python
def create_signers() -> ExternalSigners:
    mnemonic = "<mnemonic words>"
    network = Network.MAINNET
    account_number = 0

    signers = default_external_signers(
        mnemonic=mnemonic,
        passphrase=None,
        network=network,
        account_number=account_number,
    )

    return signers
```



Provide both signers to the `connect_with_signer` method instead of the regular `connect` method:

```python
async def example_connect_with_signer(signers: ExternalSigners) -> BreezSdk:
    # Create the config
    config = default_config(Network.MAINNET)
    config.api_key = "<breez api key>"

    # Connect using the external signers
    sdk = await connect_with_signer(ConnectWithSignerRequest(
        config=config,
        breez_signer=signers.breez_signer,
        spark_signer=signers.spark_signer,
        storage_dir="./.data"
    ))

    return sdk
```



**Developer note**

When using an external signer, you don't provide a seed directly to the SDK. Instead, the signer handles all cryptographic operations internally.

## Advanced Setup with Sdk Builder

To compose an external signer along with the options in [customizing the SDK](./customizing.md) (custom storage backends, a shared SDK context, an account number), build the SDK with `new_with_signer` instead. It takes the same two signers and returns an `SdkBuilder` you chain the customization methods on before calling `build`:

```python
async def example_build_with_signer(signers: ExternalSigners) -> BreezSdk:
    config = default_config(Network.MAINNET)
    config.api_key = "<breez api key>"
    builder = SdkBuilder.new_with_signer(
        config=config,
        breez_signer=signers.breez_signer,
        spark_signer=signers.spark_signer,
    )
    # await builder.with_storage_backend(<your storage backend>)
    # await builder.with_shared_context(<your shared context>)
    sdk = await builder.build()
    return sdk
```



For a signer that provides signing only (see [Signers Without Local ECIES/HMAC Support](#signers-without-local-ecieshmac-support)), use `new_with_signing_only_signer` the same way:

```python
async def example_build_with_signing_only_signer(
    config: Config, signers: SigningOnlyExternalSigners
) -> BreezSdk:
    builder = SdkBuilder.new_with_signing_only_signer(
        config=config,
        breez_signer=signers.breez_signer,
        spark_signer=signers.spark_signer,
    )
    sdk = await builder.build()
    return sdk
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
