# Listening to events

The SDK emits several events to provide the application with an up-to-date state of the SDK or ongoing payments.

## Event reference

| Event | Payload | What it means and what to do |
| --- | --- | --- |
| `SdkEvent::Synced` | none | The wallet finished syncing with the network. Refresh the balance and the payment list. See [getting the SDK info](get_info.md). |
| `SdkEvent::PaymentSucceeded` | `Payment` | A payment completed. The SDK refreshes its cached balance before emitting this, so `get_info` returns the new value. |
| `SdkEvent::PaymentPending` | `Payment` | A payment is in flight. The same payment is emitted again as succeeded or failed once it settles. |
| `SdkEvent::PaymentFailed` | `Payment` | A payment failed. Its `details` carry the method-specific context to show the user. |
| `SdkEvent::NewDeposits` | `DepositInfo` list | On-chain deposits were detected. Only deposits whose `is_mature` is true can be claimed, so show the rest as pending. |
| `SdkEvent::ClaimedDeposits` | `DepositInfo` list | Deposits were claimed into the wallet. The matching payment is emitted separately as `SdkEvent::PaymentSucceeded`. |
| `SdkEvent::UnclaimedDeposits` | `DepositInfo` list | The SDK could not claim these. Read `claim_error` for the reason, then claim manually or refund. See [claiming on-chain deposits](onchain_claims.md). |
| `SdkEvent::AutoOptimization` | `AutoOptimizationEvent` | Progress of the background leaf optimizer. Manual `optimize_leaves` calls do not emit this. See [custom leaf optimization](optimize.md). |
| `SdkEvent::LightningAddressChanged` | `LightningAddressInfo`, unset when the address was deleted | The Lightning address changed on another device. See [receiving payments using LNURL-Pay](receive_lnurl_pay.md). |

The fields of `Payment` are described in [listing payments](list_payments.md). For
the order in which these events arrive during a receive, see
[receiving payments](receive_payment.md).

### Deposit fields

The three deposit events each carry a list of `DepositInfo`, whose fields determine
what to do next.

| Field | Meaning |
| --- | --- |
| `txid`, `vout` | The on-chain output the deposit came from. |
| `amount_sats` | Deposit value in satoshis. |
| `is_mature` | Whether the deposit has enough confirmations to be claimed. |
| `claim_error` | Why the last claim attempt failed. Set on `SdkEvent::UnclaimedDeposits`. |
| `refund_tx`, `refund_tx_id` | The refund transaction, once one has been created. |

## Add event listener

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.add_event_listener

```rust
pub(crate) struct SdkEventListener {}

#[async_trait::async_trait]
impl EventListener for SdkEventListener {
    async fn on_event(&self, e: SdkEvent) {
        match e {
            SdkEvent::Synced => {
                // Data has been synchronized with the network. When this event is received,
                // it is recommended to refresh the payment list and wallet balance.
            }
            SdkEvent::NewDeposits { new_deposits } => {
                // Detected deposits, as DepositInfo. Only those with is_mature set
                // have enough confirmations to be claimed. Show the rest as pending.
            }
            SdkEvent::UnclaimedDeposits { unclaimed_deposits } => {
                // Deposits the SDK could not claim. Each claim_error says why,
                // most often the fee exceeded the configured maximum.
            }
            SdkEvent::ClaimedDeposits { claimed_deposits } => {
                // Deposits claimed into the wallet. The resulting payment
                // arrives separately as its own event.
            }
            SdkEvent::PaymentSucceeded { payment } => {
                // A payment completed. The cached balance is already refreshed,
                // so get_info returns the new value.
            }
            SdkEvent::PaymentPending { payment } => {
                // A payment is awaiting confirmation. It arrives again as
                // succeeded or failed once it settles.
            }
            SdkEvent::PaymentFailed { payment } => {
                // A payment failed. payment.details carries the method-specific
                // context to show the user.
            }
            SdkEvent::AutoOptimization { optimization_event } => {
                // Background optimizer progress: started, round completed, or a
                // terminal outcome. Manual optimize_leaves calls do not emit these.
            }
            SdkEvent::LightningAddressChanged { lightning_address } => {
                // The lightning address changed on another device. Unset when the
                // address was deleted.
            }
        }
    }
}

pub(crate) async fn add_event_listener(
    sdk: &BreezSdk,
    listener: Box<SdkEventListener>,
) -> Result<String> {
    let listener_id = sdk.add_event_listener(listener).await;
    Ok(listener_id)
}
```



## Remove event listener

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.remove_event_listener

When you no longer need to listen to events, you can remove the listener.

```rust
pub(crate) async fn remove_event_listener(sdk: &BreezSdk, listener_id: &str) -> Result<()> {
    sdk.remove_event_listener(listener_id).await;
    Ok(())
}
```
