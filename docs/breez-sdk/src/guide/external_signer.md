# Using an External Signer

The External Signer feature allows you to provide custom signing logic for the SDK rather than relying on the SDK's internal key management. This is useful when you want to:

- Keep keys in a secured environment
- Implement custom key derivation logic
- Integrate with existing wallet infrastructure

## Using the Default External Signer

The SDK provides a convenient factory function {{#name default_external_signer}} that creates a signer from a mnemonic. This is the easiest way to get started:

{{#tabs external_signer:default-external-signer}}

Once you have a signer, you can use it when connecting to the SDK with the {{#name connect_with_signer}} method instead of the regular {{#name connect}} method:

{{#tabs external_signer:connect-with-signer}}

<div class="warning">
<h4>Developer note</h4>
When using an external signer, you don't provide a seed directly to the SDK. Instead, the signer handles all cryptographic operations internally.
</div>

## Implementing a Custom Signer

If you need full control over the signing process, you can implement the [ExternalSigner](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/core/src/signer/external.rs) interface in your application. This interface defines all the cryptographic operations the SDK needs.

The [DefaultSigner](https://github.com/breez/spark-sdk/blob/main/crates/spark/src/signer/default_signer.rs) implementation can be used as a reference for what's expected.

<div class="warning">
<h4>Developer note</h4>

Implementing a custom signer requires deep understanding of Bitcoin cryptography. The default signer implementation provides a solid reference for what's expected.

Most applications should use the default external signer factory function rather than implementing their own.
</div>

<div class="warning">
<h4>Flutter Limitation</h4>

External signers are not supported in Flutter due to limitations with passing trait objects through the flutter_rust_bridge FFI. Flutter applications should use the standard `connect` method with mnemonic-based key management.
</div>
