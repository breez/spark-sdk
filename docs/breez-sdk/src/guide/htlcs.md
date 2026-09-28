# Conditional Payments

Conditional payments use Hash Time-Locked Contracts (HTLCs) to lock funds with a cryptographic hash of a secret preimage and an expiration time. The payment can only be claimed by revealing the preimage before expiration. If not claimed in time, the funds are automatically returned to the sender. This enables use cases like atomic cross-chain swaps.

The SDK supports both sending conditional payments via Spark HTLCs and receiving them via HODL invoices.

<div class="warning">
<h4>Developer note</h4>
Preimages are required to be unique and are not managed by the SDK. It is your responsibility as a developer to manage them, including how to generate them, store them, and provide them when claiming payments.
</div>

## Sending Spark HTLC payments

HTLC payments use the standard payment API described in [Sending payments](send_payment.md). To create an HTLC payment, prepare the payment normally, then provide the Spark HTLC options when [sending](send_payment.md#spark). These options include the payment hash (SHA-256 hash of the preimage) and the expiry duration.

{{#tabs htlcs:send-htlc-payment}}

## Receiving using HODL invoices

You can receive using HODL invoices — Lightning invoices where the payment is held until you claim it by revealing the preimage. To create one, provide a {{#name payment_hash}} when calling {{#name receive_payment}} with the {{#enum ReceivePaymentMethod::Bolt11Invoice}} payment method.

{{#tabs htlcs:receive-hodl-invoice-payment}}

## Listing claimable conditional payments

Once detected, claimable HTLC payments are immediately listed as pending in the [list of payments](/guide/list_payments.md). Additionally, a {{#enum SdkEvent::PaymentPending}} event is emitted to notify your application. See [Listening to events](/guide/events.md) for more details.

To list only claimable HTLC payments, you can filter by HTLC status. This works for both Spark HTLC payments and HODL invoices.

{{#tabs htlcs:list-claimable-htlc-payments}}

## Claiming conditional payments

To claim an HTLC payment, provide the preimage that matches the payment hash. This works for both Spark HTLC payments and HODL invoices.

{{#tabs htlcs:claim-htlc-payment}}
