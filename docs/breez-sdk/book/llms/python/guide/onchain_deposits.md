# Tracking deposits

## Listing deposits

`list_unclaimed_deposits` returns every deposit the SDK is tracking, from the moment it reaches the mempool until its output is spent by a claim or a refund.

```python
try:
    request = ListUnclaimedDepositsRequest()
    response = await sdk.list_unclaimed_deposits(request=request)

    for deposit in response.deposits:
        logging.info(f"Unclaimed deposit: {deposit.txid}:{deposit.vout}")
        logging.info(f"Amount: {deposit.amount_sats} sats")

        if deposit.claim_error:
            if isinstance(
                deposit.claim_error, DepositClaimError.MAX_DEPOSIT_CLAIM_FEE_EXCEEDED
            ):
                max_fee_sats = deposit.claim_error.max_fee_sats
                max_fee_str = "none" if max_fee_sats is None else f"{max_fee_sats} sats"
                logging.info(
                    f"Max claim fee exceeded. Max: {max_fee_str}, "
                    f"Required: {deposit.claim_error.required_fee_sats} sats"
                )
            elif isinstance(deposit.claim_error, DepositClaimError.MISSING_UTXO):
                logging.info("Claim failed: UTXO not found")
            elif isinstance(deposit.claim_error, DepositClaimError.DEPOSIT_TOO_SMALL):
                logging.info("Claim failed: deposit too small to claim")
            elif isinstance(deposit.claim_error, DepositClaimError.GENERIC):
                logging.info(f"Claim failed: {deposit.claim_error.message}")
except Exception as error:
    logging.error(error)
    raise
```



## Deposit states

What to show for a deposit follows from a few of its fields. The first row that matches applies.

| State | How to tell | What to show |
|---|---|---|
| Claimed | `instant_claim_status` is `InstantClaimStatus.CLAIMED` | Settled. No action needed. |
| Being claimed | `instant_claim_status` is `InstantClaimStatus.SUBMITTED` | Being credited |
| Refunding | `refund_state` is set | The refund's progress. See [Tracking a refund](onchain_refunds.md#tracking-a-refund). |
| Claim failed | `claim_error` is set | The reason, with the option to [claim manually](onchain_manual_claims.md) or [refund](onchain_refunds.md) |
| Pending | None of the above | Pending. The SDK claims it automatically. |

A claimed deposit stays in the list until the provider spends its output, some time after the credit. When the SDK claims a deposit on its own it emits `SdkEvent.CLAIMED_DEPOSITS` as the claim is submitted, so a deposit can appear in that event and in the list at the same time.

A deposit claimed by another instance sharing the wallet, or on another device, becomes `InstantClaimStatus.CLAIMED` the next time the SDK tries to claim it. No `SdkEvent.CLAIMED_DEPOSITS` event is emitted for it, since the claim was not made here, but the credit still arrives as a payment.

## Seeing deposits before they confirm

A deposit arrives through `SdkEvent.NEW_DEPOSITS` and appears in `list_unclaimed_deposits` as soon as it reaches the mempool, with `is_mature` false until it reaches the standard claim depth.

**Developer note**

The Spark operators only report confirmed deposits, so the SDK finds unconfirmed ones by asking its chain service about recently used deposit addresses, one request per address per sync. Requesting a receive address watches it for 24 hours, and requesting it again restarts that window. An address that received a deposit stays watched until the deposit confirms.
