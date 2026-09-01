# Listening to events

The SDK emits several events to provide the application with an up-to-date state of the SDK or ongoing payments.

## Event reference

| Event | Payload | What it means and what to do |
| --- | --- | --- |
| `SdkEventSynced` | none | The wallet finished syncing with the network. Refresh the balance and the payment list. See [getting the SDK info](get_info.md). |
| `SdkEventPaymentSucceeded` | `Payment` | A payment completed. The SDK refreshes its cached balance before emitting this, so `GetInfo` returns the new value. |
| `SdkEventPaymentPending` | `Payment` | A payment is in flight. The same payment is emitted again as succeeded or failed once it settles. |
| `SdkEventPaymentFailed` | `Payment` | A payment failed. Its `Details` carry the method-specific context to show the user. |
| `SdkEventNewDeposits` | `DepositInfo` list | On-chain deposits were detected. Only deposits whose `IsMature` is true can be claimed, so show the rest as pending. |
| `SdkEventClaimedDeposits` | `DepositInfo` list | Deposits were claimed into the wallet. The matching payment is emitted separately as `SdkEventPaymentSucceeded`. |
| `SdkEventUnclaimedDeposits` | `DepositInfo` list | The SDK could not claim these. Read `ClaimError` for the reason, then claim manually or refund. See [claiming on-chain deposits](onchain_claims.md). |
| `SdkEventAutoOptimization` | `AutoOptimizationEvent` | Progress of the background leaf optimizer. Manual `OptimizeLeaves` calls do not emit this. See [custom leaf optimization](optimize.md). |
| `SdkEventLightningAddressChanged` | `LightningAddressInfo`, unset when the address was deleted | The Lightning address changed on another device. See [receiving payments using LNURL-Pay](receive_lnurl_pay.md). |
| `SdkEventUnilateralExitStateChanged` | none | An exit state exported earlier is now out of date. Export it again. See [unilateral exit](unilateral_exit.md). |

The fields of `Payment` are described in [listing payments](list_payments.md). For
the order in which these events arrive during a receive, see
[receiving payments](receive_payment.md).

### Deposit fields

The three deposit events each carry a list of `DepositInfo`, whose fields determine
what to do next.

| Field | Meaning |
| --- | --- |
| `Txid`, `Vout` | The on-chain output the deposit came from. |
| `AmountSats` | Deposit value in satoshis. |
| `IsMature` | Whether the deposit has enough confirmations to be claimed. |
| `ClaimError` | Why the last claim attempt failed. Set on `SdkEventUnclaimedDeposits`. |
| `RefundTx`, `RefundTxId` | The refund transaction, once one has been created. |
| `RefundState` | How far the refund has got towards the network. Read it through `ListUnclaimedDeposits`: a refunded deposit no longer appears in these events. See [tracking a refund](onchain_claims.md#tracking-a-refund). |
| `InstantClaimStatus` | State of an instant (0-conf) claim attempt. Unset when none was attempted. |

## Add event listener

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.add_event_listener

```go
type SdkListener struct{}

func (SdkListener) OnEvent(e breez_sdk_spark.SdkEvent) {
	switch event := e.(type) {
	case breez_sdk_spark.SdkEventSynced:
		// Data has been synchronized with the network. When this event is received,
		// it is recommended to refresh the payment list and wallet balance.
	case breez_sdk_spark.SdkEventNewDeposits:
		// Detected deposits, as DepositInfo. Only those with IsMature set
		// have enough confirmations to be claimed. Show the rest as pending.
		newDeposits := event.NewDeposits
		_ = newDeposits
	case breez_sdk_spark.SdkEventUnclaimedDeposits:
		// Deposits the SDK could not claim. Each ClaimError says why,
		// most often the fee exceeded the configured maximum.
		unclaimedDeposits := event.UnclaimedDeposits
		_ = unclaimedDeposits
	case breez_sdk_spark.SdkEventClaimedDeposits:
		// Deposits claimed into the wallet. An instant (0-conf) claim is
		// reported here on submission and settles shortly after.
		claimedDeposits := event.ClaimedDeposits
		_ = claimedDeposits
	case breez_sdk_spark.SdkEventPaymentSucceeded:
		// A payment completed. The cached balance is already refreshed,
		// so GetInfo returns the new value.
		payment := event.Payment
		_ = payment
	case breez_sdk_spark.SdkEventPaymentPending:
		// A payment is awaiting confirmation. It arrives again as
		// succeeded or failed once it settles.
		pendingPayment := event.Payment
		_ = pendingPayment
	case breez_sdk_spark.SdkEventPaymentFailed:
		// A payment failed. payment.Details carries the method-specific
		// context to show the user.
		failedPayment := event.Payment
		_ = failedPayment
	case breez_sdk_spark.SdkEventAutoOptimization:
		// Background optimizer progress: started, round completed, or a
		// terminal outcome. Manual OptimizeLeaves calls do not emit these.
		optimizationEvent := event.OptimizationEvent
		_ = optimizationEvent
	case breez_sdk_spark.SdkEventLightningAddressChanged:
		// The lightning address changed on another device. Unset when the
		// address was deleted.
		lightningAddress := event.LightningAddress
		_ = lightningAddress
	case breez_sdk_spark.SdkEventUnilateralExitStateChanged:
		// The unilateral exit state changed, so a previously exported
		// one is now out of date. Export it again.
	default:
		// Handle any future event types
	}
}

func AddEventListener(sdk *breez_sdk_spark.BreezSdk, listener SdkListener) string {
	return sdk.AddEventListener(listener)
}
```



## Remove event listener

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.remove_event_listener

When you no longer need to listen to events, you can remove the listener.

```go
func RemoveEventListener(sdk *breez_sdk_spark.BreezSdk, listenerId string) bool {
	return sdk.RemoveEventListener(listenerId)
}
```
