# Claiming deposits manually

A deposit the SDK does not claim on its own, usually because the claim costs more than the [max claim fee](onchain_claims.md#the-max-claim-fee), can be claimed with `claim_deposit`. The recommended flow is to show the user the fee, ask for approval, then claim with a max fee that covers it.

## Pricing a claim

`fetch_claim_deposit_quote` prices both ways of claiming a deposit: `instant` for an instant or expedited claim, and `mature` for the standard claim. Each quote carries its fee and `confirmations_required`, and the response carries the deposit's current `confirmations`. The early quote is requested from the provider on every call, so call it while the user is deciding, not on a timer.

```rust
let quote = sdk
    .fetch_claim_deposit_quote(FetchClaimDepositQuoteRequest {
        txid: deposit.txid.clone(),
        vout: deposit.vout,
    })
    .await?;

// The standard claim, and how many blocks away it is.
let blocks_to_wait = quote
    .mature
    .confirmations_required
    .saturating_sub(quote.confirmations);
info!(
    "Wait {} blocks and pay {} sats",
    blocks_to_wait, quote.mature.fee_sats
);

// An instant or expedited claim, when the provider offers one.
if let Some(instant) = &quote.instant {
    let blocks_to_wait = instant
        .confirmations_required
        .saturating_sub(quote.confirmations);
    info!(
        "Or wait {} blocks and pay {} sats",
        blocks_to_wait, instant.fee_sats
    );
}
```



- `confirmations_required` is the depth the claim becomes available at, not the number of blocks left to wait. On the `instant` quote, 0 means an instant claim and 1 or 2 an expedited one.
- The `instant` quote is absent when no early claim is offered right now: the provider will not front this deposit, the deposit has already reached the standard claim depth, the early claim would credit no sooner, or the provider could not be reached. Only the last one is worth retrying.
- Each quote's `max_fee_sats` is the most that claim may cost for this deposit, under the max claim fee that applies to it. A `fee_sats` within it means the SDK makes that claim on its own. It is unset when automatic claiming is off.
- The `instant` quote shows the provider's price, whether or not it is within its `max_fee_sats`.
- The `mature` quote is always present. It is flagged `is_estimate` when the provider will not quote the deposit yet, and its fee is then estimated from current on-chain fees.

What to offer depends on what the SDK would do anyway. Check the middle column first: where the SDK claims by itself, a dialog defaulting to the standard claim shows the user one outcome and delivers another.

| Quote | What the SDK will do | What to offer |
|---|---|---|
| No `instant` quote | Standard claim | The standard claim only |
| `instant` quote, depth not reached yet | Claim early once the depth arrives, if `fee_sats` is within `max_fee_sats` | The standard claim, with the early claim shown as available in N blocks |
| `instant` quote at a reachable depth, `fee_sats` above `max_fee_sats` | Wait for the standard claim | Both, the early claim requiring a higher max fee |
| `instant` quote at a reachable depth, `fee_sats` within `max_fee_sats` | Claim early by itself | The early claim, or no choice at all |

## Claiming

Call `claim_deposit` with a `max_fee` that covers the fee the user approved. To claim early, it has to cover the `instant` quote's `fee_sats`. For a deposit whose automatic claim failed, the required fee is in its `claim_error`:

```rust
if let Some(DepositClaimError::MaxDepositClaimFeeExceeded {
    required_fee_sats, ..
}) = &deposit.claim_error
{
    // Show UI to user with the required fee and get approval
    let user_approved = true; // Replace with actual user approval logic

    if user_approved {
        let request = ClaimDepositRequest {
            txid: deposit.txid.clone(),
            vout: deposit.vout,
            max_fee: Some(MaxFee {
                onchain: MaxOnchainFee::Fixed {
                    amount: *required_fee_sats,
                },
                proportional_ppm: None,
            }),
        };
        sdk.claim_deposit(request).await?;
    }
}
```



Claiming a deposit that already has a claim returns `SdkError::DepositClaimInProgress`: either a claim is still running, or one has credited the deposit and is waiting for the provider to spend the output. Neither needs any action, and `instant_claim_status` tells them apart.

## Handling the outcome

`claim_deposit` reports what it did in `outcome`:

| Outcome | Meaning |
|---|---|
| `ClaimDepositOutcome::Settled` | A standard claim settled. Carries the resulting payment. |
| `ClaimDepositOutcome::Submitted` | An instant or expedited claim is settling. The payment arrives through `list_payments` and the [payment events](events.md). |
| `ClaimDepositOutcome::Deferred` | Nothing was claimed yet. Its `reason` says what happens next. |

A deferred claim's `reason` is one of:

- `ClaimDeferredReason::NoEarlyClaimAvailable`: the provider will not front the deposit at its current depth. This usually clears with the next confirmation, and the SDK retries on its own.
- `ClaimDeferredReason::MaxFeeExceeded`: the early claim costs more than the max fee. The deposit waits for the standard claim unless the max fee is raised.
- `ClaimDeferredReason::ProviderDeclined`: the provider refused or could not be reached. Another confirmation does not change this.

Only the first is a wait you can put a time on, so don't show "claimed in about 30 minutes" for the others.

The call can also fail with:

- `SdkError::MaxDepositClaimFeeExceeded`: the deposit has reached the standard claim depth and its claim costs more than the max fee. Nothing claims it until the max fee rises or on-chain fees fall.
- `SdkError::DepositTooSmall`: what would be left after the claim fee is below the dust limit. No max fee helps, but the deposit may become claimable once on-chain fees fall. `fetch_claim_deposit_quote` returns it too, and automatic claims record it in `claim_error`.

## Giving one deposit its own max fee

The `max_fee` passed to `claim_deposit` is recorded on the deposit, and the SDK's later automatic attempts on it use it too. This lets one deposit be treated differently without changing the configuration:

- Above the instant claim fee, it lets that deposit be claimed early. The SDK keeps retrying, so there is no need to call `claim_deposit` again.
- Below the instant claim fee, it holds that deposit back from an early claim while the others still claim early.

The standard claim runs under the larger of the deposit's own max fee and the configured one. A raised max fee therefore covers the standard claim too, while a lowered one only restricts the early claim.

- Calling `claim_deposit` without a `max_fee` claims under the configured max claim fee and clears the deposit's own.
- The max fee is recorded before the claim is attempted, so it stands whatever the outcome.
- It is readable as `max_claim_fee` on the deposit, unset while the configured max claim fee applies.
- It is kept on this device only. Another device sharing the wallet applies its own configuration to the same deposit.

## Writing your own claim logic

For rules the [max claim fee](config.md#max-deposit-claim-fee) cannot express, such as claiming only above an amount, at certain times, or under a fee limit capped in sats, claim with `claim_deposit` yourself. Keep automatic claiming on and handle only the deposits it leaves unclaimed, or unset the max claim fee to decide every claim.

A deposit whose claim exceeded its max fee carries the total the claim would cost, `required_fee_sats`, in its `claim_error`. This example accepts up to 1% of the deposit, but never more than 5,000 sats:

```rust
if let Some(DepositClaimError::MaxDepositClaimFeeExceeded {
    required_fee_sats, ..
}) = &deposit.claim_error
{
    // Accept up to 1% of the deposit amount, but never more than 5,000 sats
    let max_fee_sats = (deposit.amount_sats / 100).min(5_000);
    if *required_fee_sats <= max_fee_sats {
        let request = ClaimDepositRequest {
            txid: deposit.txid.clone(),
            vout: deposit.vout,
            max_fee: Some(MaxFee {
                onchain: MaxOnchainFee::Fixed {
                    amount: *required_fee_sats,
                },
                proportional_ppm: None,
            }),
        };
        sdk.claim_deposit(request).await?;
    }
}
```
