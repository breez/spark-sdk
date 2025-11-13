<h1 id="fetching-the-balance">
    <a class="header" href="#fetching-the-balance">Fetching the balance</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_info">API docs</a>
</h1>

Once connected, the balance can be retrieved at any time.

{{#tabs getting_started:fetch-balance}}

<div class="warning">
<h4>Developer note</h4>
The SDK maintains a cached balance for fast responses and updates it on every change. The `get_info` call returns the value from this cache to provide a low-latency user experience.

Right after startup, the cache may not yet reflect the latest state from the network. Depends on your use case you can use one of the following options to get the fully up to date balance:

- If your application runs continuously in the background, call `get_info` after each `SdkEvent::Synced` event.
- If you're only briefly using the SDK to fetch the balance, call `get_info` with `ensure_synced = true` before disconnecting.

</div>