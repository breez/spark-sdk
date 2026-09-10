# Claiming on-chain deposits

On-chain deposits go through three stages. Once detected, the deposit is visible in the SDK and each deposit includes a `is_mature` field. After **3 on-chain confirmations** the deposit has sufficient confirmations (`is_mature` is true) and the SDK [automatically attempts](#setting-a-max-fee-for-automatic-claims) to claim it. The SDK also claims automatically [before maturity](#claiming-before-maturity) when the configured ceiling covers the provider's spread, so a deposit can be credited sooner than 3 confirmations. If the maximum deposit claim fee is too low for either, the deposit won't be automatically claimed and should be [manually claimed](#manually-claiming-deposits).

## Setting a max fee for automatic claims

The [maximum deposit claim fee](config.md#max-deposit-claim-fee) setting in the SDK configuration defines the maximum fee the SDK uses when automatically claiming an on-chain deposit. The SDK's default fee limit is set to 1 sats/vbyte, which is low and requires manual claiming when fees exceed this threshold. You can set a higher fee, either in sats/vbyte, in absolute sats, or to the fastest recommended fee at the time of claim, with a leeway in sats/vbyte.

This ceiling is not only an on-chain fee tolerance. It also caps what the provider may take to credit a deposit [before it matures](#claiming-before-maturity), so the value you choose decides both how much on-chain fee the SDK will pay and whether deposits are claimed early at all.

To increase the likelihood of automatically claiming deposits, you may set the maximum fee to the fastest recommended rate at the time of claim, which can result in higher fees.

```python
# Create the default config
config = default_config(network=Network.MAINNET)
config.api_key = "<breez api key>"

# Set the maximum fee to the fastest network recommended fee at the time of claim
# with a leeway of 1 sats/vbyte
config.max_deposit_claim_fee = MaxFee.NETWORK_RECOMMENDED(leeway_sat_per_vbyte=1)
```



However, even when setting a high fee, the SDK might still fail to automatically claim deposits. In these cases, it's recommended to manually claim them by letting the end user accept the required fees. When [manual intervention](#manually-claiming-deposits) is required, the SDK emits an `SdkEvent.UNCLAIMED_DEPOSITS` event containing information about the deposit. See [Listening to events](events.md) for how to subscribe to events.

## Claiming before maturity

A deposit does not have to wait for maturity. The Spark Service Provider will front the credited amount earlier and take a spread for carrying the risk, and the SDK claims this way automatically whenever the spread fits within the [maximum deposit claim fee](config.md#max-deposit-claim-fee). The default of 1 sat/vbyte works out to about 99 sats, below any spread the provider charges, so deposits are claimed at maturity until the ceiling is raised enough to cover one. The same applies to `claim_deposit`, which claims a not-yet-mature deposit early when its own `max_fee` allows.

The spread is largely the on-chain cost of the provider's claim plus a percentage of the deposit, so it grows with the deposit.

## Manually claiming deposits

When a deposit cannot be automatically claimed due to the configured maximum fee being too low, you can manually claim it by specifying a higher fee limit. The recommended approach is to display a user interface showing the required fee amount and request user approval before proceeding with manual claiming.

Claiming a deposit the SDK is already claiming, whether from a background attempt or another call, returns `SdkError.DEPOSIT_CLAIM_IN_PROGRESS`. The claim already running may still succeed, so treat this as transient rather than as a failure to show the user.

```python
try:
    if isinstance(
        deposit.claim_error, DepositClaimError.MAX_DEPOSIT_CLAIM_FEE_EXCEEDED
    ):
        required_fee = deposit.claim_error.required_fee_sats

        # Show UI to user with the required fee and get approval
        user_approved = True  # Replace with actual user approval logic

        if user_approved:
            claim_request = ClaimDepositRequest(
                txid=deposit.txid,
                vout=deposit.vout,
                max_fee=Fee.FIXED(amount=required_fee),
            )
            await sdk.claim_deposit(request=claim_request)
except Exception as error:
    logging.error(error)
    raise
```



### Showing the choice to the user

`fetch_claim_deposit_quote` prices both ways of claiming a deposit, so an app can offer the choice rather than deciding for the user. It returns the deposit's current `confirmations` alongside a quote for claiming early and one for claiming at maturity, each carrying the fee and the `confirmations_required`, which is the depth it becomes claimable at rather than a count of blocks still to wait. Subtract the deposit's current confirmations for that: an early claim claimable at 1 confirmation, on a deposit with 0, is available a block from now.

The early quote is absent when the provider will not front this particular deposit, and when claiming early would not actually be earlier: once a deposit has matured, or when the provider would only credit at maturity's own depth, waiting is both cheaper and no slower, so there is no choice left to offer. The early quote is priced whether or not the configured [maximum deposit claim fee](config.md#max-deposit-claim-fee) would allow it, since that ceiling is usually far below a spread and the point of the quote is to let the user decide. Acting on it therefore means passing a `max_fee` to `claim_deposit` of at least the quoted `fee_sats`; with a lower one the call returns `SdkError.MAX_DEPOSIT_CLAIM_FEE_EXCEEDED` and the deposit waits for maturity instead. The quote for maturity is always present, but may be flagged `is_estimate` when the provider will not quote a deposit this early, in which case the fee is derived from current on-chain fees and the final one may differ.

A claim made before maturity settles asynchronously, so `claim_deposit` returns no payment. Watch for it via `list_payments` or the [payment events](events.md).

```python
try:
    request = FetchClaimDepositQuoteRequest(txid=deposit.txid, vout=deposit.vout)
    quote = await sdk.fetch_claim_deposit_quote(request=request)

    # Claiming once the deposit matures, and how many blocks that is away.
    blocks_to_wait = max(
        0, quote.mature.confirmations_required - quote.confirmations
    )
    logging.info(
        f"Wait {blocks_to_wait} blocks and pay {quote.mature.fee_sats} sats"
    )

    # Claiming earlier, when the provider offers it.
    if quote.instant is not None:
        blocks_to_wait = max(
            0, quote.instant.confirmations_required - quote.confirmations
        )
        logging.info(
            f"Or wait {blocks_to_wait} blocks and pay {quote.instant.fee_sats} sats"
        )
except Exception as error:
    logging.error(error)
    raise
```



## Listing unclaimed deposits

Retrieve all deposits that have not yet been claimed. This includes pending deposits that do not yet have sufficient confirmations, as well as deposits with sufficient confirmations that failed to claim (with the specific failure reason). Pending deposits will be automatically claimed once they have sufficient confirmations, or sooner if the configured ceiling covers an early claim.

A deposit claimed before maturity stays in the list with its `instant_claim_status` set to `InstantClaimStatus.SUBMITTED` for a short time after submission, and is removed once the claim settles. When the SDK claims automatically it emits `SdkEvent.CLAIMED_DEPOSITS` at submission, so a deposit can briefly appear both in that event and in this list.

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
                max_fee_str = "none"
                if deposit.claim_error.max_fee is not None:
                    if isinstance(deposit.claim_error.max_fee, Fee.FIXED):
                        max_fee_str = f"{deposit.claim_error.max_fee.amount} sats"
                    elif isinstance(deposit.claim_error.max_fee, Fee.RATE):
                        max_fee_str = f"{deposit.claim_error.max_fee.sat_per_vbyte} sats/vByte"
                logging.info(
                    f"Claim failed: Fee exceeded. Max: {max_fee_str}, "
                    f"Required: {deposit.claim_error.required_fee_sats} sats "
                    f"or {deposit.claim_error.required_fee_rate_sat_per_vbyte} sats/vByte"
                )
            elif isinstance(deposit.claim_error, DepositClaimError.MISSING_UTXO):
                logging.info("Claim failed: UTXO not found")
            elif isinstance(deposit.claim_error, DepositClaimError.GENERIC):
                logging.info(f"Claim failed: {deposit.claim_error.message}")
except Exception as error:
    logging.error(error)
    raise
```



## Refunding deposits

When a deposit cannot be successfully claimed you can refund it to an external Bitcoin address. This creates a transaction that sends the amount (minus transaction fees) to the specified destination address.

The [recommended fees](#recommended-fees) API is useful for determining appropriate fee levels for refund transactions.

A deposit can only be refunded once it has enough confirmations. Calling `refund_deposit` earlier fails, reporting the deposit as unknown while it is unconfirmed and as having too few confirmations for a block or so after that. Nothing is signed or stored when this happens, so retry after a few more blocks.

```python
try:
    txid = "your_deposit_txid"
    vout = 0
    destination_address = "bc1qexample..."  # Your Bitcoin address

    # Set the fee for the refund transaction using the half-hour feerate
    recommended_fees = await sdk.recommended_fees()
    fee = Fee.RATE(sat_per_vbyte=recommended_fees.half_hour_fee)
    # or using a fixed amount
    #fee = Fee.FIXED(amount=500)
    #

    request = RefundDepositRequest(
        txid=txid, vout=vout, destination_address=destination_address, fee=fee
    )

    response = await sdk.refund_deposit(request=request)
    logging.info("Refund transaction created:")
    logging.info(f"Transaction ID: {response.tx_id}")
    logging.info(f"Transaction hex: {response.tx_hex}")
except Exception as error:
    logging.error(error)
    raise
```



**Developer note**

The total fee must cover at least 1 sat/vB of the refund transaction so it can be relayed by the Bitcoin network. The exact minimum depends on the size of the transaction, which varies with the destination address type: around 99 sats to a native segwit address and 111 sats to a taproot one. If the fee is lower, the refund request is rejected and the error states the required minimum.

### Tracking a refund

`refund_state` on `DepositInfo` reports how far the refund has got:

- **`RefundState.BROADCAST_PENDING`**: the refund is signed and stored but has not been seen on the network. The SDK rebroadcasts it on every sync until the deposit is spent, so a refund that failed to send because of a temporary network problem recovers on its own.
- **`RefundState.BROADCAST`**: the network has accepted the refund and it is waiting to confirm. The deposit disappears from `list_unclaimed_deposits` once it does.

A refund created near the 1 sat/vB minimum can stay at `RefundState.BROADCAST_PENDING` indefinitely if the network's minimum relay fee later rises above what it pays. Rebroadcasting cannot fix this, because the network keeps refusing the same transaction. Read `last_error` for the reason the network gave, then call `refund_deposit` again at a higher fee to replace it.

Replacing a refund that is already on the network costs more than the original fee, because the replacement also pays to relay its own size. When the fee offered is too low, the call is rejected and the error states the minimum required.

## Implementing a custom claim logic

For advanced use cases, you may want to implement a custom claim logic instead of relying on the SDK's automatic process. This gives you complete control over when and how deposits are claimed.

To disable automatic claims, unset the [maximum deposit claim fee](config.md#max-deposit-claim-fee). Then use the methods described above to manually claim deposits based on your business logic.

Common scenarios for custom claiming logic include:

- **Dynamic fee adjustment**: Adjust claiming fees based on market conditions or priority
- **Conditional claiming**: Only claim deposits that meet certain criteria (amount thresholds, time windows, etc.)
- **Integration with external systems**: Coordinate claims with other business processes

The [recommended fees](#recommended-fees) API is useful for determining appropriate fee levels for claiming deposits. For example, you can implement a custom claim logic to only claim deposits if the required fee rate is less than the fastest recommended fee (or any other).

```python
try:
    if isinstance(
        deposit.claim_error, DepositClaimError.MAX_DEPOSIT_CLAIM_FEE_EXCEEDED
    ):
        required_fee_rate = deposit.claim_error.required_fee_rate_sat_per_vbyte

        recommended_fees = await sdk.recommended_fees()

        if required_fee_rate <= recommended_fees.fastest_fee:
            claim_request = ClaimDepositRequest(
                txid=deposit.txid,
                vout=deposit.vout,
                max_fee=MaxFee.RATE(sat_per_vbyte=required_fee_rate),
            )
            await sdk.claim_deposit(request=claim_request)
except Exception as error:
    logging.error(error)
    raise
```



## Recommended fees

Get Bitcoin fee estimates for different confirmation targets to help determine appropriate fee levels for claiming or refunding deposits.

```python
response = await sdk.recommended_fees()
logging.info(f"Fastest fee: {response.fastest_fee} sats/vByte")
logging.info(f"Half-hour fee: {response.half_hour_fee} sats/vByte")
logging.info(f"Hour fee: {response.hour_fee} sats/vByte")
logging.info(f"Economy fee: {response.economy_fee} sats/vByte")
logging.info(f"Minimum fee: {response.minimum_fee} sats/vByte")
```
