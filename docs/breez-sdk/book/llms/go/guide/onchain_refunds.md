# Refunding deposits

A deposit that cannot or should not be claimed can be refunded to an external Bitcoin address. The refund sends the deposit amount, minus the transaction fee, to that address.

```go
txid := "<your_deposit_txid>"
vout := uint32(0)
destinationAddress := "bc1qexample..." // Your Bitcoin address

// Set the fee for the refund transaction using the half-hour feerate
recommendedFees, err := sdk.RecommendedFees()
if err != nil {
	return err
}
fee := breez_sdk_spark.Fee(breez_sdk_spark.FeeRate{SatPerVbyte: recommendedFees.HalfHourFee})
// or using a fixed amount
//fee := breez_sdk_spark.Fee(breez_sdk_spark.FeeFixed{Amount: 500})
//

request := breez_sdk_spark.RefundDepositRequest{
	Txid:               txid,
	Vout:               vout,
	DestinationAddress: destinationAddress,
	Fee:                fee,
}
response, err := sdk.RefundDeposit(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}

log.Print("Refund transaction created:")
log.Printf("Transaction ID: %v", response.TxId)
log.Printf("Transaction hex: %v", response.TxHex)
```



- A claimed deposit cannot be refunded. Check that its `InstantClaimStatus` is not `InstantClaimStatusClaimed` before offering a refund.
- A deposit can only be refunded once it has enough confirmations. Before that, `RefundDeposit` fails and nothing is signed or stored, so retry after a few more blocks.
- The fee must cover at least 1 sat/vB of the refund transaction: around 99 sats to a native segwit address and 111 sats to a taproot one. A lower fee is rejected, and the error states the minimum.

[Recommended fees](#recommended-fees) help choose the fee.

## Tracking a refund

`RefundState` on the deposit reports the refund's progress:

- `RefundStateBroadcastPending`: signed and stored, but not yet seen on the network. The SDK rebroadcasts it on every sync, so a temporary network problem resolves on its own.
- `RefundStateBroadcast`: accepted by the network and waiting to confirm. The deposit leaves `ListUnclaimedDeposits` once it confirms.

A refund paying close to the minimum can stay at `RefundStateBroadcastPending` if the network's minimum relay fee rises above what it pays, and rebroadcasting cannot fix that. Read `LastError` for the network's reason, then call `RefundDeposit` again with a higher fee to replace it. The replacement has to pay more than the original, and a fee too low to replace it is rejected with the minimum in the error.

## Recommended fees

`RecommendedFees` returns fee rate estimates for different confirmation targets.

```go
response, err := sdk.RecommendedFees()
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}
log.Printf("Fastest fee: %v sats/vByte", response.FastestFee)
log.Printf("Half-hour fee: %v sats/vByte", response.HalfHourFee)
log.Printf("Hour fee: %v sats/vByte", response.HourFee)
log.Printf("Economy fee: %v sats/vByte", response.EconomyFee)
log.Printf("Minimum fee: %v sats/vByte", response.MinimumFee)
```
