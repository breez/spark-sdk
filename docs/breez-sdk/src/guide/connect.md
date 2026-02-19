<h1 id="connecting">
    <a class="header" href="#connecting">Connecting to the SDK</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.Breez.html#method.connect">API docs</a>
</h1>

## Simple Connect

The easiest way to initialize the SDK is with {{#name Breez::connect}}. You only need your
API key and a mnemonic — sensible defaults are applied automatically:

{{#tabs connect:connect-simple}}

## Connecting with Options

To customize the connection (network, storage directory, sync interval, etc.),
pass a {{#name ConnectOptions}} struct. All fields are optional — any field left as `None`
uses the default value:

{{#tabs connect:connect-with-options}}

<div class="warning">
<h4>Developer note</h4>

On some platforms (e.g., Android, iOS), you must use an application-specific
writable directory within the app's sandbox for the SDK storage. Set the
{{#name storage_dir}} option accordingly.

For WASM Web, SDK storage is managed using IndexedDB.
</div>

## Connecting with Custom Providers

For advanced use cases where you need to inject custom implementations of storage,
chain service, or other providers, use {{#name Breez::with_providers}}:

{{#tabs connect:connect-with-providers}}

See [Customizing the SDK](customizing.md) for details on each provider.

<h2 id="disconnecting">
    <a class="header" href="#disconnecting">Disconnecting</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.disconnect">API docs</a>
</h2>

When you're done using the SDK, call the {{#name disconnect}} method to release any
resources in use.

This is particularly useful if you need to re-instantiate the SDK, such as when
changing the mnemonic or updating configuration.

{{#tabs getting_started:disconnect}}
