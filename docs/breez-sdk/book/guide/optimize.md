# Custom leaf optimization

The SDK implements a configurable Spark leaf optimization process. It supports two optimization policies:

- **Maximize unilateral exit efficiency**: aims to minimize the number of leaves, reducing costs for unilaterally exiting Bitcoin funds.
- **Increase payment speed**: maintains multiple copies of each leaf denomination to reduce the need for swaps during Bitcoin payments.

## Configuring the optimization policy

The optimization behavior is controlled by the **multiplicity** setting. Setting it to 0 fully optimizes for unilateral exit efficiency, while values greater than 0 also optimize for payment speed. Higher values prioritize payment speed more aggressively, resulting in higher unilateral exit costs but faster payments, especially for bursts of transactions.

For most end-user wallets, a multiplicity of 1-5 is recommended. Values above 5 are intended for high-throughput server environments that require maximum transactions per second (TPS) and should not be used in end-user wallet applications due to the significantly higher unilateral exit costs.

See [Configuration](./config.md#optimization-configuration) to learn how to set the multiplicity.

### Impact on payment speed

Multiplicity defines how many copies of each leaf denomination the SDK maintains. A higher multiplicity provides more flexibility in leaf combinations, reducing the frequency of swaps during payments. However, the exact number of swap-free payments depends on transaction amounts and patterns.

With automatic optimization, which is enabled by default, a multiplicity of 1 (the default) works well for most single-user applications with low payment frequency, eliminating the need for swaps in the vast majority of payment scenarios. Higher multiplicities are better suited for high-volume payment processing.

### Impact on unilateral exit costs

Maintaining more leaves increases the total cost of unilaterally exiting funds, as each leaf incurs its own exit fee regardless of the leaf's value. This makes small denomination leaves cost-ineffective to exit.

**Developer note**

Keep multiplicity as low as possible while meeting your performance requirements. A high multiplicity can make unilateral exits prohibitively expensive.

## Controlling optimization timing

The optimization process reorganizes leaves by swapping them to achieve optimal denominations. During this process, funds in leaves being swapped become temporarily unavailable for payments, which can delay transaction processing.

By default, the SDK automatically triggers optimization after each payment (sent or received). For applications requiring more control, you can disable automatic optimization in the [configuration](./config.md#optimization-configuration) and drive it manually using `optimize_leaves`.

### Run optimization to completion

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.optimize_leaves

Call `optimize_leaves` with an `OptimizeLeavesRequest` using the default `OptimizationMode::Full` mode to run optimization until no further work is productive. The call blocks for the duration of the run and returns an `OptimizeLeavesResponse` whose `outcome` is `OptimizationOutcome::Completed` with the number of rounds executed. A `rounds_executed` of `0` means the wallet was already optimal at call time.

#### Rust

```rust
let outcome = sdk
    .optimize_leaves(OptimizeLeavesRequest::default())
    .await?
    .outcome;

match outcome {
    OptimizationOutcome::Completed { rounds_executed } => {
        if rounds_executed == 0 {
            info!("Optimization skipped — wallet already optimal");
        } else {
            info!("Optimization completed in {} rounds", rounds_executed);
        }
    }
    OptimizationOutcome::InProgress => {
        // Full mode runs to completion in one call, so InProgress is
        // not reachable here.
        unreachable!("Full mode never returns InProgress");
    }
}
```

#### Swift

```swift
let outcome = try await sdk.optimizeLeaves(request: OptimizeLeavesRequest(mode: .full)).outcome

switch outcome {
case .completed(let roundsExecuted):
    if roundsExecuted == 0 {
        print("Optimization skipped — wallet already optimal")
    } else {
        print("Optimization completed in \(roundsExecuted) rounds")
    }
case .inProgress:
    // Full mode runs to completion in one call, so InProgress is
    // not reachable here.
    fatalError("Full mode never returns InProgress")
}
```

#### Kotlin

```kotlin
val outcome = sdk.optimizeLeaves(OptimizeLeavesRequest(mode = OptimizationMode.FULL)).outcome

when (outcome) {
    is OptimizationOutcome.Completed -> {
        if (outcome.roundsExecuted == 0u) {
            // Log.v("Breez", "Optimization skipped — wallet already optimal")
        } else {
            // Log.v("Breez", "Optimization completed in ${outcome.roundsExecuted} rounds")
        }
    }
    is OptimizationOutcome.InProgress -> {
        // Full mode runs to completion in one call, so InProgress is
        // not reachable here.
        throw IllegalStateException("Full mode never returns InProgress")
    }
}
```

#### C#

```csharp
var outcome = (await sdk.OptimizeLeaves(new OptimizeLeavesRequest(OptimizationMode.Full))).outcome;

switch (outcome)
{
    case OptimizationOutcome.Completed { roundsExecuted: var roundsExecuted }:
        if (roundsExecuted == 0)
        {
            Console.WriteLine("Optimization skipped — wallet already optimal");
        }
        else
        {
            Console.WriteLine($"Optimization completed in {roundsExecuted} rounds");
        }
        break;
    case OptimizationOutcome.InProgress:
        // Full mode runs to completion in one call, so InProgress is
        // not reachable here.
        throw new InvalidOperationException("Full mode never returns InProgress");
}
```

#### Javascript (Wasm)

```typescript
const outcome = (await sdk.optimizeLeaves({ mode: 'full' })).outcome

switch (outcome.type) {
  case 'completed': {
    if (outcome.roundsExecuted === 0) {
      console.log('Optimization skipped — wallet already optimal')
    } else {
      console.log(`Optimization completed in ${outcome.roundsExecuted} rounds`)
    }
    break
  }
  case 'inProgress': {
    // Full mode runs to completion in one call, so inProgress is
    // not reachable here.
    break
  }
}
```

#### React Native

```typescript
const outcome = (await sdk.optimizeLeaves({ mode: OptimizationMode.Full })).outcome

if (outcome.tag === OptimizationOutcome_Tags.Completed) {
  if (outcome.inner.roundsExecuted === 0) {
    console.log('Optimization skipped — wallet already optimal')
  } else {
    console.log(`Optimization completed in ${outcome.inner.roundsExecuted} rounds`)
  }
} else if (outcome.tag === OptimizationOutcome_Tags.InProgress) {
  // Full mode runs to completion in one call, so InProgress is
  // not reachable here.
}
```

#### Flutter

```dart
final outcome = (await sdk.optimizeLeaves(
        request: OptimizeLeavesRequest(mode: OptimizationMode.full)))
    .outcome;

switch (outcome) {
  case OptimizationOutcome_Completed(:final roundsExecuted):
    if (roundsExecuted == 0) {
      print("Optimization skipped — wallet already optimal");
    } else {
      print("Optimization completed in $roundsExecuted rounds");
    }
    break;
  case OptimizationOutcome_InProgress():
    // Full mode runs to completion in one call, so InProgress is
    // not reachable here.
    throw StateError("Full mode never returns InProgress");
}
```

#### Python

```python
response = await sdk.optimize_leaves(OptimizeLeavesRequest(mode=OptimizationMode.FULL))
outcome = response.outcome

if isinstance(outcome, OptimizationOutcome.COMPLETED):
    if outcome.rounds_executed == 0:
        logging.debug("Optimization skipped — wallet already optimal")
    else:
        logging.debug(f"Optimization completed in {outcome.rounds_executed} rounds")
elif isinstance(outcome, OptimizationOutcome.IN_PROGRESS):
    raise AssertionError("Full mode never returns IN_PROGRESS")
```

#### Go

```go
response, err := sdk.OptimizeLeaves(breez_sdk_spark.OptimizeLeavesRequest{
	Mode: breez_sdk_spark.OptimizationModeFull,
})
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
	}
	return err
}

switch o := response.Outcome.(type) {
case breez_sdk_spark.OptimizationOutcomeCompleted:
	if o.RoundsExecuted == 0 {
		log.Printf("Optimization skipped — wallet already optimal")
	} else {
		log.Printf("Optimization completed in %v rounds", o.RoundsExecuted)
	}
case breez_sdk_spark.OptimizationOutcomeInProgress:
	// Full mode runs to completion in one call, so InProgress is
	// not reachable here.
	log.Panicf("Full mode never returns InProgress")
}
```



### Run optimization one round at a time

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.optimize_leaves

To display progress or cancel between rounds, pass an `OptimizeLeavesRequest` with `OptimizationMode::SingleRound`. Each call executes one round and the response `outcome` is `OptimizationOutcome::InProgress` (more work remains) or `OptimizationOutcome::Completed` (terminal — either the planner confirmed this swap finished optimization, or a `rounds_executed` of `0` indicates the wallet was already optimal). Cancel between rounds simply by stopping the loop.

#### Rust

```rust
let mut rounds_executed = 0u32;
loop {
    let request = OptimizeLeavesRequest {
        mode: OptimizationMode::SingleRound,
    };
    match sdk.optimize_leaves(request).await?.outcome {
        OptimizationOutcome::InProgress => {
            rounds_executed += 1;
            info!("Executed round {}", rounds_executed);
        }
        OptimizationOutcome::Completed {
            rounds_executed: this_round,
        } => {
            rounds_executed += this_round;
            if rounds_executed == 0 {
                info!("Optimization skipped — wallet already optimal");
            } else {
                info!("Optimization done after {} rounds", rounds_executed);
            }
            break;
        }
    }
}
```

#### Swift

```swift
var roundsExecuted: UInt32 = 0
loop: while true {
    let outcome = try await sdk.optimizeLeaves(
        request: OptimizeLeavesRequest(mode: .singleRound)
    ).outcome

    switch outcome {
    case .inProgress:
        roundsExecuted += 1
        print("Executed round \(roundsExecuted)")
    case .completed(let thisRound):
        roundsExecuted += thisRound
        if roundsExecuted == 0 {
            print("Optimization skipped — wallet already optimal")
        } else {
            print("Optimization done after \(roundsExecuted) rounds")
        }
        break loop
    }
}
```

#### Kotlin

```kotlin
var roundsExecuted: UInt = 0u
while (true) {
    val outcome = sdk.optimizeLeaves(
        OptimizeLeavesRequest(mode = OptimizationMode.SINGLE_ROUND)
    ).outcome
    when (outcome) {
        is OptimizationOutcome.InProgress -> {
            roundsExecuted += 1u
            // Log.v("Breez", "Executed round $roundsExecuted")
        }
        is OptimizationOutcome.Completed -> {
            roundsExecuted += outcome.roundsExecuted
            if (roundsExecuted == 0u) {
                // Log.v("Breez", "Optimization skipped — wallet already optimal")
            } else {
                // Log.v("Breez", "Optimization done after $roundsExecuted rounds")
            }
            break
        }
    }
}
```

#### C#

```csharp
uint roundsExecuted = 0;
while (true)
{
    var outcome = (await sdk.OptimizeLeaves(
        new OptimizeLeavesRequest(OptimizationMode.SingleRound)
    )).outcome;

    if (outcome is OptimizationOutcome.InProgress)
    {
        roundsExecuted += 1;
        Console.WriteLine($"Executed round {roundsExecuted}");
    }
    else if (outcome is OptimizationOutcome.Completed { roundsExecuted: var n })
    {
        roundsExecuted += n;
        if (roundsExecuted == 0)
        {
            Console.WriteLine("Optimization skipped — wallet already optimal");
        }
        else
        {
            Console.WriteLine($"Optimization done after {roundsExecuted} rounds");
        }
        break;
    }
}
```

#### Javascript (Wasm)

```typescript
let roundsExecuted = 0
while (true) {
  const outcome = (await sdk.optimizeLeaves({ mode: 'singleRound' })).outcome

  if (outcome.type === 'inProgress') {
    roundsExecuted += 1
    console.log(`Executed round ${roundsExecuted}`)
  } else if (outcome.type === 'completed') {
    roundsExecuted += outcome.roundsExecuted
    if (roundsExecuted === 0) {
      console.log('Optimization skipped — wallet already optimal')
    } else {
      console.log(`Optimization done after ${roundsExecuted} rounds`)
    }
    break
  }
}
```

#### React Native

```typescript
let roundsExecuted = 0
while (true) {
  const outcome: OptimizationOutcome = (
    await sdk.optimizeLeaves({ mode: OptimizationMode.SingleRound })
  ).outcome

  if (outcome.tag === OptimizationOutcome_Tags.InProgress) {
    roundsExecuted += 1
    console.log(`Executed round ${roundsExecuted}`)
  } else if (outcome.tag === OptimizationOutcome_Tags.Completed) {
    roundsExecuted += outcome.inner.roundsExecuted
    if (roundsExecuted === 0) {
      console.log('Optimization skipped — wallet already optimal')
    } else {
      console.log(`Optimization done after ${roundsExecuted} rounds`)
    }
    break
  }
}
```

#### Flutter

```dart
var roundsExecuted = 0;
while (true) {
  final outcome = (await sdk.optimizeLeaves(
      request: OptimizeLeavesRequest(mode: OptimizationMode.singleRound))).outcome;
  switch (outcome) {
    case OptimizationOutcome_InProgress():
      roundsExecuted += 1;
      print("Executed round $roundsExecuted");
      break;
    case OptimizationOutcome_Completed(roundsExecuted: var n):
      roundsExecuted += n;
      if (roundsExecuted == 0) {
        print("Optimization skipped — wallet already optimal");
      } else {
        print("Optimization done after $roundsExecuted rounds");
      }
      return;
  }
}
```

#### Python

```python
rounds_executed = 0
while True:
    response = await sdk.optimize_leaves(
        OptimizeLeavesRequest(mode=OptimizationMode.SINGLE_ROUND)
    )
    outcome = response.outcome
    if isinstance(outcome, OptimizationOutcome.IN_PROGRESS):
        rounds_executed += 1
        logging.debug(f"Executed round {rounds_executed}")
    elif isinstance(outcome, OptimizationOutcome.COMPLETED):
        rounds_executed += outcome.rounds_executed
        if rounds_executed == 0:
            logging.debug("Optimization skipped — wallet already optimal")
        else:
            logging.debug(f"Optimization done after {rounds_executed} rounds")
        break
```

#### Go

```go
var roundsExecuted uint32 = 0
for {
	response, err := sdk.OptimizeLeaves(breez_sdk_spark.OptimizeLeavesRequest{
		Mode: breez_sdk_spark.OptimizationModeSingleRound,
	})
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
		}
		return err
	}

	switch o := response.Outcome.(type) {
	case breez_sdk_spark.OptimizationOutcomeInProgress:
		roundsExecuted += 1
		log.Printf("Executed round %v", roundsExecuted)
	case breez_sdk_spark.OptimizationOutcomeCompleted:
		roundsExecuted += o.RoundsExecuted
		if roundsExecuted == 0 {
			log.Printf("Optimization skipped — wallet already optimal")
		} else {
			log.Printf("Optimization done after %v rounds", roundsExecuted)
		}
		return nil
	}
}
```



**Developer note**

If `optimize_leaves` is invoked while another optimization run (auto or manual) is already in flight, it returns `SdkError::OptimizationAlreadyRunning`. The SDK may also preempt a manual run to free leaves for a higher-priority payment, in which case the call returns `SdkError::OptimizationCancelled`.

## Auto-optimization events

When automatic optimization is enabled, the SDK emits `SdkEvent::AutoOptimization` events so your application can track the background optimizer's progress. Manual `optimize_leaves` calls do not emit these events — inspect their return value instead. See [Listening to events](./events.md) for subscription instructions.

### Rust

```rust
match event {
    AutoOptimizationEvent::Started { total_rounds } => {
        info!("Auto-optimization started with {} rounds", total_rounds);
    }
    AutoOptimizationEvent::RoundCompleted {
        current_round,
        total_rounds,
    } => {
        info!(
            "Auto-optimization round {} of {} completed",
            current_round, total_rounds
        );
    }
    AutoOptimizationEvent::Completed => {
        info!("Auto-optimization completed successfully");
    }
    AutoOptimizationEvent::Cancelled => {
        info!("Auto-optimization was cancelled");
    }
    AutoOptimizationEvent::Failed { error } => {
        info!("Auto-optimization failed: {}", error);
    }
    AutoOptimizationEvent::Skipped => {
        info!("Auto-optimization was skipped because leaves are already optimal");
    }
}
```

### Swift

```swift
switch event {
case .started(let totalRounds):
    print("Auto-optimization started with \(totalRounds) rounds")
case .roundCompleted(let currentRound, let totalRounds):
    print("Auto-optimization round \(currentRound) of \(totalRounds) completed")
case .completed:
    print("Auto-optimization completed successfully")
case .cancelled:
    print("Auto-optimization was cancelled")
case .failed(let error):
    print("Auto-optimization failed: \(error)")
case .skipped:
    print("Auto-optimization was skipped because leaves are already optimal")
}
```

### Kotlin

```kotlin
when (optimizationEvent) {
    is AutoOptimizationEvent.Started -> {
        // Log.v("Breez", "Auto-optimization started with ${optimizationEvent.totalRounds} rounds")
    }
    is AutoOptimizationEvent.RoundCompleted -> {
        // Log.v("Breez", "Auto-optimization round
        // ${optimizationEvent.currentRound} of
        // ${optimizationEvent.totalRounds} completed")
    }
    is AutoOptimizationEvent.Completed -> {
        // Log.v("Breez", "Auto-optimization completed successfully")
    }
    is AutoOptimizationEvent.Cancelled -> {
        // Log.v("Breez", "Auto-optimization was cancelled")
    }
    is AutoOptimizationEvent.Failed -> {
        // Log.v("Breez", "Auto-optimization failed: ${optimizationEvent.error}")
    }
    is AutoOptimizationEvent.Skipped -> {
        // Log.v("Breez", "Auto-optimization was skipped because leaves are already optimal")
    }
}
```

### C#

```csharp
switch (optimizationEvent)
{
    case AutoOptimizationEvent.Started { totalRounds: var totalRounds }:
        Console.WriteLine($"Auto-optimization started with {totalRounds} rounds");
        break;
    case AutoOptimizationEvent.RoundCompleted
    {
        currentRound: var currentRound,
        totalRounds: var totalRounds
    }:
        Console.WriteLine($"Auto-optimization round {currentRound} of {totalRounds} completed");
        break;
    case AutoOptimizationEvent.Completed:
        Console.WriteLine("Auto-optimization completed successfully");
        break;
    case AutoOptimizationEvent.Cancelled:
        Console.WriteLine("Auto-optimization was cancelled");
        break;
    case AutoOptimizationEvent.Failed { error: var error }:
        Console.WriteLine($"Auto-optimization failed: {error}");
        break;
    case AutoOptimizationEvent.Skipped:
        Console.WriteLine("Auto-optimization was skipped because leaves are already optimal");
        break;
}
```

### Javascript (Wasm)

```typescript
switch (event.type) {
  case 'started': {
    console.log(`Auto-optimization started with ${event.totalRounds} rounds`)
    break
  }
  case 'roundCompleted': {
    console.log(`Auto-optimization round ${event.currentRound} of ${event.totalRounds} completed`)
    break
  }
  case 'completed': {
    console.log('Auto-optimization completed successfully')
    break
  }
  case 'cancelled': {
    console.log('Auto-optimization was cancelled')
    break
  }
  case 'failed': {
    console.log(`Auto-optimization failed: ${event.error}`)
    break
  }
  case 'skipped': {
    console.log('Auto-optimization was skipped because leaves are already optimal')
    break
  }
}
```

### React Native

```typescript
if (optimizationEvent.tag === AutoOptimizationEvent_Tags.Started) {
  console.log(`Auto-optimization started with ${optimizationEvent.inner.totalRounds} rounds`)
} else if (optimizationEvent.tag === AutoOptimizationEvent_Tags.RoundCompleted) {
  console.log(
    `Auto-optimization round ${optimizationEvent.inner.currentRound} of ` +
    `${optimizationEvent.inner.totalRounds} completed`
  )
} else if (optimizationEvent.tag === AutoOptimizationEvent_Tags.Completed) {
  console.log('Auto-optimization completed successfully')
} else if (optimizationEvent.tag === AutoOptimizationEvent_Tags.Cancelled) {
  console.log('Auto-optimization was cancelled')
} else if (optimizationEvent.tag === AutoOptimizationEvent_Tags.Failed) {
  console.log(`Auto-optimization failed: ${optimizationEvent.inner.error}`)
} else if (optimizationEvent.tag === AutoOptimizationEvent_Tags.Skipped) {
  console.log('Auto-optimization was skipped because leaves are already optimal')
}
```

### Flutter

```dart
switch (optimizationEvent) {
  case AutoOptimizationEvent_Started(totalRounds: var totalRounds):
    print("Auto-optimization started with $totalRounds rounds");
    break;
  case AutoOptimizationEvent_RoundCompleted(
      currentRound: var currentRound,
      totalRounds: var totalRounds
    ):
    print("Auto-optimization round $currentRound of $totalRounds completed");
    break;
  case AutoOptimizationEvent_Completed():
    print("Auto-optimization completed successfully");
    break;
  case AutoOptimizationEvent_Cancelled():
    print("Auto-optimization was cancelled");
    break;
  case AutoOptimizationEvent_Failed(error: var error):
    print("Auto-optimization failed: $error");
    break;
  case AutoOptimizationEvent_Skipped():
    print("Auto-optimization was skipped because leaves are already optimal");
    break;
}
```

### Python

```python
if isinstance(event, AutoOptimizationEvent.STARTED):
    logging.debug(f"Auto-optimization started with {event.total_rounds} rounds")
elif isinstance(event, AutoOptimizationEvent.ROUND_COMPLETED):
    logging.debug(f"Auto-optimization round {event.current_round} of "
        f"{event.total_rounds} completed")
elif isinstance(event, AutoOptimizationEvent.COMPLETED):
    logging.debug("Auto-optimization completed successfully")
elif isinstance(event, AutoOptimizationEvent.CANCELLED):
    logging.debug("Auto-optimization was cancelled")
elif isinstance(event, AutoOptimizationEvent.FAILED):
    logging.debug(f"Auto-optimization failed: {event.error}")
elif isinstance(event, AutoOptimizationEvent.SKIPPED):
    logging.debug("Auto-optimization was skipped because leaves are already optimal")
```

### Go

```go
switch event := optimizationEvent.(type) {
case breez_sdk_spark.AutoOptimizationEventStarted:
	log.Printf("Auto-optimization started with %v rounds", event.TotalRounds)
case breez_sdk_spark.AutoOptimizationEventRoundCompleted:
	log.Printf("Auto-optimization round %v of %v completed", event.CurrentRound, event.TotalRounds)
case breez_sdk_spark.AutoOptimizationEventCompleted:
	log.Printf("Auto-optimization completed successfully")
case breez_sdk_spark.AutoOptimizationEventCancelled:
	log.Printf("Auto-optimization was cancelled")
case breez_sdk_spark.AutoOptimizationEventFailed:
	log.Printf("Auto-optimization failed: %v", event.Error)
case breez_sdk_spark.AutoOptimizationEventSkipped:
	log.Printf("Auto-optimization was skipped because leaves are already optimal")
}
```



---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
