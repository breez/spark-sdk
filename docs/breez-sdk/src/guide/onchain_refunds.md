# Refunding deposits

A deposit that cannot or should not be claimed can be refunded to an external Bitcoin address. The refund sends the deposit amount, minus the transaction fee, to that address.

{{#tabs refunding_payments:refund-deposit}}

- A claimed deposit cannot be refunded. Check that its {{#name instant_claim_status}} is not {{#enum InstantClaimStatus::Claimed}} before offering a refund.
- A deposit can only be refunded once it has enough confirmations. Before that, {{#name refund_deposit}} fails and nothing is signed or stored, so retry after a few more blocks.
- The fee must cover at least 1 sat/vB of the refund transaction: around 99 sats to a native segwit address and 111 sats to a taproot one. A lower fee is rejected, and the error states the minimum.

[Recommended fees](#recommended-fees) help choose the fee.

## Tracking a refund

{{#name refund_state}} on the deposit reports the refund's progress:

- {{#enum RefundState::BroadcastPending}}: signed and stored, but not yet seen on the network. The SDK rebroadcasts it on every sync, so a temporary network problem resolves on its own.
- {{#enum RefundState::Broadcast}}: accepted by the network and waiting to confirm. The deposit leaves {{#name list_unclaimed_deposits}} once it confirms.

A refund paying close to the minimum can stay at {{#enum RefundState::BroadcastPending}} if the network's minimum relay fee rises above what it pays, and rebroadcasting cannot fix that. Read {{#name last_error}} for the network's reason, then call {{#name refund_deposit}} again with a higher fee to replace it. The replacement has to pay more than the original, and a fee too low to replace it is rejected with the minimum in the error.

## Recommended fees

{{#name recommended_fees}} returns fee rate estimates for different confirmation targets.

{{#tabs refunding_payments:recommended-fees}}
