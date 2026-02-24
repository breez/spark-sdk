# Customizing the SDK

Using the SDK Builder gives you more control over the initialization and modular components used when the SDK is running. Below you can find examples of initializing the SDK using the SDK Builder and implementing modular components:

- [Storage](#with-storage) to manage stored data
- [PostgreSQL Storage](#with-postgres-storage) as an alternative storage backend
- [Bitcoin Chain Service](#with-chain-service) to provide network data
- [LNURL Client](#with-lnurl-client) to make REST requests
- [Fiat Service](#with-fiat-service) to provide Fiat currencies and exchange rates
- Change the [Key Set](#with-key-set) to alter the derivation path used
- [Payment Observer](#with-payment-observer) to be notified before payments occur

{{#tabs sdk_building:init-sdk-advanced}}

<h2 id="with-storage">
    <a class="header" href="#with-storage">With Storage</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_storage">API docs</a>
</h2>

When using the SDK Builder, you either have to provide a Storage implementation or use the default storage from the SDK.

**Note:** Flutter currently only supports using the default storage.

<h2 id="with-postgres-storage">
    <a class="header" href="#with-postgres-storage">With PostgreSQL Storage</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_postgres_storage">API docs</a>
</h2>

The SDK includes a PostgreSQL storage implementation as an alternative. This is useful for environments where file-based storage may not be suitable.

**Note:** Not available for Javascript, React Native or Flutter.

{{#tabs sdk_building:init-sdk-postgres}}

<div class="warning">
<h4>Developer note</h4>

Sharing the same PostgreSQL database with multiple SDK instances is incompatible with real-time sync. See [Real-time sync server URL](./config.md#real-time-sync-server-url) for how to disable it.

</div>

<h2 id="with-chain-service">
    <a class="header" href="#with-chain-service">With Chain Service</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_chain_service">API docs</a>
</h2>

The SDK provides a default Bitcoin Chain Service implementation. If you want to use your own, you can provide it either by using [With REST Chain Service](#with-rest-chain-service) or by implementing the Bitcoin Chain Service interface.

<h2 id="with-rest-chain-service">
    <a class="header" href="#with-rest-chain-service">With REST Chain Service</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_rest_chain_service">API docs</a>
</h2>

The SDK provides a default Bitcoin Chain Service implementation. If you want to use your own, you can provide it either by using [With Chain Service](#with-chain-service) or by providing a URL and optional credentials.

{{#tabs sdk_building:with-rest-chain-service}}

<h2 id="with-fiat-service">
    <a class="header" href="#with-fiat-service">With Fiat Service</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_fiat_service">API docs</a>
</h2>

The SDK by default provides a list of available Fiat currencies and current exchange rates. If you want to use your own, you can provide it by implementing the Fiat Service interface.

<h2 id="with-lnurl-client">
    <a class="header" href="#with-lnurl-client">With LNURL Client</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_lnurl_client">API docs</a>
</h2>

The LNURL Client is used to make REST requests specifically when interacting with LNURL. If you want to use your own, you can it provide by implementing the REST Service interface.

<h2 id="with-key-set">
    <a class="header" href="#with-key-set">With Key Set</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_key_set">API docs</a>
</h2>

The SDK uses by default the Default key set with the account number 1 on Mainnet (0 on Regtest). You can change this to alter the derivation path used with the provided seed:

- **Default** - Uses derivation path `m/8797555'/<account number>` (use address index is ignored)
- **Taproot** - Uses derivation path `m/86'/0'/<account number>'/0/0`<br/>(or `m/86'/0'/0'/0/<account number>` when use address index is enabled)
- **Native Segwit** - Uses derivation path `m/84'/0'/<account number>'/0/0`<br/>(or `m/84'/0'/0'/0/<account number>` when use address index is enabled)
- **Wrapped Segwit** - Uses derivation path `m/49'/0'/<account number>'/0/0`<br/>(or `m/49'/0'/0'/0/<account number>` when use address index is enabled)
- **Legacy** - Uses derivation path `m/44'/0'/<account number>'/0/0`<br/>(or `m/44'/0'/0'/0/<account number>` when use address index is enabled)

{{#tabs sdk_building:with-key-set}}

<h2 id="with-payment-observer">
    <a class="header" href="#with-payment-observer">With Payment Observer</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_payment_observer">API docs</a>
</h2>

By implementing the Payment Observer interface you can be notified before a payment is sent. It includes information about the provisional payment including the payment ID, amount to be sent (in satoshis or token base units) and payment details based on the payment method.

**Note:** Flutter currently does not support this.

{{#tabs sdk_building:with-payment-observer}}
