# Using an External Signer

The External Signer feature allows you to provide custom signing logic for the SDK rather than relying on the SDK's internal key management. This is useful when you want to:

- Keep keys in a secured environment
- Implement custom key derivation logic
- Integrate with existing wallet infrastructure

## Using the Default External Signers

The external signer interface is split into two parts: an `ExternalBreezSigner` for SDK-layer signing (LNURL-auth, sync, message signing, ECIES) and an `ExternalSparkSigner` for the Spark wallet flows (transfers, claims, FROST signing, deposits).

The SDK provides a convenient factory function {{#name default_external_signers}} that creates both signers from a mnemonic:

{{#tabs external_signer:default-external-signer}}

Provide both signers to the {{#name connect_with_signer}} method instead of the regular {{#name connect}} method:

{{#tabs external_signer:connect-with-signer}}

<div class="warning">
<h4>Developer note</h4>
When using an external signer, you don't provide a seed directly to the SDK. Instead, the signer handles all cryptographic operations internally.
</div>

## Advanced Setup with Sdk Builder

To compose an external signer along with the options in [customizing the SDK](./customizing.md) (custom storage backends, a shared SDK context, an account number), build the SDK with {{#name new_with_signer}} instead. It takes the same two signers and returns an {{#name SdkBuilder}} you chain the customization methods on before calling {{#name build}}:

{{#tabs external_signer:sdk-builder-with-signer}}

For a signer that provides signing only (see [Signers Without Local ECIES/HMAC Support](#signers-without-local-ecieshmac-support)), use {{#name new_with_signing_only_signer}} the same way:

{{#tabs external_signer:sdk-builder-with-signing-only-signer}}

## Implementing a Custom Signer

If you need full control over the signing process, you can implement the [ExternalBreezSigner](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/core/src/signer/external.rs) and [ExternalSparkSigner](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/core/src/signer/external_spark.rs) interfaces in your application. These interfaces define all the cryptographic operations the SDK needs.

The default implementations of the two interfaces, [DefaultExternalSigner](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/core/src/signer/default_external.rs) and [DefaultExternalSparkSigner](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/core/src/signer/default_external_spark.rs), can be used as a reference for what's expected.

<div class="warning">
<h4>Developer note</h4>

Implementing a custom signer requires deep understanding of Bitcoin cryptography. The default signer implementations provide a solid reference for what's expected.

Most applications should use the default external signers factory function rather than implementing their own.
</div>

<div class="warning">
<h4>Flutter Limitation</h4>

External signers are not supported in Flutter due to limitations with passing trait objects through the flutter_rust_bridge FFI. Flutter applications should use the standard `connect` method with mnemonic-based key management.
</div>

### Signers Without Local ECIES/HMAC Support

Some external signers can't perform the SDK's local ECIES/HMAC operations (for example, a policy-restricted enclave that won't release key material). For these, implement {{#name ExternalSigningSigner}} instead of {{#name ExternalBreezSigner}}, then connect with {{#name connect_with_signing_only_signer}}. With such a signer:

- **Session tokens** are stored in plaintext instead of encrypted at rest.
- **LNURL-auth** returns an error when called.
- **Real-time sync** must be disabled: leave [{{#name real_time_sync_server_url}}](./config.md#real-time-sync-server-url) unset, or the build fails.
- **Cross-chain** must be disabled: leave [{{#name cross_chain_config}}](./config.md#send-usdc-usdt) unset, or the build fails.
