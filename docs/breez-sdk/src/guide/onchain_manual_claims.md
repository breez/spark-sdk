# Claiming deposits manually

A deposit the SDK does not claim on its own, usually because the claim costs more than the [max claim fee](onchain_claims.md#the-max-claim-fee), can be claimed with {{#name claim_deposit}}. The recommended flow is to show the user the fee, ask for approval, then claim with a max fee that covers it.

## Pricing a claim

{{#name fetch_claim_deposit_quote}} prices both ways of claiming a deposit: {{#name instant}} for an instant or expedited claim, and {{#name mature}} for the standard claim. Each quote carries its fee and {{#name confirmations_required}}, and the response carries the deposit's current {{#name confirmations}}. The early quote is requested from the provider on every call, so call it while the user is deciding, not on a timer.

{{#tabs refunding_payments:fetch-claim-deposit-quote}}

- {{#name confirmations_required}} is the depth the claim becomes available at, not the number of blocks left to wait. On the {{#name instant}} quote, 0 means an instant claim and 1 or 2 an expedited one.
- The {{#name instant}} quote is absent when no early claim is offered right now: the provider will not front this deposit, the deposit has already reached the standard claim depth, the early claim would credit no sooner, or the provider could not be reached. Only the last one is worth retrying.
- The {{#name instant}} quote shows the provider's price, whether or not the max claim fee allows it.
- The {{#name mature}} quote is always present. It is flagged {{#name is_estimate}} when the provider will not quote the deposit yet, and its fee is then estimated from current on-chain fees.

What to offer depends on what the SDK would do anyway. Check the middle column first: where the SDK claims by itself, a dialog defaulting to the standard claim shows the user one outcome and delivers another.

| Quote | What the SDK will do | What to offer |
|---|---|---|
| No {{#name instant}} quote | Standard claim | The standard claim only |
| {{#name instant}} quote, depth not reached yet | Claim early once the depth arrives, if the fee fits the max claim fee | The standard claim, with the early claim shown as available in N blocks |
| {{#name instant}} quote at a reachable depth, fee above the max claim fee | Wait for the standard claim | Both, the early claim requiring a higher max fee |
| {{#name instant}} quote at a reachable depth, fee within the max claim fee | Claim early by itself | The early claim, or no choice at all |

## Claiming

Call {{#name claim_deposit}} with a {{#name max_fee}} that covers the fee the user approved. To claim early, it has to cover the {{#name instant}} quote's {{#name fee_sats}}. For a deposit whose automatic claim failed, the required fee is in its {{#name claim_error}}:

{{#tabs refunding_payments:handle-fee-exceeded}}

Claiming a deposit that already has a claim returns {{#enum SdkError::DepositClaimInProgress}}: either a claim is still running, or one has credited the deposit and is waiting for the provider to spend the output. Neither needs any action, and {{#name instant_claim_status}} tells them apart.

## Handling the outcome

{{#name claim_deposit}} reports what it did in {{#name outcome}}:

| Outcome | Meaning |
|---|---|
| {{#enum ClaimDepositOutcome::Settled}} | A standard claim settled. Carries the resulting payment. |
| {{#enum ClaimDepositOutcome::Submitted}} | An instant or expedited claim is settling. The payment arrives through {{#name list_payments}} and the [payment events](events.md). |
| {{#enum ClaimDepositOutcome::Deferred}} | Nothing was claimed yet. Its {{#name reason}} says what happens next. |

A deferred claim's {{#name reason}} is one of:

- {{#enum ClaimDeferredReason::NoEarlyClaimAvailable}}: the provider will not front the deposit at its current depth. This usually clears with the next confirmation, and the SDK retries on its own.
- {{#enum ClaimDeferredReason::MaxFeeExceeded}}: the early claim costs more than the max fee. The deposit waits for the standard claim unless the max fee is raised.
- {{#enum ClaimDeferredReason::ProviderDeclined}}: the provider refused or could not be reached. Another confirmation does not change this.

Only the first is a wait you can put a time on, so don't show "claimed in about 30 minutes" for the others.

The call can also fail with:

- {{#enum SdkError::MaxDepositClaimFeeExceeded}}: the deposit has reached the standard claim depth and its claim costs more than the max fee. Nothing claims it until the max fee rises or on-chain fees fall.
- {{#enum SdkError::DepositTooSmall}}: what would be left after the claim fee is below the dust limit. No max fee helps, but the deposit may become claimable once on-chain fees fall. {{#name fetch_claim_deposit_quote}} returns it too, and automatic claims record it in {{#name claim_error}}.

## Giving one deposit its own max fee

The {{#name max_fee}} passed to {{#name claim_deposit}} is recorded on the deposit, and the SDK's later automatic attempts on it use it too. This lets one deposit be treated differently without changing the configuration:

- Above the instant claim fee, it lets that deposit be claimed early. The SDK keeps retrying, so there is no need to call {{#name claim_deposit}} again.
- Below the instant claim fee, it holds that deposit back from an early claim while the others still claim early.

The standard claim runs under the larger of the deposit's own max fee and the configured one. A raised max fee therefore covers the standard claim too, while a lowered one only restricts the early claim.

- Calling {{#name claim_deposit}} without a {{#name max_fee}} claims under the configured max claim fee and clears the deposit's own.
- The max fee is recorded before the claim is attempted, so it stands whatever the outcome.
- It is readable as {{#name max_claim_fee}} on the deposit, unset while the configured max claim fee applies.
- It is kept on this device only. Another device sharing the wallet applies its own configuration to the same deposit.

## Writing your own claim logic

To decide every claim yourself, unset the [max claim fee](config.md#max-deposit-claim-fee) to turn off automatic claiming, then claim with {{#name claim_deposit}} by your own rules, such as claiming only above an amount, at certain times, or within a fee budget.

A deposit whose claim exceeded its max fee carries the total the claim would cost, {{#name required_fee_sats}}, in its {{#name claim_error}}. This example claims only when that fee is at most 1% of the deposit:

{{#tabs refunding_payments:custom-claim-logic}}
