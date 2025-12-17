# Custom leaf optimization

The SDK implements a configurable Spark leaf optimization process. It supports two optimization policies:

- **Maximize unilateral exit efficiency**: aims to minimize the number of leaves, reducing costs for unilaterally exiting Bitcoin funds.
- **Increase payment speed**: maintains multiple copies of each leaf denomination to reduce the need for swaps during Bitcoin payments.

## Configuring the optimization policy

The optimization behavior is controlled by the **multiplicity** setting, an integer value in the range 0-5. Setting it to 0 fully optimizes for unilateral exit efficiency, while values greater than 0 also optimize for payment speed. Higher values prioritize payment speed more aggressively, resulting in higher unilateral exit costs but faster payments, especially for bursts of transactions.

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

The optimization process runs as a background task that reorganizes leaves by swapping them to achieve optimal denominations. During this process, funds in leaves being swapped become temporarily unavailable for payments, which can delay transaction processing.

By default, the SDK automatically triggers optimization after each payment (sent or received). For applications requiring more control, you can disable automatic optimization in the [configuration](./config.md#optimization-configuration) and manage it manually as described below.

<h3 id="start-optimization">
    <a class="header" href="#start-optimization">Start optimization</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.startLeafOptimization">API docs</a>
</h3>

You can manually trigger the optimization task to start running in the background. If optimization is already running, no new task will be started.

{{#tabs optimize:start-optimization}}

<h3 id="cancel-optimization">
    <a class="header" href="#cancel-optimization">Cancel optimization</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.cancelLeafOptimization">API docs</a>
</h3>

You can cancel an ongoing optimization task and wait for it to stop completely. Optimization is done in rounds, and the current round will complete before stopping.

{{#tabs optimize:cancel-optimization}}

<div class="warning">
<h4>Developer note</h4>

The SDK automatically cancels optimization when it would block an immediate payment. Use manual cancellation only when anticipating upcoming payment activity, not when there is an immediate need to make a payment.

</div>

<h3 id="get-optimization-progress">
    <a class="header" href="#get-optimization-progress">Get optimization progress</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.getLeafOptimizationProgress">API docs</a>
</h3>

You can retrieve the current optimization progress to monitor the optimization task.

{{#tabs optimize:get-optimization-progress}}

## Optimization events

The SDK emits events to keep your application informed about optimization status. See [Listening to events](./events.md) for subscription instructions.

{{#tabs optimize:optimization-events}}
