# Treasury configuration

A treasury wallet is a single wallet the service itself owns, held in one long-lived SDK instance: a float, a settlement account, or the balance a backend pays out of and receives into.

This is a standard deployment. Background tasks stay on and the instance stays connected, so the rest of this guide applies unchanged. Build the config with `DefaultConfig`, not `DefaultServerConfig`: that preset turns off the background tasks a treasury relies on and exists for the [multi-user configuration](server_mode.md).

Three defaults are chosen for a mobile wallet and are worth changing:

```go
// Construct the seed using a mnemonic, entropy or passkey
mnemonic := "<mnemonic words>"
var seed breez_sdk_spark.Seed = breez_sdk_spark.SeedMnemonic{
	Mnemonic:   mnemonic,
	Passphrase: nil,
}

// A treasury wallet is an ordinary long-lived SDK instance, so start from
// DefaultConfig and keep background tasks on.
apiKey := "<breez api key>"
config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
config.ApiKey = &apiKey

// Keeps user data in step across devices that each hold their own
// storage, which a server-side wallet has no use for.
config.RealTimeSyncServerUrl = nil

// Stays connected, and syncs cost more with more leaves, so it can
// afford a longer interval.
config.SyncIntervalSecs = 300

// Optional: on a treasury that pays often and holds many leaves, collecting
// unilateral exit data behind every operation adds up. Turn it off and
// sync on a cadence of your own instead: a sync collects regardless of
// this flag.
config.ExitChainAutoFetchEnabled = false

builder := breez_sdk_spark.NewSdkBuilder(config, seed)
builder.WithDefaultStorage("./.data")
sdk, err := builder.Build()
if err != nil {
	return nil, err
}
```



## Disable real-time sync

The [real-time sync server](./config.md#real-time-sync-server-url) keeps wallet data in step across devices with separate storage. A treasury has one storage layer, so the WebSocket subscription and the upload after every change serve no purpose.

Unset `RealTimeSyncServerUrl`.

## Lengthen the synchronization interval

The [periodic sync](./config.md#synchronization-interval) defaults to 60 seconds. That suits a phone, whose event stream drops whenever the app is backgrounded. A treasury stays connected, and its syncs cost more because it holds more leaves.

300 seconds is a reasonable starting point. A wallet that settles rarely can go longer.

## Schedule the unilateral exit data collection

By default the SDK collects the data a [unilateral exit](unilateral_exit.md) needs in the background whenever the wallet gains leaves. On a treasury that pays often and holds many leaves, that is a round trip to the operators behind almost every operation.

Set `ExitChainAutoFetchEnabled` to `false` to disable the automatic collection. `SyncWallet` collects the missing data regardless of the flag and returns once it is done, so call it on a schedule of your own:

```go
// With automatic collection off, an explicit sync is what collects the data
// a unilateral exit needs, and it waits for the collection to finish. Needs
// the Spark operators reachable, so run it on a schedule rather than at the
// moment an exit is needed.
_, err := sdk.SyncWallet(breez_sdk_spark.SyncWalletRequest{})
if err != nil {
	return err
}
```



Funds received between two syncs cannot be exited without the operators until the next one. Pick a schedule with a gap you can accept, and keep the default if the automatic collection is not a measurable cost.
