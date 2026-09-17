<h1 id="treasury-wallets">
    <a class="header" href="#treasury-wallets">Treasury wallets</a>
</h1>

A treasury wallet is a single wallet your own service owns, held in one long-lived SDK instance: a settlement account, or the balance a backend pays out of and receives into.

This is an ordinary SDK deployment. Background tasks stay on and the instance stays connected, so the rest of this guide applies unchanged. Build the config with {{#name default_config}}, not {{#name default_server_config}}: that preset turns off the background work a treasury wants. See [Serving end-user wallets](server_mode.md) for what it is for.

A few defaults are chosen for a wallet in someone's pocket and are worth revisiting here.

{{#tabs sdk_building:init-sdk-treasury}}

## Turn off real-time sync

The [real-time sync server](./config.md#real-time-sync-server-url) keeps user data in step across devices that each hold their own storage, such as a phone and a tablet. A treasury has no such split: whatever runs against the wallet shares one storage layer. Leaving it on pays for a standing WebSocket subscription and an upload behind every change, for nothing.

Unset {{#name real_time_sync_server_url}} to skip it.

## Lengthen the synchronization interval

The [periodic sync](./config.md#synchronization-interval) defaults to 60 seconds, which suits a wallet on a phone: its event stream drops whenever the app is backgrounded, so the sync is what catches up. A treasury stays connected, and its syncs cost more because it holds more leaves.

300 seconds is a reasonable starting point, and a wallet that settles rarely can go longer.

## Choose when to download unilateral exit data

By default the SDK collects the data a [unilateral exit](unilateral_exit.md) needs as funds arrive, after an operation rather than during it. On a treasury that pays frequently and holds many leaves, that is a round trip behind almost every operation.

Set {{#name exit_chain_auto_fetch_enabled}} to `false` to stop it, then call {{#name sync_wallet}} on a cadence of your own to collect:

{{#tabs unilateral_exit:sync-exit-data}}

The flag governs only the automatic collection, so a sync you call yourself collects regardless of it, and waits for the pass before returning. Nothing else has to change: the leaf set is still kept current for you by the background sync and the operator event stream.

Between those syncs, recently received funds are not exitable without the operators. The gap you choose is the window you are accepting, so leave the default alone if the automatic collection does not show up in your metrics.
