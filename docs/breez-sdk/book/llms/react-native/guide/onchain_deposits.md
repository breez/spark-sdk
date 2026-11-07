# Tracking deposits

## Listing deposits

`listUnclaimedDeposits` returns every deposit the SDK is tracking, from the moment it reaches the mempool until its output is spent by a claim or a refund.

```typescript
const request: ListUnclaimedDepositsRequest = {}
const response = await sdk.listUnclaimedDeposits(request)

for (const deposit of response.deposits) {
  console.log(`Unclaimed deposit: ${deposit.txid}:${deposit.vout}`)
  console.log(`Amount: ${deposit.amountSats} sats`)

  if (deposit.claimError != null) {
    if (deposit.claimError?.tag === DepositClaimError_Tags.MaxDepositClaimFeeExceeded) {
      let maxFeeStr = 'none'
      if (deposit.claimError.inner.maxFeeSats != null) {
        maxFeeStr = `${deposit.claimError.inner.maxFeeSats} sats`
      }
      console.log(
        `Max claim fee exceeded. Max: ${maxFeeStr}, ` +
        `Required: ${deposit.claimError.inner.requiredFeeSats} sats`
      )
    } else if (deposit.claimError?.tag === DepositClaimError_Tags.MissingUtxo) {
      console.log('UTXO not found when claiming deposit')
    } else if (deposit.claimError?.tag === DepositClaimError_Tags.DepositTooSmall) {
      console.log('Deposit too small to claim')
    } else if (deposit.claimError?.tag === DepositClaimError_Tags.Generic) {
      console.log(`Claim failed: ${deposit.claimError.inner.message}`)
    }
  }
}
```



## Deposit states

What to show for a deposit follows from a few of its fields. The first row that matches applies.

| State | How to tell | What to show |
|---|---|---|
| Claimed | `instantClaimStatus` is `InstantClaimStatus.Claimed` | Settled. No action needed. |
| Being claimed | `instantClaimStatus` is `InstantClaimStatus.Submitted` | Being credited |
| Refunding | `refundState` is set | The refund's progress. See [Tracking a refund](onchain_refunds.md#tracking-a-refund). |
| Claim failed | `claimError` is set | The reason, with the option to [claim manually](onchain_manual_claims.md) or [refund](onchain_refunds.md) |
| Pending | None of the above | Pending. The SDK claims it automatically. |

A claimed deposit stays in the list until the provider spends its output, some time after the credit. When the SDK claims a deposit on its own it emits `SdkEvent.ClaimedDeposits` as the claim is submitted, so a deposit can appear in that event and in the list at the same time.

A deposit claimed by another instance sharing the wallet, or on another device, becomes `InstantClaimStatus.Claimed` the next time the SDK tries to claim it. No `SdkEvent.ClaimedDeposits` event is emitted for it, since the claim was not made here, but the credit still arrives as a payment.

## Seeing deposits before they confirm

A deposit arrives through `SdkEvent.NewDeposits` and appears in `listUnclaimedDeposits` as soon as it reaches the mempool, with `isMature` false until it reaches the standard claim depth.

**Developer note**

The Spark operators only report confirmed deposits, so the SDK finds unconfirmed ones by asking its chain service about recently used deposit addresses, one request per address per sync. Requesting a receive address watches it for 24 hours, and requesting it again restarts that window. An address that received a deposit stays watched until the deposit confirms.
