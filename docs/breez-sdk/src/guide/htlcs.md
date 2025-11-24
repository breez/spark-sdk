# Spark HTLC payments

Hash Time-Locked Contract (HTLC) payments are conditional payments that enable atomic cross-chain swaps. The SDK supports Spark HTLCs through a simple API.

This feature is available when paying to **Spark addresses only** (Spark invoices are not supported).

In an HTLC payment, the sender locks funds using a cryptographic hash of a secret preimage and sets an expiration time. The receiver can claim the payment by revealing the preimage before expiration. If the receiver fails to claim the payment in time, the funds are automatically returned to the sender.

## Sending HTLC Payments

HTLC payments use the standard payment API described in the [Sending payments](send_payment.md) guide. To create an HTLC payment, prepare the payment normally, then provide the Spark HTLC options when [sending](send_payment.md#spark). These options include the payment hash (SHA-256 hash of the preimage) and the expiry duration.

{{#tabs htlcs:send-htlc-payment}}

<div class="warning">
<h4>Developer note</h4>
Preimages are required to be unique and are not managed by the SDK. It is your responsibility as a developer to manage them, including how to generate them, store them, and eventually share them with the receiver.
</div>

<h2 id="listing-claimable-htlc-payments">
    <a class="header" href="#listing-claimable-htlc-payments">Listing claimable HTLC payments</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments">API docs</a>
</h2>

Claimable HTLC payments will be included in the [list of payments](/guide/list_payments.md) as pending as soon as they are detected. To list only claimable HTLC payments, you can filter by HTLC status.

{{#tabs htlcs:list-claimable-htlc-payments}}

<h2 id="claiming-htlc-payments">
    <a class="header" href="#claiming-htlc-payments">Claiming HTLC Payments</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.claim_htlc_payment">API docs</a>
</h2>

To claim an HTLC payment, provide the preimage that matches the payment hash.

{{#tabs htlcs:claim-htlc-payment}}
