# Sending payments

Once the SDK is initialized, you can directly begin sending payments. The send process takes two steps:

1. [Preparing the Payment](send_payment.md#preparing-payments)
1. [Sending the Payment](send_payment.md#sending-payments)

For sending payments via LNURL, see [LNURL-Pay](lnurl_pay.md).

<h2 id="preparing-payments">
    <a class="header" href="#preparing-payments">Preparing Payments</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_payment">API docs</a>
</h2>

During the prepare step, the SDK ensures that the inputs are valid with respect to the payment request type,
and also returns the fees related to the payment so they can be confirmed.

The payment request field supports Lightning invoices, Bitcoin addresses, Spark addresses and Spark invoices.

### Lightning

#### BOLT11 invoice

For BOLT11 invoices the amount can be optionally set. The amount set in the request is only taken into account if it's an amountless invoice.

If the invoice also contains a Spark address, the payment can be sent directly via a Spark transfer instead. When this is the case, the prepare response includes the Spark transfer fee.

{{#tabs send_payment:prepare-send-payment-lightning-bolt11}}

### Bitcoin

For Bitcoin addresses, the amount must be set in the request. The prepare response includes fee quotes for three payment speeds: Slow, Medium, and Fast.

{{#tabs send_payment:prepare-send-payment-onchain}}

### Spark address

For Spark addresses, the amount must be set in the request. Sending to a Spark address uses a direct Spark transfer.

{{#tabs send_payment:prepare-send-payment-spark-address}}

### Spark invoice

For Spark invoices, the amount can be optionally set. It is only required if the invoice doesn't specify an amount. If the invoice specifies an amount, providing a different amount is not supported.

<div class="warning">
<h4>Developer note</h4>
Spark invoices may require a token (non-Bitcoin) as the payment asset. To determine the requirements of a Spark invoice and any restrictions it may impose, see the <a href="./parse.md">Parsing inputs</a> page. To learn more about tokens, see the <a href="./tokens.md">Handling tokens</a> page.
</div>

{{#tabs send_payment:prepare-send-payment-spark-invoice}}

<h2 id="sending-payments">
    <a class="header" href="#sending-payments">Sending Payments</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.send_payment">API docs</a>
</h2>

Once the payment has been prepared, pass the prepare response as an argument to the send method and set any selected payment options.

### Lightning

In the optional send payment options for BOLT11 invoices, you can set:
- **Prefer Spark** - Set the preference to use Spark to transfer the payment if the invoice contains a Spark address. By default, using Spark transfers are disabled.
- **Completion Timeout** - By default, this function returns immediately. You can override this behavior by specifying a completion timeout in seconds. If the timeout is reached, a pending payment object is returned. If the payment completes within the timeout, the completed payment object is returned.

{{#tabs send_payment:send-payment-lightning-bolt11}}

### Bitcoin

In the optional send payment options for Bitcoin addresses, you can set:
- **Confirmation Speed** - The priority that the Bitcoin transaction confirms, that also effects the fee paid. By default, it is set to Fast.
- **Idempotency Key** - An optional UUID that identifies the payment. If set, providing the same idempotency key for multiple requests will ensure that only one payment is made.

{{#tabs send_payment:send-payment-onchain}}

### Spark

In the optional send payment options for Spark address and invoices, you can set:
- **Idempotency Key** - A UUID that identifies the payment. Providing the same idempotency key for multiple requests will ensure that only one payment is made. This applies only to non-token Spark transfers.

{{#tabs send_payment:send-payment-spark}}

## Event Flows

Once a send payment is initiated, you can follow and react to the different payment events using the guide below for each payment method. See [Listening to events](/guide/events.html) for how to subscribe to events.

| Event      | Description                                    | UX Suggestion                                                                                                                         |
| ---------- | ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| **Synced** | The SDK has synced payments in the background. | Update the payments list and balance. See [listing payments](/guide/list_payments.md) and [fetching the balance](/guide/get_info.md). |

### Lightning

| Event                | Description                                                                       | UX Suggestion                                    |
| -------------------- | --------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer has been started. Awaiting Lightning payment completion.       | Show payment as pending.                         |
| **PaymentSucceeded** | The Lightning invoice has been paid either over Lightning or via a Spark transfer | Update the balance and show payment as complete. |
| **PaymentFailed**    | The attempt to pay the Lightning invoice failed.                                  |                                                  |

### Bitcoin

| Event                | Description                                                                  | UX Suggestion                                    |
| -------------------- | ---------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer has been started. Awaiting onchain withdrawal completion. | Show payment as pending.                         |
| **PaymentSucceeded** | The payment amount was successfully withdrawn onchain.                       | Update the balance and show payment as complete. |

### Spark

| Event                | Description                     | UX Suggestion                                    |
| -------------------- | ------------------------------- | ------------------------------------------------ |
| **PaymentSucceeded** | The Spark transfer is complete. | Update the balance and show payment as complete. |
