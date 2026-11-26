# Earning fees on Lightning payments

You can add your own fee to the Lightning payments your users send and receive through the SDK. You set the fee in the [Partner portal](https://partners.breez.technology/), and the SDK applies it with no code or configuration change.

- The fee applies to Lightning payments made with your API key (see [Which payments carry it](#which-payments-carry-it)).
- It is charged as a percentage of each payment.
- You receive a share of each fee, paid directly to your Spark wallet as the payment settles. Breez and Lightspark, which processes Lightning payments, keep the rest. The partner portal shows the split, with an example payment.

## Which payments carry it

- **Lightning sends**: [BOLT11 invoices](./send_payment.md#bolt11-invoice), and [LNURL-Pay and Lightning addresses](./lnurl_pay.md), including payments funded by a [token conversion](./stable_balance.md).
- **Lightning receives**: [BOLT11 invoices](./receive_payment.md#bolt11-invoice). [Lightning address](./receive_lnurl_pay.md) receives can carry it too: [contact Breez](mailto:contact@breez.technology) to enable it.

Payments that move over Spark rather than Lightning carry no fee: a send to an invoice's Spark address, and a receive on an invoice that carries your Spark address (with {{#name prefer_spark_over_lightning}} enabled), since a payer could pay that address directly.

With [Turnkey](./turnkey.md), or an [external signer](./external_signer.md) whose {{#name supports_split_transfers}} returns false, Lightning sends carry no fee, since the fee is paid in the same transfer as the payment. Receives still carry it.

## Setting your fee

In the partner portal, open **Settings**. The fee is set in the **Lightning Fee Settings** card.

| Field | Value |
| ----- | ----- |
| Fee Percentage | 0.01% to 10%, in steps of 0.01%. 0 turns the fee off. |
| Payout Spark Address | A mainnet Spark address starting with `spark1`, asked for when you first set a fee. |

The payout address can't be changed once saved. Use a Spark wallet you control and keep its keys safe: to receive fees in a different wallet, use a [new API key](./getting_started.md#api-key). A wallet already receiving fees for another API key is refused.

### API keys

The fee applies to payments made with the API key shown in your partner portal **Settings**. Each partner portal account, registered with its own email, holds one API key. If your apps use several API keys, set the fee in the account for each key, with its own payout wallet.

## Receiving your fees

There is nothing to claim. Your share of each fee is paid to your payout wallet as the payment settles, and shows up in that wallet's balance like any other payment it receives.

## Turning the fee off

Set the fee to 0. Your payout address stays registered for your API key, and is used again if you set a fee later.

## Tracking your earnings

The **Fees** page has a **Lightning Fees** section. For the selected period it shows your fees earned (your share), payment volume, and payment count, along with your current fee, a monthly earnings chart split into sends and receives, and recent payments with their amount, fee and your share. The figures are refreshed periodically, so a recent payment can take a few minutes to appear.

## Showing the fee to your users

Once you set a fee, Lightning receives cost your users a fee as well as sends. Show both before the user confirms:

- **Sends**: the fee is part of the Lightning fee in the prepare response, {{#name lightning_fee_sats}} for an [invoice](./send_payment.md#bolt11-invoice) and {{#name fee_sats}} for [LNURL-Pay](./lnurl_pay.md).
- **Receives**: the receive response's {{#name fee}} reports it (see [BOLT11 invoice](./receive_payment.md#bolt11-invoice)). By default the payer covers it, and with {{#name fee_policy}} set to {{#enum FeePolicy::FeesIncluded}} it comes out of what your user receives. For Lightning address receives, the payment notification's `fee_sat` reports it (see [payment notifications](./lnurl_webhooks.md)).

## When a fee quote expires

Each fee is priced in a quote that lasts a few minutes. If it expires before the payment is sent, the SDK quotes it again on the same terms. If the new quote prices the payment differently, the send fails with [FeesChanged](./errors.md#fees-changed): prepare it again and show the new fee. With [client signing](./client_signing.md), the user has to approve the payment before its quote expires, or publishing fails with [QuoteExpired](./errors.md#quote-expired).
