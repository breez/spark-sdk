# Claiming on-chain deposits

On-chain deposits go through three stages. Once detected, the deposit is visible in the SDK and each deposit includes a `IsMature` field. After **3 on-chain confirmations** the deposit has sufficient confirmations (`IsMature` is true) and the SDK [automatically attempts](#setting-a-max-fee-for-automatic-claims) to claim it. The SDK also claims automatically [before maturity](#claiming-before-maturity) when the configured ceiling covers the provider's spread, so a deposit can be credited sooner than 3 confirmations. If the maximum deposit claim fee is too low for either, the deposit won't be automatically claimed and should be [manually claimed](#manually-claiming-deposits).

## Setting a max fee for automatic claims

The [maximum deposit claim fee](config.md#max-deposit-claim-fee) setting in the SDK configuration defines the maximum fee the SDK uses when automatically claiming an on-chain deposit. The SDK's default fee limit is set to 1 sats/vbyte, which is low and requires manual claiming when fees exceed this threshold. You can set a higher fee, either in sats/vbyte, in absolute sats, or to the fastest recommended fee at the time of claim, with a leeway in sats/vbyte.

This ceiling is not only an on-chain fee tolerance. It also caps what the provider may take to credit a deposit [before it matures](#claiming-before-maturity), so the value you choose decides both how much on-chain fee the SDK will pay and whether deposits are claimed early at all.

To increase the likelihood of automatically claiming deposits, you may set the maximum fee to the fastest recommended rate at the time of claim, which can result in higher fees.

```go
// Create the default config
config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
apiKey := "<breez api key>"
config.ApiKey = &apiKey

// Set the maximum fee to the fastest network recommended fee at the time of claim
// with a leeway of 1 sats/vbyte
networkRecommendedInterface := breez_sdk_spark.MaxFee(
	breez_sdk_spark.MaxFeeNetworkRecommended{LeewaySatPerVbyte: 1},
)
config.MaxDepositClaimFee = &networkRecommendedInterface
```



However, even when setting a high fee, the SDK might still fail to automatically claim deposits. In these cases, it's recommended to manually claim them by letting the end user accept the required fees. When [manual intervention](#manually-claiming-deposits) is required, the SDK emits an `SdkEventUnclaimedDeposits` event containing information about the deposit. See [Listening to events](events.md) for how to subscribe to events.

## Claiming before maturity

A deposit does not have to wait for maturity. The Spark Service Provider will front the credited amount earlier and take a spread for carrying the risk, and the SDK claims this way automatically whenever the spread fits within the [maximum deposit claim fee](config.md#max-deposit-claim-fee). The default of 1 sat/vbyte works out to about 99 sats, below any spread the provider charges, so deposits are claimed at maturity until the ceiling is raised enough to cover one. The same applies to `ClaimDeposit`, which claims a not-yet-mature deposit early when its own `MaxFee` allows.

The spread is largely the on-chain cost of the provider's claim plus a percentage of the deposit, so it grows with the deposit.

## Manually claiming deposits

When a deposit cannot be automatically claimed due to the configured maximum fee being too low, you can manually claim it by specifying a higher fee limit. The recommended approach is to display a user interface showing the required fee amount and request user approval before proceeding with manual claiming.

Claiming a deposit the SDK is already claiming, whether from a background attempt or another call, returns `SdkErrorDepositClaimInProgress`. The claim already running may still succeed, so treat this as transient rather than as a failure to show the user.

```go
if claimErr := *deposit.ClaimError; claimErr != nil {
	if exceeded, ok := claimErr.(breez_sdk_spark.DepositClaimErrorMaxDepositClaimFeeExceeded); ok {
		requiredFee := exceeded.RequiredFeeSats

		// Show UI to user with the required fee and get approval
		userApproved := true // Replace with actual user approval logic

		if userApproved {
			maxFee := breez_sdk_spark.MaxFee(breez_sdk_spark.MaxFeeFixed{Amount: requiredFee})
			claimRequest := breez_sdk_spark.ClaimDepositRequest{
				Txid:   deposit.Txid,
				Vout:   deposit.Vout,
				MaxFee: &maxFee,
			}
			_, err := sdk.ClaimDeposit(claimRequest)
			if err != nil {
				var sdkErr *breez_sdk_spark.SdkError
				if errors.As(err, &sdkErr) {
					// Handle SdkError - can inspect specific variants if needed
					// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
				}
				return err
			}
		}
	}
}
```



### Showing the choice to the user

`FetchClaimDepositQuote` prices both ways of claiming a deposit, so an app can offer the choice rather than deciding for the user. It returns the deposit's current `Confirmations` alongside a quote for claiming early and one for claiming at maturity, each carrying the fee and the `ConfirmationsRequired`, which is the depth it becomes claimable at rather than a count of blocks still to wait. Subtract the deposit's current confirmations for that: an early claim claimable at 1 confirmation, on a deposit with 0, is available a block from now.

The early quote is absent when the provider will not front this particular deposit, and when claiming early would not actually be earlier: once a deposit has matured, or when the provider would only credit at maturity's own depth, waiting is both cheaper and no slower, so there is no choice left to offer. The early quote is priced whether or not the configured [maximum deposit claim fee](config.md#max-deposit-claim-fee) would allow it, since that ceiling is usually far below a spread and the point of the quote is to let the user decide. Acting on it therefore means passing a `MaxFee` to `ClaimDeposit` of at least the quoted `FeeSats`; with a lower one the call returns `SdkErrorMaxDepositClaimFeeExceeded` and the deposit waits for maturity instead. The quote for maturity is always present, but may be flagged `IsEstimate` when the provider will not quote a deposit this early, in which case the fee is derived from current on-chain fees and the final one may differ.

A claim made before maturity settles asynchronously, so `ClaimDeposit` returns no payment. Watch for it via `ListPayments` or the [payment events](events.md).

```go
quote, err := sdk.FetchClaimDepositQuote(breez_sdk_spark.FetchClaimDepositQuoteRequest{
	Txid: deposit.Txid,
	Vout: deposit.Vout,
})
if err != nil {
	return err
}

// Claiming once the deposit matures, and how many blocks that is away.
blocksToWait := uint32(0)
if quote.Mature.ConfirmationsRequired > quote.Confirmations {
	blocksToWait = quote.Mature.ConfirmationsRequired - quote.Confirmations
}
log.Printf("Wait %v blocks and pay %v sats", blocksToWait, quote.Mature.FeeSats)

// Claiming earlier, when the provider offers it.
if quote.Instant != nil {
	instantBlocks := uint32(0)
	if quote.Instant.ConfirmationsRequired > quote.Confirmations {
		instantBlocks = quote.Instant.ConfirmationsRequired - quote.Confirmations
	}
	log.Printf("Or wait %v blocks and pay %v sats", instantBlocks, quote.Instant.FeeSats)
}
```



## Listing unclaimed deposits

Retrieve all deposits that have not yet been claimed. This includes pending deposits that do not yet have sufficient confirmations, as well as deposits with sufficient confirmations that failed to claim (with the specific failure reason). Pending deposits will be automatically claimed once they have sufficient confirmations, or sooner if the configured ceiling covers an early claim.

A deposit claimed before maturity stays in the list with its `InstantClaimStatus` set to `InstantClaimStatusSubmitted` for a short time after submission, and is removed once the claim settles. When the SDK claims automatically it emits `SdkEventClaimedDeposits` at submission, so a deposit can briefly appear both in that event and in this list.

```go
request := breez_sdk_spark.ListUnclaimedDepositsRequest{}
response, err := sdk.ListUnclaimedDeposits(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}

for _, deposit := range response.Deposits {
	log.Printf("Unclaimed Deposit: %v:%v", deposit.Txid, deposit.Vout)
	log.Printf("Amount: %v sats", deposit.AmountSats)

	if claimErr := *deposit.ClaimError; claimErr != nil {
		switch claimErr := claimErr.(type) {
		case breez_sdk_spark.DepositClaimErrorMaxDepositClaimFeeExceeded:
			maxFeeStr := "none"
			if claimErr.MaxFee != nil {
				switch fee := (*claimErr.MaxFee).(type) {
				case breez_sdk_spark.FeeFixed:
					maxFeeStr = fmt.Sprintf("%v sats", fee.Amount)
				case breez_sdk_spark.FeeRate:
					maxFeeStr = fmt.Sprintf("%v sats/vByte", fee.SatPerVbyte)
				}
			}
			log.Printf(
				"Max claim fee exceeded. Max: %v, Required: %v sats or %v sats/vByte",
				maxFeeStr,
				claimErr.RequiredFeeSats,
				claimErr.RequiredFeeRateSatPerVbyte,
			)
		case breez_sdk_spark.DepositClaimErrorMissingUtxo:
			log.Print("UTXO not found when claiming deposit")
		case breez_sdk_spark.DepositClaimErrorGeneric:
			log.Printf("Claim failed: %v", claimErr.Message)
		}
	}
}
```



## Refunding deposits

When a deposit cannot be successfully claimed you can refund it to an external Bitcoin address. This creates a transaction that sends the amount (minus transaction fees) to the specified destination address.

The [recommended fees](#recommended-fees) API is useful for determining appropriate fee levels for refund transactions.

A deposit can only be refunded once it has enough confirmations. Calling `RefundDeposit` earlier fails, reporting the deposit as unknown while it is unconfirmed and as having too few confirmations for a block or so after that. Nothing is signed or stored when this happens, so retry after a few more blocks.

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



**Developer note**

The total fee must cover at least 1 sat/vB of the refund transaction so it can be relayed by the Bitcoin network. The exact minimum depends on the size of the transaction, which varies with the destination address type: around 99 sats to a native segwit address and 111 sats to a taproot one. If the fee is lower, the refund request is rejected and the error states the required minimum.

### Tracking a refund

`RefundState` on `DepositInfo` reports how far the refund has got:

- **`RefundStateBroadcastPending`**: the refund is signed and stored but has not been seen on the network. The SDK rebroadcasts it on every sync until the deposit is spent, so a refund that failed to send because of a temporary network problem recovers on its own.
- **`RefundStateBroadcast`**: the network has accepted the refund and it is waiting to confirm. The deposit disappears from `ListUnclaimedDeposits` once it does.

A refund created near the 1 sat/vB minimum can stay at `RefundStateBroadcastPending` indefinitely if the network's minimum relay fee later rises above what it pays. Rebroadcasting cannot fix this, because the network keeps refusing the same transaction. Read `LastError` for the reason the network gave, then call `RefundDeposit` again at a higher fee to replace it.

Replacing a refund that is already on the network costs more than the original fee, because the replacement also pays to relay its own size. When the fee offered is too low, the call is rejected and the error states the minimum required.

## Implementing a custom claim logic

For advanced use cases, you may want to implement a custom claim logic instead of relying on the SDK's automatic process. This gives you complete control over when and how deposits are claimed.

To disable automatic claims, unset the [maximum deposit claim fee](config.md#max-deposit-claim-fee). Then use the methods described above to manually claim deposits based on your business logic.

Common scenarios for custom claiming logic include:

- **Dynamic fee adjustment**: Adjust claiming fees based on market conditions or priority
- **Conditional claiming**: Only claim deposits that meet certain criteria (amount thresholds, time windows, etc.)
- **Integration with external systems**: Coordinate claims with other business processes

The [recommended fees](#recommended-fees) API is useful for determining appropriate fee levels for claiming deposits. For example, you can implement a custom claim logic to only claim deposits if the required fee rate is less than the fastest recommended fee (or any other).

```go
if claimErr := *deposit.ClaimError; claimErr != nil {
	if exceeded, ok := claimErr.(breez_sdk_spark.DepositClaimErrorMaxDepositClaimFeeExceeded); ok {
		requiredFeeRate := exceeded.RequiredFeeRateSatPerVbyte

		recommendedFees, err := sdk.RecommendedFees()
		if err != nil {
			return err
		}

		if requiredFeeRate <= recommendedFees.FastestFee {
			maxFee := breez_sdk_spark.MaxFee(breez_sdk_spark.MaxFeeRate{SatPerVbyte: requiredFeeRate})
			claimRequest := breez_sdk_spark.ClaimDepositRequest{
				Txid:   deposit.Txid,
				Vout:   deposit.Vout,
				MaxFee: &maxFee,
			}
			_, err := sdk.ClaimDeposit(claimRequest)
			if err != nil {
				var sdkErr *breez_sdk_spark.SdkError
				if errors.As(err, &sdkErr) {
					// Handle SdkError - can inspect specific variants if needed
					// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
				}
				return err
			}
		}
	}
}
```



## Recommended fees

Get Bitcoin fee estimates for different confirmation targets to help determine appropriate fee levels for claiming or refunding deposits.

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
