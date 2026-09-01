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
| `SdkEvent::UnilateralExitStateChanged` | none | An exit state exported earlier is now out of date. Export it again. See [unilateral exit](unilateral_exit.md). |

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
| `refund_state` | How far the refund has got towards the network. Read it through `list_unclaimed_deposits`: a refunded deposit no longer appears in these events. See [tracking a refund](onchain_claims.md#tracking-a-refund). |
| `instant_claim_status` | State of an instant (0-conf) claim attempt. Unset when none was attempted. |

## Add event listener

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.add_event_listener

### Rust

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
                // Deposits claimed into the wallet. An instant (0-conf) claim is
                // reported here on submission and settles shortly after.
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
            SdkEvent::UnilateralExitStateChanged => {
                // The unilateral exit state changed, so a previously exported
                // one is now out of date. Export it again.
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

### Swift

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
            // Deposits claimed into the wallet. An instant (0-conf) claim is
            // reported here on submission and settles shortly after.
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
        case .unilateralExitStateChanged:
            // The unilateral exit state changed, so a previously exported
            // one is now out of date. Export it again.
            break
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

### Kotlin

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

### C#

```csharp
class SdkListener : EventListener
{
    public async Task OnEvent(SdkEvent sdkEvent)
    {
        switch (sdkEvent)
        {
            case SdkEvent.Synced syncedEvent:
                // Data has been synchronized with the network. When this event is received,
                // it is recommended to refresh the payment list and wallet balance.
                break;

            case SdkEvent.NewDeposits newDepositsEvent:
                // Detected deposits, as DepositInfo. Only those with IsMature set
                // have enough confirmations to be claimed. Show the rest as pending.
                var newDeposits = newDepositsEvent.newDeposits;
                break;

            case SdkEvent.UnclaimedDeposits unclaimedDepositsEvent:
                // Deposits the SDK could not claim. Each ClaimError says why,
                // most often the fee exceeded the configured maximum.
                var unclaimedDeposits = unclaimedDepositsEvent.unclaimedDeposits;
                break;

            case SdkEvent.ClaimedDeposits claimedDepositsEvent:
                // Deposits claimed into the wallet. An instant (0-conf) claim is
                // reported here on submission and settles shortly after.
                var claimedDeposits = claimedDepositsEvent.claimedDeposits;
                break;

            case SdkEvent.PaymentSucceeded paymentSucceededEvent:
                // A payment completed. The cached balance is already refreshed,
                // so GetInfo returns the new value.
                var payment = paymentSucceededEvent.payment;
                break;

            case SdkEvent.PaymentPending paymentPendingEvent:
                // A payment is awaiting confirmation. It arrives again as
                // succeeded or failed once it settles.
                var pendingPayment = paymentPendingEvent.payment;
                break;

            case SdkEvent.PaymentFailed paymentFailedEvent:
                // A payment failed. payment.Details carries the method-specific
                // context to show the user.
                var failedPayment = paymentFailedEvent.payment;
                break;

            case SdkEvent.AutoOptimization optimizationEvent:
                // Background optimizer progress: started, round completed, or a
                // terminal outcome. Manual OptimizeLeaves calls do not emit these.
                var optimization = optimizationEvent.optimizationEvent;
                break;

            case SdkEvent.LightningAddressChanged lightningAddressChangedEvent:
                // The lightning address changed on another device. Unset when the
                // address was deleted.
                var lightningAddress = lightningAddressChangedEvent.lightningAddress;
                break;

            case SdkEvent.UnilateralExitStateChanged unilateralExitStateChangedEvent:
                // The unilateral exit state changed, so a previously exported
                // one is now out of date. Export it again.
                break;

            default:
                // Handle any future event types
                break;
        }
    }
}

async Task<string> AddEventListener(BreezSdk sdk, SdkListener listener)
{
    var listenerId = await sdk.AddEventListener(listener: listener);
    return listenerId;
}
```

### Javascript (Wasm)

```typescript
class JsEventListener {
  onEvent = async (event: SdkEvent) => {
    switch (event.type) {
      case 'synced': {
        // Data has been synchronized with the network. When this event is received,
        // it is recommended to refresh the payment list and wallet balance.
        break
      }
      case 'newDeposits': {
        // Detected deposits, as DepositInfo. Only those with isMature set
        // have enough confirmations to be claimed. Show the rest as pending.
        const newDeposits = event.newDeposits
        break
      }
      case 'unclaimedDeposits': {
        // Deposits the SDK could not claim. Each claimError says why,
        // most often the fee exceeded the configured maximum.
        const unclaimedDeposits = event.unclaimedDeposits
        break
      }
      case 'claimedDeposits': {
        // Deposits claimed into the wallet. An instant (0-conf) claim is
        // reported here on submission and settles shortly after.
        const claimedDeposits = event.claimedDeposits
        break
      }
      case 'paymentSucceeded': {
        // A payment completed. The cached balance is already refreshed,
        // so getInfo returns the new value.
        const payment = event.payment
        break
      }
      case 'paymentPending': {
        // A payment is awaiting confirmation. It arrives again as
        // succeeded or failed once it settles.
        const pendingPayment = event.payment
        break
      }
      case 'paymentFailed': {
        // A payment failed. payment.details carries the method-specific
        // context to show the user.
        const failedPayment = event.payment
        break
      }
      case 'autoOptimization': {
        // Background optimizer progress: started, round completed, or a
        // terminal outcome. Manual optimizeLeaves calls do not emit these.
        const optimizationEvent = event.optimizationEvent
        break
      }
      case 'lightningAddressChanged': {
        // The lightning address changed on another device. Unset when the
        // address was deleted.
        const lightningAddress = event.lightningAddress
        break
      }
      case 'unilateralExitStateChanged': {
        // The unilateral exit state changed, so a previously exported
        // one is now out of date. Export it again.
        break
      }
      default: {
        // Handle any future event types
        break
      }
    }
  }
}

const eventListener = new JsEventListener()

const listenerId = await sdk.addEventListener(eventListener)
```

### React Native

```typescript
class JsEventListener {
  onEvent = async (event: SdkEvent) => {
    if (event.tag === SdkEvent_Tags.Synced) {
      // Data has been synchronized with the network. When this event is received,
      // it is recommended to refresh the payment list and wallet balance.
    } else if (event.tag === SdkEvent_Tags.NewDeposits) {
      // Detected deposits, as DepositInfo. Only those with isMature set
      // have enough confirmations to be claimed. Show the rest as pending.
      const newDeposits = event.inner.newDeposits
    } else if (event.tag === SdkEvent_Tags.UnclaimedDeposits) {
      // Deposits the SDK could not claim. Each claimError says why,
      // most often the fee exceeded the configured maximum.
      const unclaimedDeposits = event.inner.unclaimedDeposits
    } else if (event.tag === SdkEvent_Tags.ClaimedDeposits) {
      // Deposits claimed into the wallet. An instant (0-conf) claim is
      // reported here on submission and settles shortly after.
      const claimedDeposits = event.inner.claimedDeposits
    } else if (event.tag === SdkEvent_Tags.PaymentSucceeded) {
      // A payment completed. The cached balance is already refreshed,
      // so getInfo returns the new value.
      const payment = event.inner.payment
    } else if (event.tag === SdkEvent_Tags.PaymentPending) {
      // A payment is awaiting confirmation. It arrives again as
      // succeeded or failed once it settles.
      const pendingPayment = event.inner.payment
    } else if (event.tag === SdkEvent_Tags.PaymentFailed) {
      // A payment failed. payment.details carries the method-specific
      // context to show the user.
      const failedPayment = event.inner.payment
    } else if (event.tag === SdkEvent_Tags.AutoOptimization) {
      // Background optimizer progress: started, round completed, or a
      // terminal outcome. Manual optimizeLeaves calls do not emit these.
      const optimizationEvent = event.inner.optimizationEvent
    } else if (event.tag === SdkEvent_Tags.LightningAddressChanged) {
      // The lightning address changed on another device. Unset when the
      // address was deleted.
      const lightningAddress = event.inner.lightningAddress
    } else if (event.tag === SdkEvent_Tags.UnilateralExitStateChanged) {
      // The unilateral exit state changed, so a previously exported
      // one is now out of date. Export it again.
    } else {
      // Handle any future event types
    }
  }
}

const eventListener = new JsEventListener()

const listenerId = await sdk.addEventListener(eventListener)
```

### Flutter

```dart
StreamSubscription<SdkEvent>? _eventSubscription;
Stream<SdkEvent>? _eventStream;

// Initializes SDK event stream.
//
// Call once on your Dart entrypoint file, e.g.; `lib/main.dart`
// or singleton SDK service. It is recommended to use a single instance
// of the SDK across your Flutter app.
void initializeEventsStream(BreezSdk sdk) {
  _eventStream ??= sdk.addEventListener().asBroadcastStream();
}

final _eventStreamController = StreamController<SdkEvent>.broadcast();
Stream<SdkEvent> get eventStream => _eventStreamController.stream;

// Subscribe to the event stream
void subscribeToEventStream() {
  _eventSubscription = _eventStream?.listen((sdkEvent) {
    switch (sdkEvent) {
      case SdkEvent_Synced():
        // Data has been synchronized with the network. When this event is received,
        // it is recommended to refresh the payment list and wallet balance.
        break;
      case SdkEvent_NewDeposits(:final newDeposits):
        // Detected deposits, as DepositInfo. Only those with isMature set
        // have enough confirmations to be claimed. Show the rest as pending.
        final _ = newDeposits;
        break;
      case SdkEvent_UnclaimedDeposits(:final unclaimedDeposits):
        // Deposits the SDK could not claim. Each claimError says why,
        // most often the fee exceeded the configured maximum.
        final _ = unclaimedDeposits;
        break;
      case SdkEvent_ClaimedDeposits(:final claimedDeposits):
        // Deposits claimed into the wallet. An instant (0-conf) claim is
        // reported here on submission and settles shortly after.
        final _ = claimedDeposits;
        break;
      case SdkEvent_PaymentSucceeded(:final payment):
        // A payment completed. The cached balance is already refreshed,
        // so getInfo returns the new value.
        final _ = payment;
        break;
      case SdkEvent_PaymentPending(:final payment):
        // A payment is awaiting confirmation. It arrives again as
        // succeeded or failed once it settles.
        final _ = payment;
        break;
      case SdkEvent_PaymentFailed(:final payment):
        // A payment failed. payment.details carries the method-specific
        // context to show the user.
        final _ = payment;
        break;
      case SdkEvent_AutoOptimization(:final optimizationEvent):
        // Background optimizer progress: started, round completed, or a
        // terminal outcome. Manual optimizeLeaves calls do not emit these.
        final _ = optimizationEvent;
        break;
      case SdkEvent_LightningAddressChanged(:final lightningAddress):
        // The lightning address changed on another device. Unset when the
        // address was deleted.
        final _ = lightningAddress;
        break;
      case SdkEvent_UnilateralExitStateChanged():
        // The unilateral exit state changed, so a previously exported
        // one is now out of date. Export it again.
        break;
    }
    _eventStreamController.add(sdkEvent);
  }, onError: (e) {
    _eventStreamController.addError(e);
  });
}
```

### Python

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
            # Deposits claimed into the wallet. An instant (0-conf) claim is
            # reported here on submission and settles shortly after.
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
        elif isinstance(event, SdkEvent.UNILATERAL_EXIT_STATE_CHANGED):
            # The unilateral exit state changed, so a previously exported
            # one is now out of date. Export it again.
            pass
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

### Go

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

### Rust

```rust
pub(crate) async fn remove_event_listener(sdk: &BreezSdk, listener_id: &str) -> Result<()> {
    sdk.remove_event_listener(listener_id).await;
    Ok(())
}
```

### Swift

```swift
func removeEventListener(sdk: BreezSdk, listenerId: String) async {
    await sdk.removeEventListener(id: listenerId)
}
```

### Kotlin

```kotlin
suspend fun removeEventListener(sdk: BreezSdk, listenerId: String)  {
    try {
        sdk.removeEventListener(listenerId)
    } catch (e: Exception) {
        // handle error
    }
}
```

### C#

```csharp
async Task RemoveEventListener(BreezSdk sdk, string listenerId)
{
    await sdk.RemoveEventListener(id: listenerId);
}
```

### Javascript (Wasm)

```typescript
await sdk.removeEventListener(listenerId)
```

### React Native

```typescript
await sdk.removeEventListener(listenerId)
```

### Flutter

```dart
void unsubscribeFromEventStream() {
  _eventSubscription?.cancel();
}
```

### Python

```python
async def remove_event_listener(sdk: BreezSdk, listener_id: str):
    try:
        await sdk.remove_event_listener(id=listener_id)
    except Exception as error:
        logging.error(error)
        raise
```

### Go

```go
func RemoveEventListener(sdk *breez_sdk_spark.BreezSdk, listenerId string) bool {
	return sdk.RemoveEventListener(listenerId)
}
```



---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
