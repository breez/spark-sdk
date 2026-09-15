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

By default, the SDK automatically triggers optimization after each payment (sent or received). For applications requiring more control, you can disable automatic optimization in the [configuration](./config.md#optimization-configuration) and drive it manually using `optimizeLeaves`.

### Run optimization to completion

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.optimize_leaves

Call `optimizeLeaves` with an `OptimizeLeavesRequest` using the default `OptimizationMode.full` mode to run optimization until no further work is productive. The call blocks for the duration of the run and returns an `OptimizeLeavesResponse` whose `outcome` is `OptimizationOutcome.completed` with the number of rounds executed. A `roundsExecuted` of `0` means the wallet was already optimal at call time.

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



### Run optimization one round at a time

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.optimize_leaves

To display progress or cancel between rounds, pass an `OptimizeLeavesRequest` with `OptimizationMode.singleRound`. Each call executes one round and the response `outcome` is `OptimizationOutcome.inProgress` (more work remains) or `OptimizationOutcome.completed` (terminal — either the planner confirmed this swap finished optimization, or a `roundsExecuted` of `0` indicates the wallet was already optimal). Cancel between rounds simply by stopping the loop.

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



**Developer note**

If `optimizeLeaves` is invoked while another optimization run (auto or manual) is already in flight, it returns `SdkError.optimizationAlreadyRunning`. The SDK may also preempt a manual run to free leaves for a higher-priority payment, in which case the call returns `SdkError.optimizationCancelled`.

## Auto-optimization events

When automatic optimization is enabled, the SDK emits `SdkEvent.autoOptimization` events so your application can track the background optimizer's progress. Manual `optimizeLeaves` calls do not emit these events — inspect their return value instead. See [Listening to events](./events.md) for subscription instructions.

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
