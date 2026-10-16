# Claiming deposits manually

A deposit the SDK does not claim on its own, usually because the claim costs more than the [max claim fee](onchain_claims.md#the-max-claim-fee), can be claimed with `ClaimDeposit`. The recommended flow is to show the user the fee, ask for approval, then claim with a max fee that covers it.

## Pricing a claim

`FetchClaimDepositQuote` prices both ways of claiming a deposit: `Instant` for an instant or expedited claim, and `Mature` for the standard claim. Each quote carries its fee and `ConfirmationsRequired`, and the response carries the deposit's current `Confirmations`. The early quote is requested from the provider on every call, so call it while the user is deciding, not on a timer.

```go
quote, err := sdk.FetchClaimDepositQuote(breez_sdk_spark.FetchClaimDepositQuoteRequest{
	Txid: deposit.Txid,
	Vout: deposit.Vout,
})
if err != nil {
	return err
}

// The standard claim, and how many blocks away it is.
blocksToWait := uint32(0)
if quote.Mature.ConfirmationsRequired > quote.Confirmations {
	blocksToWait = quote.Mature.ConfirmationsRequired - quote.Confirmations
}
log.Printf("Wait %v blocks and pay %v sats", blocksToWait, quote.Mature.FeeSats)

// An instant or expedited claim, when the provider offers one.
if quote.Instant != nil {
	instantBlocks := uint32(0)
	if quote.Instant.ConfirmationsRequired > quote.Confirmations {
		instantBlocks = quote.Instant.ConfirmationsRequired - quote.Confirmations
	}
	log.Printf("Or wait %v blocks and pay %v sats", instantBlocks, quote.Instant.FeeSats)
}
```



- `ConfirmationsRequired` is the depth the claim becomes available at, not the number of blocks left to wait. On the `Instant` quote, 0 means an instant claim and 1 or 2 an expedited one.
- The `Instant` quote is absent when no early claim is offered right now: the provider will not front this deposit, the deposit has already reached the standard claim depth, the early claim would credit no sooner, or the provider could not be reached. Only the last one is worth retrying.
- Each quote's `MaxFeeSats` is the most that claim may cost for this deposit, under the max claim fee that applies to it. A `FeeSats` within it means the SDK makes that claim on its own. It is unset when automatic claiming is off.
- The `Instant` quote shows the provider's price, whether or not it is within its `MaxFeeSats`.
- The `Mature` quote is always present. It is flagged `IsEstimate` when the provider will not quote the deposit yet, and its fee is then estimated from current on-chain fees.

What to offer depends on what the SDK would do anyway. Check the middle column first: where the SDK claims by itself, a dialog defaulting to the standard claim shows the user one outcome and delivers another.

| Quote | What the SDK will do | What to offer |
|---|---|---|
| No `Instant` quote | Standard claim | The standard claim only |
| `Instant` quote, depth not reached yet | Claim early once the depth arrives, if `FeeSats` is within `MaxFeeSats` | The standard claim, with the early claim shown as available in N blocks |
| `Instant` quote at a reachable depth, `FeeSats` above `MaxFeeSats` | Wait for the standard claim | Both, the early claim requiring a higher max fee |
| `Instant` quote at a reachable depth, `FeeSats` within `MaxFeeSats` | Claim early by itself | The early claim, or no choice at all |

## Claiming

Call `ClaimDeposit` with a `MaxFee` that covers the fee the user approved. To claim early, it has to cover the `Instant` quote's `FeeSats`. For a deposit whose automatic claim failed, the required fee is in its `ClaimError`:

```go
if claimErr := *deposit.ClaimError; claimErr != nil {
	if exceeded, ok := claimErr.(breez_sdk_spark.DepositClaimErrorMaxDepositClaimFeeExceeded); ok {
		requiredFee := exceeded.RequiredFeeSats

		// Show UI to user with the required fee and get approval
		userApproved := true // Replace with actual user approval logic

		if userApproved {
			maxFee := breez_sdk_spark.MaxFee(breez_sdk_spark.MaxFeeFixed{Amount: requiredFee})
			claimRequest := breez_sdk_spark.ClaimDepositRequest{
				Txid:   deposit.Txid,
				Vout:   deposit.Vout,
				MaxFee: &maxFee,
			}
			_, err := sdk.ClaimDeposit(claimRequest)
			if err != nil {
				var sdkErr *breez_sdk_spark.SdkError
				if errors.As(err, &sdkErr) {
					// Handle SdkError - can inspect specific variants if needed
					// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
				}
				return err
			}
		}
	}
}
```



Claiming a deposit that already has a claim returns `SdkErrorDepositClaimInProgress`: either a claim is still running, or one has credited the deposit and is waiting for the provider to spend the output. Neither needs any action, and `InstantClaimStatus` tells them apart.

## Handling the outcome

`ClaimDeposit` reports what it did in `Outcome`:

| Outcome | Meaning |
|---|---|
| `ClaimDepositOutcomeSettled` | A standard claim settled. Carries the resulting payment. |
| `ClaimDepositOutcomeSubmitted` | An instant or expedited claim is settling. The payment arrives through `ListPayments` and the [payment events](events.md). |
| `ClaimDepositOutcomeDeferred` | Nothing was claimed yet. Its `Reason` says what happens next. |

A deferred claim's `Reason` is one of:

- `ClaimDeferredReasonNoEarlyClaimAvailable`: the provider will not front the deposit at its current depth. This usually clears with the next confirmation, and the SDK retries on its own.
- `ClaimDeferredReasonMaxFeeExceeded`: the early claim costs more than the max fee. The deposit waits for the standard claim unless the max fee is raised.
- `ClaimDeferredReasonProviderDeclined`: the provider refused or could not be reached. Another confirmation does not change this.

Only the first is a wait you can put a time on, so don't show "claimed in about 30 minutes" for the others.

The call can also fail with:

- `SdkErrorMaxDepositClaimFeeExceeded`: the deposit has reached the standard claim depth and its claim costs more than the max fee. Nothing claims it until the max fee rises or on-chain fees fall.
- `SdkErrorDepositTooSmall`: what would be left after the claim fee is below the dust limit. No max fee helps, but the deposit may become claimable once on-chain fees fall. `FetchClaimDepositQuote` returns it too, and automatic claims record it in `ClaimError`.

## Giving one deposit its own max fee

The `MaxFee` passed to `ClaimDeposit` is recorded on the deposit, and the SDK's later automatic attempts on it use it too. This lets one deposit be treated differently without changing the configuration:

- Above the instant claim fee, it lets that deposit be claimed early. The SDK keeps retrying, so there is no need to call `ClaimDeposit` again.
- Below the instant claim fee, it holds that deposit back from an early claim while the others still claim early.

The standard claim runs under the larger of the deposit's own max fee and the configured one. A raised max fee therefore covers the standard claim too, while a lowered one only restricts the early claim.

- Calling `ClaimDeposit` without a `MaxFee` claims under the configured max claim fee and clears the deposit's own.
- The max fee is recorded before the claim is attempted, so it stands whatever the outcome.
- It is readable as `MaxClaimFee` on the deposit, unset while the configured max claim fee applies.
- It is kept on this device only. Another device sharing the wallet applies its own configuration to the same deposit.

## Writing your own claim logic

For rules the [max claim fee](config.md#max-deposit-claim-fee) cannot express, such as claiming only above an amount, at certain times, or under a fee limit capped in sats, claim with `ClaimDeposit` yourself. Keep automatic claiming on and handle only the deposits it leaves unclaimed, or unset the max claim fee to decide every claim.

A deposit whose claim exceeded its max fee carries the total the claim would cost, `RequiredFeeSats`, in its `ClaimError`. This example accepts up to 1% of the deposit, but never more than 5,000 sats:

```go
if claimErr := *deposit.ClaimError; claimErr != nil {
	if exceeded, ok := claimErr.(breez_sdk_spark.DepositClaimErrorMaxDepositClaimFeeExceeded); ok {
		requiredFee := exceeded.RequiredFeeSats

		// Accept up to 1% of the deposit amount, but never more than 5,000 sats
		maxFeeSats := deposit.AmountSats / 100
		if maxFeeSats > 5_000 {
			maxFeeSats = 5_000
		}
		if requiredFee <= maxFeeSats {
			maxFee := breez_sdk_spark.MaxFee(breez_sdk_spark.MaxFeeFixed{Amount: requiredFee})
			claimRequest := breez_sdk_spark.ClaimDepositRequest{
				Txid:   deposit.Txid,
				Vout:   deposit.Vout,
				MaxFee: &maxFee,
			}
			_, err := sdk.ClaimDeposit(claimRequest)
			if err != nil {
				var sdkErr *breez_sdk_spark.SdkError
				if errors.As(err, &sdkErr) {
					// Handle SdkError - can inspect specific variants if needed
					// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
				}
				return err
			}
		}
	}
}
```
