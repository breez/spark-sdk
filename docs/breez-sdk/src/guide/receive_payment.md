<h1 id="receiving-payments">
    <a class="header" href="#receiving-payments">Receiving payments</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment">API docs</a>
</h1>

Once the SDK is initialized, you can directly begin receiving payments. The SDK currently supports three methods of receiving: Lightning, Bitcoin and Spark.

## Lightning

#### BOLT11 invoice

When receiving via Lightning, we can generate a BOLT11 invoice to be paid. Setting the invoice amount fixes the amount the sender should pay.

**Note:** the payment may fallback to a direct Spark payment (if the payer's client supports this).

{{#tabs receive_payment:receive-payment-lightning-bolt11}}

#### LNURL-Pay & Lightning address

To receive via LNURL-Pay and/or a Lightning address, follow [these instructions](/guide/receive_lnurl_pay.md).

> Note: Lightning payments work in Spark even if the receiver is offline. To understand how it works under the hood, read [this](https://docs.spark.money/learn/lightning).

## Bitcoin

For on-chain payments you can use the static Bitcoin address to receive payments.

> **Note:** Spark currently requires **3 on-chain confirmations** for Bitcoin transactions before they can be claimed.

The SDK monitors the specified address for new UTXOs and automatically initiates the claim process when funds are detected. If the Config's maximum deposit claim fee is not set or below the current Spark fee to claim the Bitcoin deposit, the deposit will need to be claimed or refunded manually. See [Claiming on-chain deposits](/guide/onchain_claims.md) for more details on this process.

{{#tabs receive_payment:receive-payment-onchain}}

## Spark

For payments between Spark users, you can use a Spark address or generate a Spark invoice to receive payments.

#### Spark address

Spark addresses are static.

{{#tabs receive_payment:receive-payment-spark-address}}

#### Spark invoice

Spark invoices are single-use and may impose restrictions on the payment, such as amount, expiry, and who is able to pay it.

{{#tabs receive_payment:receive-payment-spark-invoice}}

## Event Flows

Once a receive payment is initiated, you can follow and react to the different payment events using the guide below for each payment method. See [Listening to events](/guide/events.md) for how to subscribe to events.

| Event      | Description                                    | UX Suggestion                                                                                                                         |
| ---------- | ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| **Synced** | The SDK has synced payments in the background. | Update the payments list and balance. See [listing payments](/guide/list_payments.md) and [fetching the balance](/guide/get_info.md). |

#### Lightning

| Event                | Description                                                       | UX Suggestion                                    |
| -------------------- | ----------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer was detected and the claim process will start. | Show payment as pending.                         |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete.        | Update the balance and show payment as complete. |

#### Bitcoin

| Event                 | Description                                                                                                                                                                                               | UX Suggestion                                                                                                    |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| **UnclaimedDeposits** | The SDK attempted to claim static address deposits but they failed from one of several reasons. Either the claim fee exceeded the maximum allowed limit or there was an issue finding the available UTXO. | Allow the user to refund these failed deposits. See [Handling unclaimed deposits](/guide/unclaimed_deposits.md). |
| **ClaimedDeposits**   | The SDK successfully claimed static address deposits.                                                                                                                                                     |                                                                                                                  |
| **PaymentPending**    | The Spark transfer was detected and the claim process will start.                                                                                                                                         | Show payment as pending.                                                                                         |
| **PaymentSucceeded**  | The Spark transfer is claimed and the payment is complete.                                                                                                                                                | Update the balance and show payment as complete.                                                                 |

#### Spark

| Event                | Description                                                                                                                                                                                          | UX Suggestion                                    |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer was detected and the claim process will start. For Spark HTLC payments, the claim will only start once the HTLC is claimed. For more details see [Spark HTLC payments](htlcs.md). | Show payment as pending.                         |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete.                                                                                                                                           | Update the balance and show payment as complete. |
