# Refunding deposits

A deposit that cannot or should not be claimed can be refunded to an external Bitcoin address. The refund sends the deposit amount, minus the transaction fee, to that address.

```csharp
var txid = "your_deposit_txid";
var vout = 0U;
var destinationAddress = "bc1qexample...";  // Your Bitcoin address

// Set the fee for the refund transaction using the half-hour feerate
var recommendedFees = await sdk.RecommendedFees();
var fee = new Fee.Rate(satPerVbyte: recommendedFees.halfHourFee);
// or using a fixed amount
//var fee = new Fee.Fixed(amount: 500);
//

var request = new RefundDepositRequest(
    txid: txid,
    vout: vout,
    destinationAddress: destinationAddress,
    fee: fee
);

var response = await sdk.RefundDeposit(request: request);
Console.WriteLine("Refund transaction created:");
Console.WriteLine($"Transaction ID: {response.txId}");
Console.WriteLine($"Transaction hex: {response.txHex}");
```



- A claimed deposit cannot be refunded. Check that its `InstantClaimStatus` is not `InstantClaimStatus.Claimed` before offering a refund.
- A deposit can only be refunded once it has enough confirmations. Before that, `RefundDeposit` fails and nothing is signed or stored, so retry after a few more blocks.
- The fee must cover at least 1 sat/vB of the refund transaction: around 99 sats to a native segwit address and 111 sats to a taproot one. A lower fee is rejected, and the error states the minimum.

[Recommended fees](#recommended-fees) help choose the fee.

## Tracking a refund

`RefundState` on the deposit reports the refund's progress:

- `RefundState.BroadcastPending`: signed and stored, but not yet seen on the network. The SDK rebroadcasts it on every sync, so a temporary network problem resolves on its own.
- `RefundState.Broadcast`: accepted by the network and waiting to confirm. The deposit leaves `ListUnclaimedDeposits` once it confirms.

A refund paying close to the minimum can stay at `RefundState.BroadcastPending` if the network's minimum relay fee rises above what it pays, and rebroadcasting cannot fix that. Read `LastError` for the network's reason, then call `RefundDeposit` again with a higher fee to replace it. The replacement has to pay more than the original, and a fee too low to replace it is rejected with the minimum in the error.

## Recommended fees

`RecommendedFees` returns fee rate estimates for different confirmation targets.

```csharp
var response = await sdk.RecommendedFees();
    Console.WriteLine($"Fastest fee: {response.fastestFee} sats/vByte");
    Console.WriteLine($"Half-hour fee: {response.halfHourFee} sats/vByte");
    Console.WriteLine($"Hour fee: {response.hourFee} sats/vByte");
    Console.WriteLine($"Economy fee: {response.economyFee} sats/vByte");
    Console.WriteLine($"Minimum fee: {response.minimumFee} sats/vByte");
}
```
