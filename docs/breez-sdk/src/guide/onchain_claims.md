# Claiming on-chain deposits

On-chain deposits go through three stages: once detected, the deposit is visible in the SDK and each deposit includes a {{#name is_mature}} field; after **3 on-chain confirmations** the deposit has sufficient confirmations ({{#name is_mature}} is true) and the SDK [automatically attempts](#setting-a-max-fee-for-automatic-claims) to claim it. If the maximum deposit claim fee is too low, the deposit won't be automatically claimed and should be [manually claimed](#manually-claiming-deposits).

## Setting a max fee for automatic claims

The [maximum deposit claim fee](config.md#max-deposit-claim-fee) setting in the SDK configuration defines the maximum fee the SDK uses when automatically claiming an on-chain deposit. The SDK's default fee limit is set to 1 sats/vbyte, which is low and requires manual claiming when fees exceed this threshold. You can set a higher fee, either in sats/vbyte, in absolute sats, or to the fastest recommended fee at the time of claim, with a leeway in sats/vbyte.

To increase the likelihood of automatically claiming deposits, you may set the maximum fee to the fastest recommended rate at the time of claim, which can result in higher fees.

{{#tabs refunding_payments:set-max-fee-to-recommended-fees}}

However, even when setting a high fee, the SDK might still fail to automatically claim deposits. In these cases, it's recommended to manually claim them by letting the end user accept the required fees. When [manual intervention](#manually-claiming-deposits) is required, the SDK emits an {{#enum SdkEvent::UnclaimedDeposits}} event containing information about the deposit. See [Listening to events](events.md) for how to subscribe to events.

## Manually claiming deposits

When a deposit cannot be automatically claimed due to the configured maximum fee being too low, you can manually claim it by specifying a higher fee limit. The recommended approach is to display a user interface showing the required fee amount and request user approval before proceeding with manual claiming.

{{#tabs refunding_payments:handle-fee-exceeded}}

### Claiming before maturity

A deposit does not have to wait for maturity. The Spark Service Provider will front the credited amount earlier and take a spread for carrying the risk, and the SDK claims this way automatically whenever the spread fits within the [maximum deposit claim fee](config.md#max-deposit-claim-fee). There is no separate setting to switch on, but there is a threshold to clear: the same ceiling governs both, and the default of 1 sat/vbyte works out to about 99 sats, below any spread the provider charges. Deposits are therefore claimed at maturity until the ceiling is raised enough to cover a spread, which is what opts into claiming early. The same applies to {{#name claim_deposit}}, which claims a not-yet-mature deposit early when its own {{#name max_fee}} allows.

The spread is largely the on-chain cost of the provider's claim plus a percentage of the deposit, so it grows with the deposit. A single ceiling therefore covers small deposits and leaves larger ones to wait for maturity, and the amount that separates the two moves with on-chain fees.

A claim made before maturity settles asynchronously, so {{#name claim_deposit}} returns no payment; watch for it via {{#name list_payments}} or the [payment events](events.md). The deposit also stays in {{#name list_unclaimed_deposits}} with its {{#name instant_claim_status}} set to {{#enum InstantClaimStatus::Submitted}} for a short time after submission (a {{#enum SdkEvent::ClaimedDeposits}} event has already fired), and is removed once the claim settles.

### Showing the choice to the user

{{#name fetch_claim_deposit_quote}} prices both ways of claiming a deposit, so an app can offer the choice rather than deciding for the user. It returns the deposit's current {{#name confirmations}} alongside a quote for claiming early and one for claiming at maturity, each carrying the fee and the {{#name confirmations_required}}, which is the depth it becomes claimable at rather than a count of blocks still to wait. Subtract the deposit's current confirmations for that: an early claim claimable at 1 confirmation, on a deposit with 0, is available a block from now.

The early quote is absent when the provider will not front this particular deposit, and when claiming early would not actually be earlier: once a deposit has matured, or when the provider would only credit at maturity's own depth, waiting is both cheaper and no slower, so there is no choice left to offer. The early quote is priced whether or not the configured [maximum deposit claim fee](config.md#max-deposit-claim-fee) would allow it, since that ceiling is usually far below a spread and the point of the quote is to let the user decide. Acting on it therefore means passing a {{#name max_fee}} to {{#name claim_deposit}} of at least the quoted {{#name fee_sats}}; with a lower one the claim declines and waits for maturity instead. The quote for maturity is always present, but may be flagged {{#name is_estimate}} when the provider will not quote a deposit this early, in which case the fee is derived from current on-chain fees and the final one may differ.

{{#tabs refunding_payments:fetch-claim-deposit-quote}}

## Listing unclaimed deposits

Retrieve all deposits that have not yet been claimed. This includes pending deposits that do not yet have sufficient confirmations, as well as deposits with sufficient confirmations that failed to claim (with the specific failure reason). Pending deposits will be automatically claimed once they have sufficient confirmations.

{{#tabs refunding_payments:list-unclaimed-deposits}}

## Refunding deposits

When a deposit cannot be successfully claimed you can refund it to an external Bitcoin address. This creates a transaction that sends the amount (minus transaction fees) to the specified destination address.

The [recommended fees](#recommended-fees) API is useful for determining appropriate fee levels for refund transactions.

{{#tabs refunding_payments:refund-deposit}}

<div class="warning">
<h4>Developer note</h4>
The total fee must cover at least 1 sat/vB of the refund transaction so it can be relayed by the Bitcoin network. The exact minimum depends on the size of the transaction, which varies with the destination address type (around 111 sats for a taproot address). If the fee is lower, the refund request is rejected and the error states the required minimum.
</div>

## Implementing a custom claim logic

For advanced use cases, you may want to implement a custom claim logic instead of relying on the SDK's automatic process. This gives you complete control over when and how deposits are claimed.

To disable automatic claims, unset the [maximum deposit claim fee](config.md#max-deposit-claim-fee). Then use the methods described above to manually claim deposits based on your business logic.

Common scenarios for custom claiming logic include:

- **Dynamic fee adjustment**: Adjust claiming fees based on market conditions or priority
- **Conditional claiming**: Only claim deposits that meet certain criteria (amount thresholds, time windows, etc.)
- **Integration with external systems**: Coordinate claims with other business processes

The [recommended fees](#recommended-fees) API is useful for determining appropriate fee levels for claiming deposits. For example, you can implement a custom claim logic to only claim deposits if the required fee rate is less than the fastest recommended fee (or any other).

{{#tabs refunding_payments:custom-claim-logic}}

## Recommended fees

Get Bitcoin fee estimates for different confirmation targets to help determine appropriate fee levels for claiming or refunding deposits.

{{#tabs refunding_payments:recommended-fees}}
