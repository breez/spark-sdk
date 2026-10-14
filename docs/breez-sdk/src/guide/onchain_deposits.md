# Tracking deposits

## Listing deposits

{{#name list_unclaimed_deposits}} returns every deposit the SDK is tracking, from the moment it reaches the mempool until its output is spent by a claim or a refund.

{{#tabs refunding_payments:list-unclaimed-deposits}}

## Deposit states

What to show for a deposit follows from a few of its fields. The first row that matches applies.

| State | How to tell | What to show |
|---|---|---|
| Claimed | {{#name instant_claim_status}} is {{#enum InstantClaimStatus::Claimed}} | Settled. No action needed. |
| Being claimed | {{#name instant_claim_status}} is {{#enum InstantClaimStatus::Submitted}} | Being credited |
| Refunding | {{#name refund_state}} is set | The refund's progress. See [Tracking a refund](onchain_refunds.md#tracking-a-refund). |
| Claim failed | {{#name claim_error}} is set | The reason, with the option to [claim manually](onchain_manual_claims.md) or [refund](onchain_refunds.md) |
| Pending | None of the above | Pending. The SDK claims it automatically. |

A claimed deposit stays in the list until the provider spends its output, some time after the credit. When the SDK claims a deposit on its own it emits {{#enum SdkEvent::ClaimedDeposits}} as the claim is submitted, so a deposit can appear in that event and in the list at the same time.

A deposit claimed by another instance sharing the wallet, or on another device, becomes {{#enum InstantClaimStatus::Claimed}} the next time the SDK tries to claim it. No {{#enum SdkEvent::ClaimedDeposits}} event is emitted for it, since the claim was not made here, but the credit still arrives as a payment.

## Seeing deposits before they confirm

A deposit arrives through {{#enum SdkEvent::NewDeposits}} and appears in {{#name list_unclaimed_deposits}} as soon as it reaches the mempool, with {{#name is_mature}} false until it reaches the standard claim depth.

<div class="warning">
<h4>Developer note</h4>
The Spark operators only report confirmed deposits, so the SDK finds unconfirmed ones by asking its chain service about recently used deposit addresses, one request per address per sync. Requesting a receive address watches it for 24 hours, and requesting it again restarts that window. An address that received a deposit stays watched until the deposit confirms.
</div>
