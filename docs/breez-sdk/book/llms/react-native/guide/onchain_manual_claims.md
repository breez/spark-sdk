# Claiming deposits manually

A deposit the SDK does not claim on its own, usually because the claim costs more than the [max claim fee](onchain_claims.md#the-max-claim-fee), can be claimed with `claimDeposit`. The recommended flow is to show the user the fee, ask for approval, then claim with a max fee that covers it.

## Pricing a claim

`fetchClaimDepositQuote` prices both ways of claiming a deposit: `instant` for an instant or expedited claim, and `mature` for the standard claim. Each quote carries its fee and `confirmationsRequired`, and the response carries the deposit's current `confirmations`. The early quote is requested from the provider on every call, so call it while the user is deciding, not on a timer.

```typescript
const quote = await sdk.fetchClaimDepositQuote({
  txid: deposit.txid,
  vout: deposit.vout
})

// The standard claim, and how many blocks away it is.
const blocksToWait = Math.max(0, quote.mature.confirmationsRequired - quote.confirmations)
console.log(`Wait ${blocksToWait} blocks and pay ${quote.mature.feeSats} sats`)

// An instant or expedited claim, when the provider offers one.
const instant = quote.instant
if (instant != null) {
  const instantBlocks = Math.max(0, instant.confirmationsRequired - quote.confirmations)
  console.log(`Or wait ${instantBlocks} blocks and pay ${instant.feeSats} sats`)
}
```



- `confirmationsRequired` is the depth the claim becomes available at, not the number of blocks left to wait. On the `instant` quote, 0 means an instant claim and 1 or 2 an expedited one.
- The `instant` quote is absent when no early claim is offered right now: the provider will not front this deposit, the deposit has already reached the standard claim depth, the early claim would credit no sooner, or the provider could not be reached. Only the last one is worth retrying.
- Each quote's `maxFeeSats` is the most that claim may cost for this deposit, under the max claim fee that applies to it. A `feeSats` within it means the SDK makes that claim on its own. It is unset when automatic claiming is off.
- The `instant` quote shows the provider's price, whether or not it is within its `maxFeeSats`.
- The `mature` quote is always present. It is flagged `isEstimate` when the provider will not quote the deposit yet, and its fee is then estimated from current on-chain fees.

What to offer depends on what the SDK would do anyway. Check the middle column first: where the SDK claims by itself, a dialog defaulting to the standard claim shows the user one outcome and delivers another.

| Quote | What the SDK will do | What to offer |
|---|---|---|
| No `instant` quote | Standard claim | The standard claim only |
| `instant` quote, depth not reached yet | Claim early once the depth arrives, if `feeSats` is within `maxFeeSats` | The standard claim, with the early claim shown as available in N blocks |
| `instant` quote at a reachable depth, `feeSats` above `maxFeeSats` | Wait for the standard claim | Both, the early claim requiring a higher max fee |
| `instant` quote at a reachable depth, `feeSats` within `maxFeeSats` | Claim early by itself | The early claim, or no choice at all |

## Claiming

Call `claimDeposit` with a `maxFee` that covers the fee the user approved. To claim early, it has to cover the `instant` quote's `feeSats`. For a deposit whose automatic claim failed, the required fee is in its `claimError`:

```typescript
if (deposit.claimError?.tag === DepositClaimError_Tags.MaxDepositClaimFeeExceeded) {
  const requiredFee = deposit.claimError.inner.requiredFeeSats

  // Show UI to user with the required fee and get approval
  const userApproved = true // Replace with actual user approval logic

  if (userApproved) {
    const claimRequest: ClaimDepositRequest = {
      txid: deposit.txid,
      vout: deposit.vout,
      maxFee: new MaxFee.Fixed({ amount: requiredFee })
    }
    await sdk.claimDeposit(claimRequest)
  }
}
```



Claiming a deposit that already has a claim returns `SdkError.DepositClaimInProgress`: either a claim is still running, or one has credited the deposit and is waiting for the provider to spend the output. Neither needs any action, and `instantClaimStatus` tells them apart.

## Handling the outcome

`claimDeposit` reports what it did in `outcome`:

| Outcome | Meaning |
|---|---|
| `ClaimDepositOutcome.Settled` | A standard claim settled. Carries the resulting payment. |
| `ClaimDepositOutcome.Submitted` | An instant or expedited claim is settling. The payment arrives through `listPayments` and the [payment events](events.md). |
| `ClaimDepositOutcome.Deferred` | Nothing was claimed yet. Its `reason` says what happens next. |

A deferred claim's `reason` is one of:

- `ClaimDeferredReason.NoEarlyClaimAvailable`: the provider will not front the deposit at its current depth. This usually clears with the next confirmation, and the SDK retries on its own.
- `ClaimDeferredReason.MaxFeeExceeded`: the early claim costs more than the max fee. The deposit waits for the standard claim unless the max fee is raised.
- `ClaimDeferredReason.ProviderDeclined`: the provider refused or could not be reached. Another confirmation does not change this.

Only the first is a wait you can put a time on, so don't show "claimed in about 30 minutes" for the others.

The call can also fail with:

- `SdkError.MaxDepositClaimFeeExceeded`: the deposit has reached the standard claim depth and its claim costs more than the max fee. Nothing claims it until the max fee rises or on-chain fees fall.
- `SdkError.DepositTooSmall`: what would be left after the claim fee is below the dust limit. No max fee helps, but the deposit may become claimable once on-chain fees fall. `fetchClaimDepositQuote` returns it too, and automatic claims record it in `claimError`.

## Giving one deposit its own max fee

The `maxFee` passed to `claimDeposit` is recorded on the deposit, and the SDK's later automatic attempts on it use it too. This lets one deposit be treated differently without changing the configuration:

- Above the instant claim fee, it lets that deposit be claimed early. The SDK keeps retrying, so there is no need to call `claimDeposit` again.
- Below the instant claim fee, it holds that deposit back from an early claim while the others still claim early.

The standard claim runs under the larger of the deposit's own max fee and the configured one. A raised max fee therefore covers the standard claim too, while a lowered one only restricts the early claim.

- Calling `claimDeposit` without a `maxFee` claims under the configured max claim fee and clears the deposit's own.
- The max fee is recorded before the claim is attempted, so it stands whatever the outcome.
- It is readable as `maxClaimFee` on the deposit, unset while the configured max claim fee applies.
- It is kept on this device only. Another device sharing the wallet applies its own configuration to the same deposit.

## Writing your own claim logic

For rules the [max claim fee](config.md#max-deposit-claim-fee) cannot express, such as claiming only above an amount, at certain times, or under a fee limit capped in sats, claim with `claimDeposit` yourself. Keep automatic claiming on and handle only the deposits it leaves unclaimed, or unset the max claim fee to decide every claim.

A deposit whose claim exceeded its max fee carries the total the claim would cost, `requiredFeeSats`, in its `claimError`. This example accepts up to 1% of the deposit, but never more than 5,000 sats:

```typescript
if (deposit.claimError?.tag === DepositClaimError_Tags.MaxDepositClaimFeeExceeded) {
  const requiredFee = deposit.claimError.inner.requiredFeeSats

  // Accept up to 1% of the deposit amount, but never more than 5,000 sats
  const onePercent = deposit.amountSats / BigInt(100)
  const maxFeeSats = onePercent < BigInt(5000) ? onePercent : BigInt(5000)
  if (requiredFee <= maxFeeSats) {
    const claimRequest: ClaimDepositRequest = {
      txid: deposit.txid,
      vout: deposit.vout,
      maxFee: new MaxFee.Fixed({ amount: requiredFee })
    }
    await sdk.claimDeposit(claimRequest)
  }
}
```
