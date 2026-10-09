# Earning fees on USDC/USDT payments

You can add your own fee to the [USDC/USDT payments](./cross_chain.md) your users make through the SDK. You set the fee in the [Partner portal](https://partners.breez.technology/), and the SDK applies it with no code or configuration change.

- The fee applies to [sends](./send_payment.md#usdc-usdt), [receives](./receive_payment.md#usdc-usdt), and [Cash App payment links](./cash_app_to_usdc_usdt.md) made with your API key and Breez SDK - Spark 0.27 or later.
- It is charged as a percentage of each payment after the provider's own fee. It is included in the `service_fee_amount` the SDK returns for each payment, so users see it in the fee your app already displays.
- Flashnet, which processes cross-chain payments, keeps 20% of your fee, and you receive the remaining 80%. See [Flashnet's fee documentation](https://docs.flashnet.xyz/orchestra/fees) for how fees are calculated.
- Until you set a fee, payments work as before.

## Setting your fee

In the partner portal, open **Settings**. The fee is set in the **USDC/USDT Fee Settings** card.

| Field | Value |
| ----- | ----- |
| Fee Percentage | 0.01% to 10%, in steps of 0.01%. 0 turns the fee off. |
| Payout Destination | USDC on Solana, or USDB on Spark. |
| Payout Address | A Solana address, or a mainnet Spark address starting with `spark1`. |

Use a payout address you control on that chain. The address is checked when you save, and an invalid address is rejected.

### When changes take effect

- **A new fee**: the next time the SDK starts, or within an hour for an app that is already running.
- **A new rate**: from the next payment.
- **A new payout address**: for fees earned after the change. Earlier fees are still paid to the address that was set when they were earned.

You're emailed whenever your USDC/USDT fee settings change.

### API keys

The fee applies to payments made with the API key shown in your partner portal **Settings**. Each partner portal account, registered with its own email, holds one API key. If your apps use several API keys, set the fee in the account for each key. Fees earned with each key are tracked and claimed in that key's account.

## Claiming your fees

Fees accrue as a balance that you claim, rather than being paid out per payment. Claim them on the **Fees** page, in the **Balances** card of the **USDC/USDT Fees** section.

- Each balance is claimed in full, and a claim needs at least $1 (some payout routes need more). If a balance can't be claimed yet, the portal shows why.
- A fee becomes claimable once the provider has confirmed it. **Pending payout** shows claims you've made that haven't been paid out yet.
- A claim is paid to the payout destination that was set when the fees were earned. Conversion and payout costs can make the amount delivered slightly lower.

## Turning the fee off

Set the fee to 0. Claim your available balance first: the portal won't turn the fee off while any balance is claimable. Anything left (amounts under the $1 minimum, or fees not yet confirmed) can't be claimed while the fee is off, and turning the fee back on makes it claimable again.

Turning the fee off takes effect from the next payment.

## Tracking your earnings

The **Fees** page shows, for the selected period, your fees earned (after Flashnet's share), payment volume, and payment count. It also lists your balances, a monthly earnings chart, your claim history, and recent payments with their route and your fee.
