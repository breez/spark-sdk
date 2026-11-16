# Refunding deposits

A deposit that cannot or should not be claimed can be refunded to an external Bitcoin address. The refund sends the deposit amount, minus the transaction fee, to that address.

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



- A claimed deposit cannot be refunded. Check that its `instant_claim_status` is not `InstantClaimStatus.CLAIMED` before offering a refund.
- A deposit can only be refunded once it has enough confirmations. Before that, `refund_deposit` fails and nothing is signed or stored, so retry after a few more blocks.
- The fee must cover at least 1 sat/vB of the refund transaction: around 99 sats to a native segwit address and 111 sats to a taproot one. A lower fee is rejected, and the error states the minimum.

[Recommended fees](#recommended-fees) help choose the fee.

## Tracking a refund

`refund_state` on the deposit reports the refund's progress:

- `RefundState.BROADCAST_PENDING`: signed and stored, but not yet seen on the network. The SDK rebroadcasts it on every sync, so a temporary network problem resolves on its own.
- `RefundState.BROADCAST`: accepted by the network and waiting to confirm. The deposit leaves `list_unclaimed_deposits` once it confirms.

A refund paying close to the minimum can stay at `RefundState.BROADCAST_PENDING` if the network's minimum relay fee rises above what it pays, and rebroadcasting cannot fix that. Read `last_error` for the network's reason, then call `refund_deposit` again with a higher fee to replace it. The replacement has to pay more than the original, and a fee too low to replace it is rejected with the minimum in the error.

## Recommended fees

`recommended_fees` returns fee rate estimates for different confirmation targets.

```python
response = await sdk.recommended_fees()
logging.info(f"Fastest fee: {response.fastest_fee} sats/vByte")
logging.info(f"Half-hour fee: {response.half_hour_fee} sats/vByte")
logging.info(f"Hour fee: {response.hour_fee} sats/vByte")
logging.info(f"Economy fee: {response.economy_fee} sats/vByte")
logging.info(f"Minimum fee: {response.minimum_fee} sats/vByte")
```
