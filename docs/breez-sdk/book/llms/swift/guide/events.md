# Listening to events

The SDK emits several events to provide the application with an up-to-date state of the SDK or ongoing payments.

## Event reference

| Event | Payload | What it means and what to do |
| --- | --- | --- |
| `SdkEvent.synced` | none | The wallet finished syncing with the network. Refresh the balance and the payment list. See [getting the SDK info](get_info.md). |
| `SdkEvent.paymentSucceeded` | `Payment` | A payment completed. The SDK refreshes its cached balance before emitting this, so `getInfo` returns the new value. |
| `SdkEvent.paymentPending` | `Payment` | A payment is in flight. The same payment is emitted again as succeeded or failed once it settles. |
| `SdkEvent.paymentFailed` | `Payment` | A payment failed. Its `details` carry the method-specific context to show the user. |
| `SdkEvent.newDeposits` | `DepositInfo` list | On-chain deposits were detected. Only deposits whose `isMature` is true can be claimed, so show the rest as pending. |
| `SdkEvent.claimedDeposits` | `DepositInfo` list | Deposits were claimed into the wallet. The matching payment is emitted separately as `SdkEvent.paymentSucceeded`. |
| `SdkEvent.unclaimedDeposits` | `DepositInfo` list | The SDK could not claim these. Read `claimError` for the reason, then claim manually or refund. See [claiming on-chain deposits](onchain_claims.md). |
| `SdkEvent.autoOptimization` | `AutoOptimizationEvent` | Progress of the background leaf optimizer. Manual `optimizeLeaves` calls do not emit this. See [custom leaf optimization](optimize.md). |
| `SdkEvent.lightningAddressChanged` | `LightningAddressInfo`, unset when the address was deleted | The Lightning address changed on another device. See [receiving payments using LNURL-Pay](receive_lnurl_pay.md). |

The fields of `Payment` are described in [listing payments](list_payments.md). For
the order in which these events arrive during a receive, see
[receiving payments](receive_payment.md).

### Deposit fields

The three deposit events each carry a list of `DepositInfo`, whose fields determine
what to do next.

| Field | Meaning |
| --- | --- |
| `txid`, `vout` | The on-chain output the deposit came from. |
| `amountSats` | Deposit value in satoshis. |
| `isMature` | Whether the deposit has enough confirmations to be claimed. |
| `claimError` | Why the last claim attempt failed. Set on `SdkEvent.unclaimedDeposits`. |
| `refundTx`, `refundTxId` | The refund transaction, once one has been created. |

## Add event listener

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.add_event_listener

```swift
class SdkEventListener: EventListener {
    func onEvent(event: SdkEvent) async {
        switch event {
        case .synced:
            // Data has been synchronized with the network. When this event is received,
            // it is recommended to refresh the payment list and wallet balance.
            break
        case .newDeposits(let newDeposits):
            // Detected deposits, as DepositInfo. Only those with isMature set
            // have enough confirmations to be claimed. Show the rest as pending.
            let _ = newDeposits
        case .unclaimedDeposits(let unclaimedDeposits):
            // Deposits the SDK could not claim. Each claimError says why,
            // most often the fee exceeded the configured maximum.
            let _ = unclaimedDeposits
        case .claimedDeposits(let claimedDeposits):
            // Deposits claimed into the wallet. The resulting payment
            // arrives separately as its own event.
            let _ = claimedDeposits
        case .paymentSucceeded(let paymentSucceeded):
            // A payment completed. The cached balance is already refreshed,
            // so getInfo returns the new value.
            let _ = paymentSucceeded
        case .paymentPending(let paymentPending):
            // A payment is awaiting confirmation. It arrives again as
            // succeeded or failed once it settles.
            let _ = paymentPending
        case .paymentFailed(let paymentFailed):
            // A payment failed. payment.details carries the method-specific
            // context to show the user.
            let _ = paymentFailed
        case .autoOptimization(let optimizationEvent):
            // Background optimizer progress: started, round completed, or a
            // terminal outcome. Manual optimizeLeaves calls do not emit these.
            let _ = optimizationEvent
        case .lightningAddressChanged(let lightningAddress):
            // The lightning address changed on another device. Unset when the
            // address was deleted.
            let _ = lightningAddress
        default:
            // Handle any future event types
            break
        }
    }
}

func addEventListener(sdk: BreezSdk, listener: SdkEventListener) async -> String {
    let listenerId = await sdk.addEventListener(listener: listener)
    return listenerId
}
```



## Remove event listener

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.remove_event_listener

When you no longer need to listen to events, you can remove the listener.

```swift
func removeEventListener(sdk: BreezSdk, listenerId: String) async {
    await sdk.removeEventListener(id: listenerId)
}
```
