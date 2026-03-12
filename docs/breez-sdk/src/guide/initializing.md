<h1 id="initializing">
    <a class="header" href="#initializing">Initializing the SDK</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.connect">API docs</a>
</h1>

## Basic Initialization

The easiest way to initialize the SDK is with the {{#name connect}} method. This method requires:

- The network, mnemonic, and Breez API key you intend to use
- A storage directory path where the SDK can manage its data

<div class="warning">
<h4>Developer note</h4>
For WASM Web, SDK storage is managed using IndexedDB.
</div>

The storage is used to persist the SDK’s state. If you run multiple SDK instances, each must have its own unique storage directory.

Once connected, you’re ready to start interacting with the SDK.

{{#tabs getting_started:init-sdk}}

<div class="warning">
<h4>Developer note</h4>

On some platforms (e.g., Android, iOS), you must use an application-specific writable directory within the app's sandbox for SDK storage.

</div>

## Advanced Initialization

For advanced use cases where you need more control, you can configure the SDK using the Builder pattern. With the SDK Builder you can define:

- [Storage](customizing.md#with-storage) to manage stored data
- [Bitcoin Chain Service](customizing.md#with-chain-service) to provide network data
- [LNURL Client](customizing.md#with-lnurl-client) to make REST requests
- [Fiat Service](customizing.md#with-fiat-service) to provide Fiat currencies and exchange rates 
- Change the [Key Set](customizing.md#with-key-set) to alter the derivation path used
- [Payment Observer](customizing.md#with-payment-observer) to be notified before payments occur

See [Customizing the SDK](customizing.md) for examples of this advanced initialization pattern.

<h2 id="disconnecting">
    <a class="header" href="#disconnecting">Disconnecting</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.disconnect">API docs</a>
</h2>

When you’re done using the SDK, call the disconnect method to release any resources in use.

This is particularly useful if you need to re-instantiate the SDK, such as when changing the mnemonic or updating configuration.

{{#tabs getting_started:disconnect}}
