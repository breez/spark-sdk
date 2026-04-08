<h1 id="getting-the-sdk-info">
    <a class="header" href="#getting-the-sdk-info">Getting the SDK info</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_info">API docs</a>
</h1>

Once connected, you can retrieve the current state of the SDK at any time using {{#name get_info}}. This returns:

- **Spark identity public key** - The wallet's unique identity on the Spark network as a hex string
- **Bitcoin balance** - The balance in satoshis
- **Token balances** - Balances of any tokens held in the wallet

{{#tabs getting_started:fetch-balance}}

## Fetching the balance

The SDK keeps a **cached balance** in local storage and {{#name get_info}} reads from this cache for a low-latency response. The cache is refreshed automatically by the SDK's background sync.

The recommended pattern is:

1. Call {{#name get_info}} with {{#name ensure_synced}} = **false** whenever you need to render the balance.
2. Subscribe to events and call {{#name get_info}} again on each {{#enum SdkEvent::Synced}} event to fetch the latest balance. See [Listening to events](/guide/events.md).

| Event | Description | UX Suggestion |
| ----- | ----------- | ------------- |
| {{#enum SdkEvent::Synced}} | The SDK has synced with the network in the background. | Call {{#name get_info}} to refresh the displayed balance, and refresh the payments list. See [listing payments](/guide/list_payments.md). |

<div class="warning">
<h4>Developer note</h4>

{{#name ensure_synced}} = **true** blocks until the SDK's **initial** sync after {{#name connect}} completes. This is useful for short-lived scripts that connect, read the balance once, and disconnect. It is **not** a "force a fresh sync now" call. In long-running applications, prefer {{#name ensure_synced}} = **false** combined with the {{#enum SdkEvent::Synced}} event listener pattern above.

</div>