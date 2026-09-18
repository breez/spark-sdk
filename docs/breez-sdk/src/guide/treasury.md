# Treasury configuration

A treasury wallet is a single wallet the service itself owns, held in one long-lived SDK instance: a float, a settlement account, or the balance a backend pays out of and receives into.

This is a standard deployment. Background tasks stay on and the instance stays connected, so the rest of this guide applies unchanged. Build the config with {{#name default_config}}, not {{#name default_server_config}}: that preset turns off the background tasks a treasury relies on and exists for the [multi-user configuration](server_mode.md).

Three defaults are chosen for a mobile wallet and are worth changing:

{{#tabs sdk_building:init-sdk-treasury}}

## Disable real-time sync

The [real-time sync server](./config.md#real-time-sync-server-url) keeps wallet data in step across devices with separate storage. A treasury has one storage layer, so the WebSocket subscription and the upload after every change serve no purpose.

Unset {{#name real_time_sync_server_url}}.

## Lengthen the synchronization interval

The [periodic sync](./config.md#synchronization-interval) defaults to 60 seconds. That suits a phone, whose event stream drops whenever the app is backgrounded. A treasury stays connected, and its syncs cost more because it holds more leaves.

300 seconds is a reasonable starting point. A wallet that settles rarely can go longer.

## Schedule the unilateral exit data collection

By default the SDK collects the data a [unilateral exit](unilateral_exit.md) needs in the background whenever the wallet gains leaves. On a treasury that pays often and holds many leaves, that is a round trip to the operators behind almost every operation.

Set {{#name exit_chain_auto_fetch_enabled}} to `false` to disable the automatic collection. {{#name sync_wallet}} collects the missing data regardless of the flag and returns once it is done, so call it on a schedule of your own:

{{#tabs unilateral_exit:sync-exit-data}}

Funds received between two syncs cannot be exited without the operators until the next one. Pick a schedule with a gap you can accept, and keep the default if the automatic collection is not a measurable cost.
