# Sending tokens to multiple recipients

A single token transaction can pay multiple recipients at once. The batch API is separate from the regular send flow because a batch has no single amount to report: it may span several tokens, so prepare reports a total per token instead.

<h2 id="preparing-the-batch">
    <a class="header" href="#preparing-the-batch">Preparing the batch</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_token_batch">API docs</a>
</h2>

Each recipient is identified by a destination, which is either a Spark address or a Spark invoice, and the two may be mixed freely in one batch:

- **Spark address**: the token identifier and the amount must be set, exactly as for a single-recipient send.
- **Spark invoice**: the token and amount are taken from the invoice. The amount is only required if the invoice doesn't specify one. If the invoice specifies an amount, providing a different amount is not supported.

The same invoice may only appear once in a batch. Repeating a plain Spark address is allowed: that is simply two outputs to the same payee.

The response resolves every recipient into the concrete token and amount it will be paid, and reports {{#name totals}}: what the batch debits, one entry per distinct token. You may show these to the user before sending.

{{#tabs tokens:send-token-batch}}

<h2 id="sending-the-batch">
    <a class="header" href="#sending-the-batch">Sending the batch</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.send_token_batch">API docs</a>
</h2>

Sending returns one payment per recipient, in the order the recipients were requested. Every payment carries the same transaction hash, so your application can group them later by filtering the payment list on that hash. See [Listing payments](./list_payments.md).

Each payment also raises its own {{#enum SdkEvent::PaymentSucceeded}} event, the same as a single-recipient token send.

<div class="warning">
<h4>Developer note</h4>
A token send carries no idempotency key. If a batch fails in a way that leaves the outcome unknown, look for the transaction first (filtering the payment list by transaction hash) instead of sending it again, which would pay every recipient twice.
</div>

<h2 id="external-signing">
    <a class="header" href="#external-signing">External signing</a>
</h2>

A batch can also be signed outside the SDK. The flow matches the single-recipient one described in [Client signing](./client_signing.md), with two differences: the unsigned package is built from the batch prepare response, and publishing it returns every payment rather than one. The package carries the same per-token totals, so the signer can show the user what they are approving.
