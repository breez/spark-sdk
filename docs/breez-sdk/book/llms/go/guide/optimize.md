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

By default, the SDK automatically triggers optimization after each payment (sent or received). For applications requiring more control, you can disable automatic optimization in the [configuration](./config.md#optimization-configuration) and drive it manually using `OptimizeLeaves`.

### Run optimization to completion

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.optimize_leaves

Call `OptimizeLeaves` with an `OptimizeLeavesRequest` using the default `OptimizationModeFull` mode to run optimization until no further work is productive. The call blocks for the duration of the run and returns an `OptimizeLeavesResponse` whose `Outcome` is `OptimizationOutcomeCompleted` with the number of rounds executed. A `RoundsExecuted` of `0` means the wallet was already optimal at call time.

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

To display progress or cancel between rounds, pass an `OptimizeLeavesRequest` with `OptimizationModeSingleRound`. Each call executes one round and the response `Outcome` is `OptimizationOutcomeInProgress` (more work remains) or `OptimizationOutcomeCompleted` (terminal — either the planner confirmed this swap finished optimization, or a `RoundsExecuted` of `0` indicates the wallet was already optimal). Cancel between rounds simply by stopping the loop.

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

If `OptimizeLeaves` is invoked while another optimization run (auto or manual) is already in flight, it returns `SdkErrorOptimizationAlreadyRunning`. The SDK may also preempt a manual run to free leaves for a higher-priority payment, in which case the call returns `SdkErrorOptimizationCancelled`.

## Auto-optimization events

When automatic optimization is enabled, the SDK emits `SdkEventAutoOptimization` events so your application can track the background optimizer's progress. Manual `OptimizeLeaves` calls do not emit these events — inspect their return value instead. See [Listening to events](./events.md) for subscription instructions.

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
