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

<div class="warning">
<h4>Developer note</h4>

Keep multiplicity as low as possible while meeting your performance requirements. A high multiplicity can make unilateral exits prohibitively expensive.

</div>

## Controlling optimization timing

The optimization process reorganizes leaves by swapping them to achieve optimal denominations. During this process, funds in leaves being swapped become temporarily unavailable for payments, which can delay transaction processing.

By default, the SDK automatically triggers optimization after each payment (sent or received). For applications requiring more control, you can disable automatic optimization in the [configuration](./config.md#optimization-configuration) and drive it manually using {{#name optimize_leaves}}.

<h3 id="optimize-leaves-full">
    <a class="header" href="#optimize-leaves-full">Run optimization to completion</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.optimize_leaves">API docs</a>
</h3>

Call {{#name optimize_leaves}} with an {{#name OptimizeLeavesRequest}} using the default {{#enum OptimizationMode::Full}} mode to run optimization until no further work is productive. The call blocks for the duration of the run and returns an {{#name OptimizeLeavesResponse}} whose {{#name outcome}} is {{#enum OptimizationOutcome::Completed}} with the number of rounds executed. A `rounds_executed` of `0` means the wallet was already optimal at call time.

{{#tabs optimize:optimize-leaves-full}}

<h3 id="optimize-leaves-single-round">
    <a class="header" href="#optimize-leaves-single-round">Run optimization one round at a time</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.optimize_leaves">API docs</a>
</h3>

To display progress or cancel between rounds, pass an {{#name OptimizeLeavesRequest}} with {{#enum OptimizationMode::SingleRound}}. Each call executes one round and the response {{#name outcome}} is {{#enum OptimizationOutcome::InProgress}} (more work remains) or {{#enum OptimizationOutcome::Completed}} (terminal — either the planner confirmed this swap finished optimization, or `rounds_executed == 0` indicating the wallet was already optimal). Cancel between rounds simply by stopping the loop.

{{#tabs optimize:optimize-leaves-single-round}}

<div class="warning">
<h4>Developer note</h4>

If {{#name optimize_leaves}} is invoked while another optimization run (auto or manual) is already in flight, it returns {{#enum SdkError::OptimizationAlreadyRunning}}. The SDK may also preempt a manual run to free leaves for a higher-priority payment, in which case the call returns {{#enum SdkError::OptimizationCancelled}}.

</div>

## Auto-optimization events

When automatic optimization is enabled, the SDK emits {{#enum SdkEvent::AutoOptimization}} events so your application can track the background optimizer's progress. Manual {{#name optimize_leaves}} calls do not emit these events — inspect their return value instead. See [Listening to events](./events.md) for subscription instructions.

{{#tabs optimize:auto-optimization-events}}
