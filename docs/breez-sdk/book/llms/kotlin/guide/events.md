# Listening to events

The SDK emits several events to provide the application with an up-to-date state of the SDK or ongoing payments.

## Event reference

| Event | Payload | What it means and what to do |
| --- | --- | --- |
| `SdkEvent.Synced` | none | The wallet finished syncing with the network. Refresh the balance and the payment list. See [getting the SDK info](get_info.md). |
| `SdkEvent.PaymentSucceeded` | `Payment` | A payment completed. The SDK refreshes its cached balance before emitting this, so `getInfo` returns the new value. |
| `SdkEvent.PaymentPending` | `Payment` | A payment is in flight. The same payment is emitted again as succeeded or failed once it settles. |
| `SdkEvent.PaymentFailed` | `Payment` | A payment failed. Its `details` carry the method-specific context to show the user. |
| `SdkEvent.NewDeposits` | `DepositInfo` list | On-chain deposits were detected. Only deposits whose `isMature` is true can be claimed, so show the rest as pending. |
| `SdkEvent.ClaimedDeposits` | `DepositInfo` list | Deposits were claimed into the wallet. The matching payment is emitted separately as `SdkEvent.PaymentSucceeded`. |
| `SdkEvent.UnclaimedDeposits` | `DepositInfo` list | The SDK could not claim these. Read `claimError` for the reason, then claim manually or refund. See [claiming on-chain deposits](onchain_claims.md). |
| `SdkEvent.AutoOptimization` | `AutoOptimizationEvent` | Progress of the background leaf optimizer. Manual `optimizeLeaves` calls do not emit this. See [custom leaf optimization](optimize.md). |
| `SdkEvent.LightningAddressChanged` | `LightningAddressInfo`, unset when the address was deleted | The Lightning address changed on another device. See [receiving payments using LNURL-Pay](receive_lnurl_pay.md). |
| `SdkEvent.UnilateralExitStateChanged` | none | An exit state exported earlier is now out of date. Export it again. See [unilateral exit](unilateral_exit.md). |

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
| `claimError` | Why the last claim attempt failed. Set on `SdkEvent.UnclaimedDeposits`. |
| `refundTx`, `refundTxId` | The refund transaction, once one has been created. |
| `instantClaimStatus` | State of an instant (0-conf) claim attempt. Unset when none was attempted. |

## Add event listener

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.add_event_listener

```kotlin
class SdkListener : EventListener {
    override suspend fun onEvent(e: SdkEvent) {
        when (e) {
            is SdkEvent.Synced -> {
                // Data has been synchronized with the network. When this event is received,
                // it is recommended to refresh the payment list and wallet balance.
            }
            is SdkEvent.NewDeposits -> {
                // Detected deposits, as DepositInfo. Only those with isMature set
                // have enough confirmations to be claimed. Show the rest as pending.
                val newDeposits = e.newDeposits
            }
            is SdkEvent.UnclaimedDeposits -> {
                // Deposits the SDK could not claim. Each claimError says why,
                // most often the fee exceeded the configured maximum.
                val unclaimedDeposits = e.unclaimedDeposits
            }
            is SdkEvent.ClaimedDeposits -> {
                // Deposits claimed into the wallet. An instant (0-conf) claim is
                // reported here on submission and settles shortly after.
                val claimedDeposits = e.claimedDeposits
            }
            is SdkEvent.PaymentSucceeded -> {
                // A payment completed. The cached balance is already refreshed,
                // so getInfo returns the new value.
                val payment = e.payment
            }
            is SdkEvent.PaymentPending -> {
                // A payment is awaiting confirmation. It arrives again as
                // succeeded or failed once it settles.
                val pendingPayment = e.payment
            }
            is SdkEvent.PaymentFailed -> {
                // A payment failed. payment.details carries the method-specific
                // context to show the user.
                val failedPayment = e.payment
            }
            is SdkEvent.AutoOptimization -> {
                // Background optimizer progress: started, round completed, or a
                // terminal outcome. Manual optimizeLeaves calls do not emit these.
                val optimizationEvent = e.optimizationEvent
            }
            is SdkEvent.LightningAddressChanged -> {
                // The lightning address changed on another device. Unset when the
                // address was deleted.
                val lightningAddress = e.lightningAddress
            }
            is SdkEvent.UnilateralExitStateChanged -> {
                // The unilateral exit state changed, so a previously exported
                // one is now out of date. Export it again.
            }
            else -> {
                // Handle any future event types
            }
        }
    }
}

suspend fun addEventListener(sdk: BreezSdk, listener: SdkListener): String? {
    try {
        val listenerId = sdk.addEventListener(listener)
        return listenerId
    } catch (e: Exception) {
        // handle error
        return null
    }
}
```



## Remove event listener

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.remove_event_listener

When you no longer need to listen to events, you can remove the listener.

```kotlin
suspend fun removeEventListener(sdk: BreezSdk, listenerId: String)  {
    try {
        sdk.removeEventListener(listenerId)
    } catch (e: Exception) {
        // handle error
    }
}
```
