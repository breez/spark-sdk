# Listening to events

The SDK emits several events to provide the application with an up-to-date state of the SDK or ongoing payments.

## Event reference

| Event | Payload | What it means and what to do |
| --- | --- | --- |
| `SdkEvent.SYNCED` | none | The wallet finished syncing with the network. Refresh the balance and the payment list. See [getting the SDK info](get_info.md). |
| `SdkEvent.PAYMENT_SUCCEEDED` | `Payment` | A payment completed. The SDK refreshes its cached balance before emitting this, so `get_info` returns the new value. |
| `SdkEvent.PAYMENT_PENDING` | `Payment` | A payment is in flight. The same payment is emitted again as succeeded or failed once it settles. |
| `SdkEvent.PAYMENT_FAILED` | `Payment` | A payment failed. Its `details` carry the method-specific context to show the user. |
| `SdkEvent.NEW_DEPOSITS` | `DepositInfo` list | On-chain deposits were detected. Only deposits whose `is_mature` is true can be claimed, so show the rest as pending. |
| `SdkEvent.CLAIMED_DEPOSITS` | `DepositInfo` list | Deposits were claimed into the wallet. The matching payment is emitted separately as `SdkEvent.PAYMENT_SUCCEEDED`. |
| `SdkEvent.UNCLAIMED_DEPOSITS` | `DepositInfo` list | The SDK could not claim these. Read `claim_error` for the reason, then claim manually or refund. See [claiming on-chain deposits](onchain_claims.md). |
| `SdkEvent.AUTO_OPTIMIZATION` | `AutoOptimizationEvent` | Progress of the background leaf optimizer. Manual `optimize_leaves` calls do not emit this. See [custom leaf optimization](optimize.md). |
| `SdkEvent.LIGHTNING_ADDRESS_CHANGED` | `LightningAddressInfo`, unset when the address was deleted | The Lightning address changed on another device. See [receiving payments using LNURL-Pay](receive_lnurl_pay.md). |

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
| `claim_error` | Why the last claim attempt failed. Set on `SdkEvent.UNCLAIMED_DEPOSITS`. |
| `refund_tx`, `refund_tx_id` | The refund transaction, once one has been created. |

## Add event listener

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.add_event_listener

```python
class SdkListener(EventListener):
    async def on_event(self, event: SdkEvent):
        if isinstance(event, SdkEvent.SYNCED):
            # Data has been synchronized with the network. When this event is received,
            # it is recommended to refresh the payment list and wallet balance.
            pass
        elif isinstance(event, SdkEvent.NEW_DEPOSITS):
            # Detected deposits, as DepositInfo. Only those with is_mature set
            # have enough confirmations to be claimed. Show the rest as pending.
            new_deposits = event.new_deposits
        elif isinstance(event, SdkEvent.UNCLAIMED_DEPOSITS):
            # Deposits the SDK could not claim. Each claim_error says why,
            # most often the fee exceeded the configured maximum.
            unclaimed_deposits = event.unclaimed_deposits
        elif isinstance(event, SdkEvent.CLAIMED_DEPOSITS):
            # Deposits claimed into the wallet. The resulting payment
            # arrives separately as its own event.
            claimed_deposits = event.claimed_deposits
        elif isinstance(event, SdkEvent.PAYMENT_SUCCEEDED):
            # A payment completed. The cached balance is already refreshed,
            # so get_info returns the new value.
            payment = event.payment
        elif isinstance(event, SdkEvent.PAYMENT_PENDING):
            # A payment is awaiting confirmation. It arrives again as
            # succeeded or failed once it settles.
            pending_payment = event.payment
        elif isinstance(event, SdkEvent.PAYMENT_FAILED):
            # A payment failed. payment.details carries the method-specific
            # context to show the user.
            failed_payment = event.payment
        elif isinstance(event, SdkEvent.AUTO_OPTIMIZATION):
            # Background optimizer progress: started, round completed, or a
            # terminal outcome. Manual optimize_leaves calls do not emit these.
            optimization_event = event.optimization_event
        elif isinstance(event, SdkEvent.LIGHTNING_ADDRESS_CHANGED):
            # The lightning address changed on another device. Unset when the
            # address was deleted.
            lightning_address = event.lightning_address
        else:
            # Handle any future event types
            pass


async def add_event_listener(sdk: BreezSdk, listener: SdkListener):
    try:
        listener_id = await sdk.add_event_listener(listener=listener)
        return listener_id
    except Exception as error:
        logging.error(error)
        raise
```



## Remove event listener

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.remove_event_listener

When you no longer need to listen to events, you can remove the listener.

```python
async def remove_event_listener(sdk: BreezSdk, listener_id: str):
    try:
        await sdk.remove_event_listener(id=listener_id)
    except Exception as error:
        logging.error(error)
        raise
```
