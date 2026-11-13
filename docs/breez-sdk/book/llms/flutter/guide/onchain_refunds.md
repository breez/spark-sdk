# Refunding deposits

A deposit that cannot or should not be claimed can be refunded to an external Bitcoin address. The refund sends the deposit amount, minus the transaction fee, to that address.

```dart
String txid = "your_deposit_txid";
int vout = 0;
String destinationAddress = "bc1qexample..."; // Your Bitcoin address

// Set the fee for the refund transaction using the half-hour feerate
final recommendedFees = await sdk.recommendedFees();
Fee fee = Fee.rate(satPerVbyte: recommendedFees.halfHourFee);
// or using a fixed amount
//Fee fee = Fee.fixed(amount: BigInt.from(500));
//

final request = RefundDepositRequest(
  txid: txid,
  vout: vout,
  destinationAddress: destinationAddress,
  fee: fee,
);

final response = await sdk.refundDeposit(request: request);
print("Refund transaction created:");
print("Transaction ID: ${response.txId}");
print("Transaction hex: ${response.txHex}");
```



- A claimed deposit cannot be refunded. Check that its `instantClaimStatus` is not `InstantClaimStatus.Claimed` before offering a refund.
- A deposit can only be refunded once it has enough confirmations. Before that, `refundDeposit` fails and nothing is signed or stored, so retry after a few more blocks.
- The fee must cover at least 1 sat/vB of the refund transaction: around 99 sats to a native segwit address and 111 sats to a taproot one. A lower fee is rejected, and the error states the minimum.

[Recommended fees](#recommended-fees) help choose the fee.

## Tracking a refund

`refundState` on the deposit reports the refund's progress:

- `RefundState.BroadcastPending`: signed and stored, but not yet seen on the network. The SDK rebroadcasts it on every sync, so a temporary network problem resolves on its own.
- `RefundState.Broadcast`: accepted by the network and waiting to confirm. The deposit leaves `listUnclaimedDeposits` once it confirms.

A refund paying close to the minimum can stay at `RefundState.BroadcastPending` if the network's minimum relay fee rises above what it pays, and rebroadcasting cannot fix that. Read `lastError` for the network's reason, then call `refundDeposit` again with a higher fee to replace it. The replacement has to pay more than the original, and a fee too low to replace it is rejected with the minimum in the error.

## Recommended fees

`recommendedFees` returns fee rate estimates for different confirmation targets.

```dart
final response = await sdk.recommendedFees();
print("Fastest fee: ${response.fastestFee} sats/vByte");
print("Half-hour fee: ${response.halfHourFee} sats/vByte");
print("Hour fee: ${response.hourFee} sats/vByte");
print("Economy fee: ${response.economyFee} sats/vByte");
print("Minimum fee: ${response.minimumFee} sats/vByte");
```
