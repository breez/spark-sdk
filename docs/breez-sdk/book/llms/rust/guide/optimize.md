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



### Run optimization one round at a time

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.optimize_leaves

To display progress or cancel between rounds, pass an `OptimizeLeavesRequest` with `OptimizationMode::SingleRound`. Each call executes one round and the response `outcome` is `OptimizationOutcome::InProgress` (more work remains) or `OptimizationOutcome::Completed` (terminal — either the planner confirmed this swap finished optimization, or a `rounds_executed` of `0` indicates the wallet was already optimal). Cancel between rounds simply by stopping the loop.

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



**Developer note**

If `optimize_leaves` is invoked while another optimization run (auto or manual) is already in flight, it returns `SdkError::OptimizationAlreadyRunning`. The SDK may also preempt a manual run to free leaves for a higher-priority payment, in which case the call returns `SdkError::OptimizationCancelled`.

## Auto-optimization events

When automatic optimization is enabled, the SDK emits `SdkEvent::AutoOptimization` events so your application can track the background optimizer's progress. Manual `optimize_leaves` calls do not emit these events — inspect their return value instead. See [Listening to events](./events.md) for subscription instructions.

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
