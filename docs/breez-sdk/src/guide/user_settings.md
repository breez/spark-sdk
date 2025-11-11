# User settings

The SDK exposes a set of user settings that are shared across all SDK instances, even from different partners.

<h2 id="available-user-settings">
    <a class="header" href="#available-user-settings">Available user settings</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.UserSettings.html">API docs</a>
</h2>

The following user settings are available:

- **Spark private mode**: Spark supports opt-in wallet privacy. When enabled, the wallet's payments and balances will not be accessible through public indexers like [Sparkscan](https://sparkscan.io). The SDK enables this by default for new wallets, and we highly recommend keeping it enabled. However, some applications may require the wallet to be visible to the public.

<h2 id="getting-the-current-user-settings">
    <a class="header" href="#getting-the-current-user-settings">Getting the current user settings</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_user_settings">API docs</a>
</h2>

{{#tabs user_settings:get-user-settings}}

<h2 id="updating-the-user-settings">
    <a class="header" href="#updating-the-user-settings">Updating the user settings</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.update_user_settings">API docs</a>
</h2>

{{#tabs user_settings:update-user-settings}}
