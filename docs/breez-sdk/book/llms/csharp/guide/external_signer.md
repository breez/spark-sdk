# Using an External Signer

The External Signer feature allows you to provide custom signing logic for the SDK rather than relying on the SDK's internal key management. This is useful when you want to:

- Keep keys in a secured environment
- Implement custom key derivation logic
- Integrate with existing wallet infrastructure

## Using the Default External Signers

The external signer interface is split into two parts: an `ExternalBreezSigner` for SDK-layer signing (LNURL-auth, sync, message signing, ECIES) and an `ExternalSparkSigner` for the Spark wallet flows (transfers, claims, FROST signing, deposits). The SDK also ships a Turnkey-backed implementation that keeps the keys in a secure enclave; see [Using Turnkey](turnkey.md).

The SDK provides a convenient factory function `DefaultExternalSigners` that creates both signers from a mnemonic:

```csharp
public static ExternalSigners CreateSigners()
{
    var mnemonic = "<mnemonic words>";
    var network = Network.Mainnet;
    uint accountNumber = 0;

    var signers = BreezSdkSparkMethods.DefaultExternalSigners(
        mnemonic: mnemonic,
        passphrase: null,
        network: network,
        accountNumber: accountNumber
    );

    return signers;
}
```



Provide both signers to the `ConnectWithSigner` method instead of the regular `Connect` method:

```csharp
public static async Task<BreezSdk> ConnectWithSigner(ExternalSigners signers)
{
    // Create the config
    var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
    {
        apiKey = "<breez api key>"
    };

    // Connect using the external signers
    var sdk = await BreezSdkSparkMethods.ConnectWithSigner(new ConnectWithSignerRequest(
        config: config,
        breezSigner: signers.breezSigner,
        sparkSigner: signers.sparkSigner,
        storageDir: "./.data"
    ));

    return sdk;
}
```



**Developer note**

When using an external signer, you don't provide a seed directly to the SDK. Instead, the signer handles all cryptographic operations internally.

## Advanced Setup with Sdk Builder

To compose an external signer along with the options in [customizing the SDK](./customizing.md) (custom storage backends, a shared SDK context, an account number), build the SDK with `NewWithSigner` instead. It takes the same two signers and returns an `SdkBuilder` you chain the customization methods on before calling `Build`:

```csharp
public static async Task<BreezSdk> BuildWithSigner(ExternalSigners signers)
{
    var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
    {
        apiKey = "<breez api key>"
    };

    var builder = SdkBuilder.NewWithSigner(
        config: config,
        breezSigner: signers.breezSigner,
        sparkSigner: signers.sparkSigner
    );
    // await builder.WithStorageBackend(storage: <your storage backend>);
    // await builder.WithSharedContext(<your shared context>);
    var sdk = await builder.Build();

    return sdk;
}
```



For a signer that provides signing only (see [Signers Without Local ECIES/HMAC Support](#signers-without-local-ecieshmac-support)), use `NewWithSigningOnlySigner` the same way:

```csharp
public static async Task<BreezSdk> BuildWithSigningOnlySigner(Config config, SigningOnlyExternalSigners signers)
{
    var builder = SdkBuilder.NewWithSigningOnlySigner(
        config: config,
        breezSigner: signers.breezSigner,
        sparkSigner: signers.sparkSigner
    );
    var sdk = await builder.Build();

    return sdk;
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
