# Claiming on-chain deposits

Bitcoin sent to the wallet's on-chain address arrives as a deposit, which has to be claimed before it reaches the balance. The SDK detects a deposit as soon as it is in the mempool and claims it on its own once the cost fits the [max claim fee](#the-max-claim-fee).

## How deposits are claimed

A deposit can be claimed at three speeds. Every claim is made through the Spark Service Provider, whose fee covers the on-chain cost of the claim and may include a share of the deposit amount. For the two faster ones the provider also fronts the funds, and charges more for doing so.

| Claim | When funds arrive | Cost |
|---|---|---|
| Instant | At 0 confirmations. Offered for some deposits only. | Instant claim fee |
| Expedited | At 1 or 2 confirmations | Instant claim fee |
| Standard | At 3 confirmations on mainnet (1 on regtest) | Standard claim fee, the lowest |

The SDK takes the fastest claim whose fee fits the max claim fee. Whether a deposit is offered an instant or expedited claim, and at which depth, is decided by the provider for each deposit. The SDK retries the early claim as confirmations arrive, so a deposit the provider will not front in the mempool is often claimed a block or two later.

## The max claim fee

The [max claim fee](config.md#max-deposit-claim-fee) is the most the SDK pays to claim a deposit on its own. It caps both the standard claim fee and the instant claim fee, so it also decides whether deposits are claimed early at all.

It is set as an absolute amount in sats, a rate in sats/vbyte, or the fastest recommended fee plus a leeway. The rate and the recommended fee can also allow a share of the deposit amount, {{#name proportional_ppm}}, because the provider's fees grow with the deposit. Without it, a large deposit can exceed the max claim fee even when on-chain fees are low.

The default, 1 sat/vbyte plus 0.1% of the deposit amount, is sized for the standard claim, so deposits are usually credited at 3 confirmations. Raise it to have deposits credited early. To make automatic claims more likely, follow the fastest recommended fee:

{{#tabs refunding_payments:set-max-fee-to-recommended-fees}}

## What your app needs to handle

When the SDK cannot claim a deposit, usually because the claim costs more than the max claim fee, it emits {{#enum SdkEvent::UnclaimedDeposits}}. The deposit then waits for your app to claim it manually, typically once the user accepts the fee, or to refund it to an external address.

- **[Tracking deposits](onchain_deposits.md)**: list deposits, see them before they confirm, and tell their states apart.
- **[Claiming deposits manually](onchain_manual_claims.md)**: price a claim, claim it with a higher max fee, and handle the outcome.
- **[Refunding deposits](onchain_refunds.md)**: send a deposit back to a Bitcoin address and follow the refund.
