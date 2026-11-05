# Refunding deposits

A deposit that cannot or should not be claimed can be refunded to an external Bitcoin address. The refund sends the deposit amount, minus the transaction fee, to that address.

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



- A claimed deposit cannot be refunded. Check that its `instant_claim_status` is not `InstantClaimStatus::Claimed` before offering a refund.
- A deposit can only be refunded once it has enough confirmations. Before that, `refund_deposit` fails and nothing is signed or stored, so retry after a few more blocks.
- The fee must cover at least 1 sat/vB of the refund transaction: around 99 sats to a native segwit address and 111 sats to a taproot one. A lower fee is rejected, and the error states the minimum.

[Recommended fees](#recommended-fees) help choose the fee.

## Tracking a refund

`refund_state` on the deposit reports the refund's progress:

- `RefundState::BroadcastPending`: signed and stored, but not yet seen on the network. The SDK rebroadcasts it on every sync, so a temporary network problem resolves on its own.
- `RefundState::Broadcast`: accepted by the network and waiting to confirm. The deposit leaves `list_unclaimed_deposits` once it confirms.

A refund paying close to the minimum can stay at `RefundState::BroadcastPending` if the network's minimum relay fee rises above what it pays, and rebroadcasting cannot fix that. Read `last_error` for the network's reason, then call `refund_deposit` again with a higher fee to replace it. The replacement has to pay more than the original, and a fee too low to replace it is rejected with the minimum in the error.

## Recommended fees

`recommended_fees` returns fee rate estimates for different confirmation targets.

```rust
let response = sdk.recommended_fees().await?;
info!("Fastest fee: {} sats/vByte", response.fastest_fee);
info!("Half-hour fee: {} sats/vByte", response.half_hour_fee);
info!("Hour fee: {} sats/vByte", response.hour_fee);
info!("Economy fee: {} sats/vByte", response.economy_fee);
info!("Minimum fee: {} sats/vByte", response.minimum_fee);
```
