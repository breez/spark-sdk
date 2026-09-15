# Claiming on-chain deposits

On-chain deposits go through three stages. Once detected, the deposit is visible in the SDK and each deposit includes a `is_mature` field. After **3 on-chain confirmations** the deposit has sufficient confirmations (`is_mature` is true) and the SDK [automatically attempts](#setting-a-max-fee-for-automatic-claims) to claim it. The SDK also claims automatically [before maturity](#claiming-before-maturity) when the configured ceiling covers the provider's spread, so a deposit can be credited sooner than 3 confirmations. If the maximum deposit claim fee is too low for either, the deposit won't be automatically claimed and should be [manually claimed](#manually-claiming-deposits).

## Setting a max fee for automatic claims

The [maximum deposit claim fee](config.md#max-deposit-claim-fee) setting in the SDK configuration defines the maximum fee the SDK uses when automatically claiming an on-chain deposit. The SDK's default fee limit is set to 1 sats/vbyte, which is low and requires manual claiming when fees exceed this threshold. You can set a higher fee, either in sats/vbyte, in absolute sats, or to the fastest recommended fee at the time of claim, with a leeway in sats/vbyte.

This ceiling is not only an on-chain fee tolerance. It also caps what the provider may take to credit a deposit [before it matures](#claiming-before-maturity), so the value you choose decides both how much on-chain fee the SDK will pay and whether deposits are claimed early at all.

To increase the likelihood of automatically claiming deposits, you may set the maximum fee to the fastest recommended rate at the time of claim, which can result in higher fees.

```rust
// Create the default config
let mut config = default_config(Network::Mainnet);
config.api_key = Some("<breez api key>".to_string());

// Set the maximum fee to the fastest network recommended fee at the time of claim
// with a leeway of 1 sats/vbyte
config.max_deposit_claim_fee = Some(MaxFee::NetworkRecommended {
    leeway_sat_per_vbyte: 1,
});
```



However, even when setting a high fee, the SDK might still fail to automatically claim deposits. In these cases, it's recommended to manually claim them by letting the end user accept the required fees. When [manual intervention](#manually-claiming-deposits) is required, the SDK emits an `SdkEvent::UnclaimedDeposits` event containing information about the deposit. See [Listening to events](events.md) for how to subscribe to events.

## Claiming before maturity

A deposit does not have to wait for maturity. The Spark Service Provider will front the credited amount earlier and take a spread for carrying the risk, and the SDK claims this way automatically whenever the spread fits within the [maximum deposit claim fee](config.md#max-deposit-claim-fee). The default of 1 sat/vbyte works out to about 99 sats, below any spread the provider charges, so deposits are claimed at maturity until the ceiling is raised enough to cover one. The same applies to `claim_deposit`, which claims a not-yet-mature deposit early when its own `max_fee` allows.

The spread is largely the on-chain cost of the provider's claim plus a percentage of the deposit, so it grows with the deposit.

## Manually claiming deposits

When a deposit cannot be automatically claimed due to the configured maximum fee being too low, you can manually claim it by specifying a higher fee limit. The recommended approach is to display a user interface showing the required fee amount and request user approval before proceeding with manual claiming.

Claiming a deposit the SDK is already claiming, whether from a background attempt or another call, returns `SdkError::DepositClaimInProgress`. The claim already running may still succeed, so treat this as transient rather than as a failure to show the user.

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
            max_fee: Some(MaxFee::Fixed {
                amount: *required_fee_sats,
            }),
        };
        sdk.claim_deposit(request).await?;
    }
}
```



### Showing the choice to the user

`fetch_claim_deposit_quote` prices both ways of claiming a deposit, so an app can offer the choice rather than deciding for the user. It returns the deposit's current `confirmations` alongside a quote for claiming early and one for claiming at maturity, each carrying the fee and the `confirmations_required`, which is the depth it becomes claimable at rather than a count of blocks still to wait. Subtract the deposit's current confirmations for that: an early claim claimable at 1 confirmation, on a deposit with 0, is available a block from now.

The early quote is absent when the provider will not front this particular deposit, and when claiming early would not actually be earlier: once a deposit has matured, or when the provider would only credit at maturity's own depth, waiting is both cheaper and no slower, so there is no choice left to offer. The early quote is priced whether or not the configured [maximum deposit claim fee](config.md#max-deposit-claim-fee) would allow it, since that ceiling is usually far below a spread and the point of the quote is to let the user decide. Acting on it therefore means passing a `max_fee` to `claim_deposit` of at least the quoted `fee_sats`; with a lower one the call returns `SdkError::MaxDepositClaimFeeExceeded` and the deposit waits for maturity instead. The quote for maturity is always present, but may be flagged `is_estimate` when the provider will not quote a deposit this early, in which case the fee is derived from current on-chain fees and the final one may differ.

A claim made before maturity settles asynchronously, so `claim_deposit` returns no payment. Watch for it via `list_payments` or the [payment events](events.md).

```rust
let quote = sdk
    .fetch_claim_deposit_quote(FetchClaimDepositQuoteRequest {
        txid: deposit.txid.clone(),
        vout: deposit.vout,
    })
    .await?;

// Claiming once the deposit matures, and how many blocks that is away.
let blocks_to_wait = quote
    .mature
    .confirmations_required
    .saturating_sub(quote.confirmations);
info!(
    "Wait {} blocks and pay {} sats",
    blocks_to_wait, quote.mature.fee_sats
);

// Claiming earlier, when the provider offers it.
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



## Listing unclaimed deposits

Retrieve all deposits that have not yet been claimed. This includes pending deposits that do not yet have sufficient confirmations, as well as deposits with sufficient confirmations that failed to claim (with the specific failure reason). Pending deposits will be automatically claimed once they have sufficient confirmations, or sooner if the configured ceiling covers an early claim.

A deposit claimed before maturity stays in the list with its `instant_claim_status` set to `InstantClaimStatus::Submitted` for a short time after submission, and is removed once the claim settles. When the SDK claims automatically it emits `SdkEvent::ClaimedDeposits` at submission, so a deposit can briefly appear both in that event and in this list.

```rust
let request = ListUnclaimedDepositsRequest {};
let response = sdk.list_unclaimed_deposits(request).await?;

for deposit in response.deposits {
    info!("Unclaimed deposit: {}:{}", deposit.txid, deposit.vout);
    info!("Amount: {} sats", deposit.amount_sats);

    if let Some(claim_error) = &deposit.claim_error {
        match claim_error {
            DepositClaimError::MaxDepositClaimFeeExceeded {
                max_fee,
                required_fee_sats,
                required_fee_rate_sat_per_vbyte,
                ..
            } => {
                info!(
                    "Max claim fee exceeded. Max: {:?}, Required: {} sats or {} sats/vByte",
                    max_fee, required_fee_sats, required_fee_rate_sat_per_vbyte
                );
            }
            DepositClaimError::MissingUtxo { .. } => {
                info!("UTXO not found when claiming deposit");
            }
            DepositClaimError::Generic { message } => {
                info!("Claim failed: {}", message);
            }
        }
    }
}
```



## Refunding deposits

When a deposit cannot be successfully claimed you can refund it to an external Bitcoin address. This creates a transaction that sends the amount (minus transaction fees) to the specified destination address.

The [recommended fees](#recommended-fees) API is useful for determining appropriate fee levels for refund transactions.

A deposit can only be refunded once it has enough confirmations. Calling `refund_deposit` earlier fails, reporting the deposit as unknown while it is unconfirmed and as having too few confirmations for a block or so after that. Nothing is signed or stored when this happens, so retry after a few more blocks.

```rust
let txid = "your_deposit_txid".to_string();
let vout = 0;
let destination_address = "bc1qexample...".to_string(); // Your Bitcoin address

// Set the fee for the refund transaction using the half-hour feerate
let recommended_fees = sdk.recommended_fees().await?;
let fee = Fee::Rate {
    sat_per_vbyte: recommended_fees.half_hour_fee,
};
// or using a fixed amount
//let fee = Fee::Fixed { amount: 500 };
//

let request = RefundDepositRequest {
    txid,
    vout,
    destination_address,
    fee,
};

let response = sdk.refund_deposit(request).await?;
info!("Refund transaction created:");
info!("Transaction ID: {}", response.tx_id);
info!("Transaction hex: {}", response.tx_hex);
```



**Developer note**

The total fee must cover at least 1 sat/vB of the refund transaction so it can be relayed by the Bitcoin network. The exact minimum depends on the size of the transaction, which varies with the destination address type: around 99 sats to a native segwit address and 111 sats to a taproot one. If the fee is lower, the refund request is rejected and the error states the required minimum.

### Tracking a refund

`refund_state` on `DepositInfo` reports how far the refund has got:

- **`RefundState::BroadcastPending`**: the refund is signed and stored but has not been seen on the network. The SDK rebroadcasts it on every sync until the deposit is spent, so a refund that failed to send because of a temporary network problem recovers on its own.
- **`RefundState::Broadcast`**: the network has accepted the refund and it is waiting to confirm. The deposit disappears from `list_unclaimed_deposits` once it does.

A refund created near the 1 sat/vB minimum can stay at `RefundState::BroadcastPending` indefinitely if the network's minimum relay fee later rises above what it pays. Rebroadcasting cannot fix this, because the network keeps refusing the same transaction. Read `last_error` for the reason the network gave, then call `refund_deposit` again at a higher fee to replace it.

Replacing a refund that is already on the network costs more than the original fee, because the replacement also pays to relay its own size. When the fee offered is too low, the call is rejected and the error states the minimum required.

## Implementing a custom claim logic

For advanced use cases, you may want to implement a custom claim logic instead of relying on the SDK's automatic process. This gives you complete control over when and how deposits are claimed.

To disable automatic claims, unset the [maximum deposit claim fee](config.md#max-deposit-claim-fee). Then use the methods described above to manually claim deposits based on your business logic.

Common scenarios for custom claiming logic include:

- **Dynamic fee adjustment**: Adjust claiming fees based on market conditions or priority
- **Conditional claiming**: Only claim deposits that meet certain criteria (amount thresholds, time windows, etc.)
- **Integration with external systems**: Coordinate claims with other business processes

The [recommended fees](#recommended-fees) API is useful for determining appropriate fee levels for claiming deposits. For example, you can implement a custom claim logic to only claim deposits if the required fee rate is less than the fastest recommended fee (or any other).

```rust
if let Some(DepositClaimError::MaxDepositClaimFeeExceeded {
    required_fee_rate_sat_per_vbyte,
    ..
}) = &deposit.claim_error
{
    let recommended_fees = sdk.recommended_fees().await?;

    if *required_fee_rate_sat_per_vbyte <= recommended_fees.fastest_fee {
        let request = ClaimDepositRequest {
            txid: deposit.txid.clone(),
            vout: deposit.vout,
            max_fee: Some(MaxFee::Rate {
                sat_per_vbyte: *required_fee_rate_sat_per_vbyte,
            }),
        };
        sdk.claim_deposit(request).await?;
    }
}
```



## Recommended fees

Get Bitcoin fee estimates for different confirmation targets to help determine appropriate fee levels for claiming or refunding deposits.

```rust
let response = sdk.recommended_fees().await?;
info!("Fastest fee: {} sats/vByte", response.fastest_fee);
info!("Half-hour fee: {} sats/vByte", response.half_hour_fee);
info!("Hour fee: {} sats/vByte", response.hour_fee);
info!("Economy fee: {} sats/vByte", response.economy_fee);
info!("Minimum fee: {} sats/vByte", response.minimum_fee);
```
