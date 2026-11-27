# Tracking deposits

## Listing deposits

`ListUnclaimedDeposits` returns every deposit the SDK is tracking, from the moment it reaches the mempool until its output is spent by a claim or a refund.

```csharp
var request = new ListUnclaimedDepositsRequest();
var response = await sdk.ListUnclaimedDeposits(request: request);

foreach (var deposit in response.deposits)
{
    Console.WriteLine($"Unclaimed deposit: {deposit.txid}:{deposit.vout}");
    Console.WriteLine($"Amount: {deposit.amountSats} sats");

    if (deposit.claimError != null)
    {
        if (deposit.claimError is DepositClaimError.MaxDepositClaimFeeExceeded exceeded)
        {
            var maxFeeStr = exceeded.maxFeeSats is ulong sats ? $"{sats} sats" : "none";
            Console.WriteLine($"Max claim fee exceeded. Max: {maxFeeStr}, " +
                            $"Required: {exceeded.requiredFeeSats} sats");
        }
        else if (deposit.claimError is DepositClaimError.MissingUtxo)
        {
            Console.WriteLine("Claim failed: UTXO not found");
        }
        else if (deposit.claimError is DepositClaimError.DepositTooSmall)
        {
            Console.WriteLine("Claim failed: deposit too small to claim");
        }
        else if (deposit.claimError is DepositClaimError.Generic generic)
        {
            Console.WriteLine($"Claim failed: {generic.message}");
        }
    }
}
```



## Deposit states

What to show for a deposit follows from a few of its fields. The first row that matches applies.

| State | How to tell | What to show |
|---|---|---|
| Claimed | `InstantClaimStatus` is `InstantClaimStatus.Claimed` | Settled. No action needed. |
| Being claimed | `InstantClaimStatus` is `InstantClaimStatus.Submitted` | Being credited |
| Refunding | `RefundState` is set | The refund's progress. See [Tracking a refund](onchain_refunds.md#tracking-a-refund). |
| Claim failed | `ClaimError` is set | The reason, with the option to [claim manually](onchain_manual_claims.md) or [refund](onchain_refunds.md) |
| Pending | None of the above | Pending. The SDK claims it automatically. |

A claimed deposit stays in the list until the provider spends its output, some time after the credit. When the SDK claims a deposit on its own it emits `SdkEvent.ClaimedDeposits` as the claim is submitted, so a deposit can appear in that event and in the list at the same time.

A deposit claimed by another instance sharing the wallet, or on another device, becomes `InstantClaimStatus.Claimed` the next time the SDK tries to claim it. No `SdkEvent.ClaimedDeposits` event is emitted for it, since the claim was not made here, but the credit still arrives as a payment.

## Seeing deposits before they confirm

A deposit arrives through `SdkEvent.NewDeposits` and appears in `ListUnclaimedDeposits` as soon as it reaches the mempool, with `IsMature` false until it reaches the standard claim depth.

**Developer note**

The Spark operators only report confirmed deposits, so the SDK finds unconfirmed ones by asking its chain service about recently used deposit addresses, one request per address per sync. Requesting a receive address watches it for 24 hours, and requesting it again restarts that window. An address that received a deposit stays watched until the deposit confirms.
