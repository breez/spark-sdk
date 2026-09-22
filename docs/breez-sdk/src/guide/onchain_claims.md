# Claiming on-chain deposits

On-chain deposits go through three stages. A deposit is detected once it reaches the mempool, while it is still unconfirmed. From then on it is visible in the SDK, and each deposit carries an {{#name is_mature}} field. After **3 on-chain confirmations** on mainnet the deposit is mature ({{#name is_mature}} is true) and the SDK [automatically attempts](#setting-a-max-fee-for-automatic-claims) to claim it. Regtest matures a deposit at 1 confirmation.

The SDK can also claim [before maturity](#claiming-before-maturity) when the cost of claiming early fits the configured [maximum deposit claim fee](config.md#max-deposit-claim-fee), so a deposit can be credited sooner. If that fee is too low for either kind of claim, the deposit is not claimed automatically and should be [manually claimed](#manually-claiming-deposits).

## Detecting a deposit before it confirms

The Spark operators report a deposit UTXO once it has a confirmation. To detect one sooner the SDK also asks its chain service about the deposit addresses it has handed out, which is what lets a deposit be claimed, or shown to the user, while it is still in the mempool. A deposit found this way arrives through {{#enum SdkEvent::NewDeposits}} and appears in {{#name list_unclaimed_deposits}} with {{#name is_mature}} false, whether or not the SDK goes on to claim it automatically.

Each watched address costs one chain-service request per sync. Requesting a receive address starts a 24-hour window on it and requesting it again restarts that, so a wallet that is not expecting an on-chain payment settles at no requests at all. An address that has taken a deposit keeps being watched past its window until that deposit confirms.

## Setting a max fee for automatic claims

The [maximum deposit claim fee](config.md#max-deposit-claim-fee) in the SDK configuration is the most the SDK will pay when it claims a deposit automatically. This page calls it the ceiling. The default is 1 sat/vbyte, which is low, so deposits need manual claiming until it is raised. The forms the setting takes (absolute sats, sats/vbyte, or the fastest recommended fee with a leeway) are described on the [configuration page](config.md#max-deposit-claim-fee).

This ceiling is not only an on-chain fee tolerance. It also caps what the provider may take to credit a deposit [before it matures](#claiming-before-maturity), so the value you choose decides both how much on-chain fee the SDK will pay and whether deposits are claimed early at all.

To increase the likelihood of automatically claiming deposits, you may set the maximum fee to the fastest recommended rate at the time of claim, which can result in higher fees.

{{#tabs refunding_payments:set-max-fee-to-recommended-fees}}

However, even when setting a high fee, the SDK might still fail to automatically claim deposits. In these cases, it's recommended to manually claim them by letting the end user accept the required fees. When [manual intervention](#manually-claiming-deposits) is required, the SDK emits an {{#enum SdkEvent::UnclaimedDeposits}} event containing information about the deposit. See [Listening to events](events.md) for how to subscribe to events.

## Claiming before maturity

A deposit does not have to wait for maturity. The Spark Service Provider will front the credited amount earlier, taking a spread for carrying the risk. The SDK claims this way automatically whenever the spread fits within the [maximum deposit claim fee](config.md#max-deposit-claim-fee). The default of 1 sat/vbyte works out to about 99 sats, below any spread the provider charges, so deposits are claimed at maturity until the ceiling is raised enough to cover one. The same applies to {{#name claim_deposit}}, which claims a not-yet-mature deposit early when its own {{#name max_fee}} allows.

The spread is largely the on-chain cost of the provider's claim plus a percentage of the deposit, so it grows with the deposit.

## Setting a max fee for one deposit

The ceiling passed to {{#name claim_deposit}} is recorded on that deposit and carried into later automatic attempts on it, so it is how one deposit is treated differently from the rest without changing the configuration for all of them.

Raising it above the provider's spread lets that single deposit be claimed early, and the SDK goes on applying it on later sync passes, so the app does not have to keep calling {{#name claim_deposit}} until the claim lands. Lowering it below the spread keeps that one deposit from being claimed early: it waits for maturity while the others keep claiming early under the configured ceiling.

An automatic claim at maturity runs under whichever ceiling is larger, the deposit's or the configured one. A raised ceiling therefore applies at maturity too, if the early claim never happens, so raise it to what you are willing to pay for the deposit, not only for the early claim. A lowered one restricts only the early claim, and holding a deposit back never leaves it stuck: at maturity there is no spread to avoid, only an ordinary on-chain fee.

Calling {{#name claim_deposit}} without a {{#name max_fee}} claims under the configured [maximum deposit claim fee](config.md#max-deposit-claim-fee), and clears any ceiling standing on the deposit.

The ceiling is recorded before the claim is attempted, so it stands whatever the attempt does. What the attempt itself reports is covered under [Claim outcomes](#claim-outcomes).

The recorded ceiling is readable as {{#name max_claim_fee}} on each deposit from {{#name list_unclaimed_deposits}}, unset while the configured ceiling applies.

Deposits are not part of the synced wallet records, so a ceiling set on one device stays on that device. The same deposit seen from another device runs under whatever that device has configured.

## Claim outcomes

{{#name claim_deposit}} reports what it did as {{#name outcome}}, which is worth handling in full:

- {{#enum ClaimDepositOutcome::Settled}} carries the payment it produced.
- {{#enum ClaimDepositOutcome::Submitted}} means a claim made before maturity is settling asynchronously. Watch for the payment via {{#name list_payments}} or the [payment events](events.md).
- {{#enum ClaimDepositOutcome::Deferred}} means nothing was claimed yet and no further call is needed.

Which outcome occurs follows from the deposit's maturity and the fee ceiling rather than from anything you ask for. A {{#name max_fee}} below what an early claim costs returns {{#enum ClaimDepositOutcome::Deferred}} rather than failing. A deposit that has already matured and whose claim exceeds the ceiling is a different matter and returns {{#enum SdkError::MaxDepositClaimFeeExceeded}}, because nothing will claim it until the ceiling rises or on-chain fees fall.

Whether a deferred deposit actually waits for maturity depends on its {{#name reason}}. The SDK re-attempts an early claim as the deposit gains confirmations, so a claim declined at a depth the provider will not yet front is often claimed early a block or two later.

- {{#enum ClaimDeferredReason::NoEarlyClaimAvailable}} usually clears with the next confirmation.
- {{#enum ClaimDeferredReason::MaxFeeExceeded}} does not clear on its own. The deposit waits for maturity unless the ceiling is raised.
- {{#enum ClaimDeferredReason::ProviderDeclined}} means the provider refused or could not be reached, which another confirmation does not address.

Only the first is a wait you can put a time on, so showing a user "claimed in about 30 minutes" for the others would be wrong.

## Manually claiming deposits

When a deposit cannot be claimed automatically because the configured maximum deposit claim fee is too low, claim it manually with a higher {{#name max_fee}}. The recommended approach is to show the user the required fee and ask for approval before claiming.

Claiming a deposit a claim already has returns {{#enum SdkError::DepositClaimInProgress}}. That covers a claim still running, from a background attempt or another call, and one that has already credited the deposit and is waiting for the provider to spend the output. Neither is a failure to show the user, and neither needs anything from you: check {{#name instant_claim_status}} to tell them apart.

{{#tabs refunding_payments:handle-fee-exceeded}}

### Showing the choice to the user

{{#name fetch_claim_deposit_quote}} prices both ways of claiming a deposit, so an app can offer the choice rather than deciding for the user. It returns the deposit's current {{#name confirmations}} alongside two quotes, one for claiming early and one for claiming at maturity. Each quote carries the fee and the {{#name confirmations_required}}. That is the depth the deposit becomes claimable at, not a count of blocks still to wait, so subtract the deposit's current confirmations to get the wait: an early claim claimable at 1 confirmation, on a deposit with 0, is available a block from now.

The early quote is absent when the provider will not front this particular deposit. It is also absent when claiming early would not actually be earlier: once a deposit has matured, or when the provider would only credit at maturity's own depth, waiting is both cheaper and no slower, so there is no choice left to offer. Whether a deposit is fronted at all, and at what depth, varies with the deposit rather than being a setting you control. An absent early quote therefore means no early claim is offered for this deposit, not that early claiming is unavailable.

The early quote is priced whether or not the configured [maximum deposit claim fee](config.md#max-deposit-claim-fee) would allow it, so the fee it shows is the provider's price rather than what the configured ceiling permits. Acting on the early quote yourself means passing a {{#name max_fee}} to {{#name claim_deposit}} of at least the quoted {{#name fee_sats}}. With a lower one the call returns {{#enum ClaimDepositOutcome::Deferred}} with {{#enum ClaimDeferredReason::MaxFeeExceeded}}, carrying what the early claim would have cost, and the deposit waits for maturity unless the ceiling is raised (see [Claim outcomes](#claim-outcomes)).

The quote for maturity is always present, but may be flagged {{#name is_estimate}} when the provider will not quote a deposit this early, in which case the fee is derived from current on-chain fees and the final one may differ.

What to offer follows from the quote and the configured ceiling. The middle column is the one to check first: where the SDK claims by itself, a dialog defaulting to maturity shows the user one outcome and delivers another.

| Quote state | What the SDK will do | What to present |
|---|---|---|
| No early quote | Claim at maturity | The maturity option only |
| Early quote, depth not yet reached | Claim early once the depth arrives, if the fee fits the ceiling | The maturity option, with early shown as available in N blocks |
| Early quote at a reachable depth, fee above the ceiling | Wait for maturity | Both, early requiring an explicit higher max fee |
| Early quote at a reachable depth, fee within the ceiling | Claim early by itself | Default to early, or offer no choice |

{{#tabs refunding_payments:fetch-claim-deposit-quote}}

## Listing unclaimed deposits

Retrieve the deposits the SDK is tracking. This includes pending deposits that do not yet have sufficient confirmations, deposits with sufficient confirmations that failed to claim (with the specific failure reason), and deposits already claimed whose output the provider has not yet spent. Pending deposits will be automatically claimed once they have sufficient confirmations, or sooner if the configured ceiling covers an early claim.

A deposit claimed before maturity carries {{#enum InstantClaimStatus::Submitted}} in its {{#name instant_claim_status}} while the claim settles, and {{#enum InstantClaimStatus::Claimed}} once the amount is credited. It stays in the list until the provider spends the deposit output, some time after the credit. Treat {{#enum InstantClaimStatus::Claimed}} as settled and branch on it rather than showing the deposit as awaiting action. When the SDK claims automatically it emits {{#enum SdkEvent::ClaimedDeposits}} at submission, so a deposit can appear both in that event and in this list.

A deposit claimed elsewhere, by another instance sharing the wallet or on another device, reaches {{#enum InstantClaimStatus::Claimed}} the next time the SDK tries to claim it and the provider reports it as already claimed. No {{#enum SdkEvent::ClaimedDeposits}} event is emitted, because the claim was not made here. The credit still arrives as a payment, so follow it through {{#name list_payments}} or the [payment events](events.md).

{{#tabs refunding_payments:list-unclaimed-deposits}}

## Refunding deposits

When a deposit cannot be successfully claimed you can refund it to an external Bitcoin address. This creates a transaction that sends the amount (minus transaction fees) to the specified destination address.

A deposit that has already been claimed is not a candidate: its {{#name instant_claim_status}} is {{#enum InstantClaimStatus::Claimed}}, so check that before offering a refund.

The [recommended fees](#recommended-fees) API is useful for determining appropriate fee levels for refund transactions.

A deposit can only be refunded once it has enough confirmations. Calling {{#name refund_deposit}} earlier fails, reporting the deposit as unknown while it is unconfirmed and as having too few confirmations for a block or so after that. Nothing is signed or stored when this happens, so retry after a few more blocks.

{{#tabs refunding_payments:refund-deposit}}

<div class="warning">
<h4>Developer note</h4>
The total fee must cover at least 1 sat/vB of the refund transaction so it can be relayed by the Bitcoin network. The exact minimum depends on the size of the transaction, which varies with the destination address type: around 99 sats to a native segwit address and 111 sats to a taproot one. If the fee is lower, the refund request is rejected and the error states the required minimum.
</div>

### Tracking a refund

{{#name refund_state}} on {{#name DepositInfo}} reports how far the refund has got:

- **{{#enum RefundState::BroadcastPending}}**: the refund is signed and stored but has not been seen on the network. The SDK rebroadcasts it on every sync until the deposit is spent, so a refund that failed to send because of a temporary network problem recovers on its own.
- **{{#enum RefundState::Broadcast}}**: the network has accepted the refund and it is waiting to confirm. The deposit disappears from {{#name list_unclaimed_deposits}} once it does.

A refund created near the 1 sat/vB minimum can stay at {{#enum RefundState::BroadcastPending}} indefinitely if the network's minimum relay fee later rises above what it pays. Rebroadcasting cannot fix this, because the network keeps refusing the same transaction. Read {{#name last_error}} for the reason the network gave, then call {{#name refund_deposit}} again at a higher fee to replace it.

Replacing a refund that is already on the network costs more than the original fee, because the replacement also pays to relay its own size. When the fee offered is too low, the call is rejected and the error states the minimum required.

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
