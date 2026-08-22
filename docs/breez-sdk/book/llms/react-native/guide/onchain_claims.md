# Claiming on-chain deposits

On-chain deposits go through three stages: once detected, the deposit is visible in the SDK and each deposit includes a `isMature` field; after **3 on-chain confirmations** the deposit has sufficient confirmations (`isMature` is true) and the SDK [automatically attempts](#setting-a-max-fee-for-automatic-claims) to claim it. If the maximum deposit claim fee is too low, the deposit won't be automatically claimed and should be [manually claimed](#manually-claiming-deposits).

## Setting a max fee for automatic claims

The [maximum deposit claim fee](config.md#max-deposit-claim-fee) setting in the SDK configuration defines the maximum fee the SDK uses when automatically claiming an on-chain deposit. The SDK's default fee limit is set to 1 sats/vbyte, which is low and requires manual claiming when fees exceed this threshold. You can set a higher fee, either in sats/vbyte, in absolute sats, or to the fastest recommended fee at the time of claim, with a leeway in sats/vbyte.

To increase the likelihood of automatically claiming deposits, you may set the maximum fee to the fastest recommended rate at the time of claim, which can result in higher fees.

```typescript
// Create the default config
const config = defaultConfig(Network.Mainnet)
config.apiKey = '<breez api key>'

// Set the maximum fee to the fastest network recommended fee at the time of claim
// with a leeway of 1 sats/vbyte
config.maxDepositClaimFee = new MaxFee.NetworkRecommended({ leewaySatPerVbyte: BigInt(1) })
```



However, even when setting a high fee, the SDK might still fail to automatically claim deposits. In these cases, it's recommended to manually claim them by letting the end user accept the required fees. When [manual intervention](#manually-claiming-deposits) is required, the SDK emits an `SdkEvent.UnclaimedDeposits` event containing information about the deposit. See [Listening to events](events.md) for how to subscribe to events.

## Manually claiming deposits

When a deposit cannot be automatically claimed due to the configured maximum fee being too low, you can manually claim it by specifying a higher fee limit. The recommended approach is to display a user interface showing the required fee amount and request user approval before proceeding with manual claiming.

```typescript
if (deposit.claimError?.tag === DepositClaimError_Tags.MaxDepositClaimFeeExceeded) {
  const requiredFee = deposit.claimError.inner.requiredFeeSats

  // Show UI to user with the required fee and get approval
  const userApproved = true // Replace with actual user approval logic

  if (userApproved) {
    const claimRequest: ClaimDepositRequest = {
      txid: deposit.txid,
      vout: deposit.vout,
      maxFee: new MaxFee.Fixed({ amount: requiredFee }),
      maxInstantFeeBps: undefined
    }
    await sdk.claimDeposit(claimRequest)
  }
}
```



### Instant (0-conf) claims

By default a deposit is only claimed once it has enough confirmations. With instant (0-conf) claims the Spark Service Provider fronts the credited amount before confirmation and takes a spread, so the funds become usable immediately.

To claim instantly in the background, set the [maximum instant deposit claim fee](config.md#max-instant-deposit-claim-fee) in the configuration, as basis points of the deposit value. The SDK then attempts a 0-conf claim on each not-yet-mature deposit whose spread is within that ceiling; deposits above it wait for the normal claim at maturity. The spread combines a flat amount and the on-chain fee of the provider's claim with a percentage of the deposit, so it is proportionally larger on small deposits and when on-chain fees are high; those fall through to the normal claim rather than overpaying for speed.

You can also claim a specific not-yet-mature deposit on demand by passing a maximum instant fee, in basis points, to `claimDeposit`. The resulting transfer settles asynchronously, so no payment is returned; watch for it via `listPayments` or the [payment events](events.md).

Because an instant claim settles asynchronously, the deposit remains in `listUnclaimedDeposits` with its `instantClaimStatus` set to `InstantClaimStatus.Submitted` for a short time after it is submitted (a `SdkEvent.ClaimedDeposits` event has already fired). It is removed automatically once the claim settles, so a listed deposit marked `InstantClaimStatus.Submitted` may be an instant claim still in flight rather than one awaiting maturity.

```typescript
// Claim a not-yet-mature deposit instantly (0-conf). Cap it at 4% (400 bps)
// of the deposit value.
const claimRequest: ClaimDepositRequest = {
  txid: deposit.txid,
  vout: deposit.vout,
  maxFee: undefined,
  maxInstantFeeBps: 400
}
await sdk.claimDeposit(claimRequest)
```



## Listing unclaimed deposits

Retrieve all deposits that have not yet been claimed. This includes pending deposits that do not yet have sufficient confirmations, as well as deposits with sufficient confirmations that failed to claim (with the specific failure reason). Pending deposits will be automatically claimed once they have sufficient confirmations.

```typescript
const request: ListUnclaimedDepositsRequest = {}
const response = await sdk.listUnclaimedDeposits(request)

for (const deposit of response.deposits) {
  console.log(`Unclaimed deposit: ${deposit.txid}:${deposit.vout}`)
  console.log(`Amount: ${deposit.amountSats} sats`)

  if (deposit.claimError != null) {
    if (deposit.claimError?.tag === DepositClaimError_Tags.MaxDepositClaimFeeExceeded) {
      let maxFeeStr = 'none'
      if (deposit.claimError.inner.maxFee != null) {
        if (deposit.claimError.inner.maxFee.tag === Fee_Tags.Fixed) {
          maxFeeStr = `${deposit.claimError.inner.maxFee.inner.amount} sats`
        } else if (deposit.claimError.inner.maxFee.tag === Fee_Tags.Rate) {
          maxFeeStr = `${deposit.claimError.inner.maxFee.inner.satPerVbyte} sats/vByte`
        }
      }
      console.log(
        `Max claim fee exceeded. Max: ${maxFeeStr}, 
        Required: ${deposit.claimError.inner.requiredFeeSats} sats 
        or ${deposit.claimError.inner.requiredFeeRateSatPerVbyte} sats/vByte`
      )
    } else if (deposit.claimError?.tag === DepositClaimError_Tags.MissingUtxo) {
      console.log('UTXO not found when claiming deposit')
    } else if (deposit.claimError?.tag === DepositClaimError_Tags.Generic) {
      console.log(`Claim failed: ${deposit.claimError.inner.message}`)
    }
  }
}
```



## Refunding deposits

When a deposit cannot be successfully claimed you can refund it to an external Bitcoin address. This creates a transaction that sends the amount (minus transaction fees) to the specified destination address.

The [recommended fees](#recommended-fees) API is useful for determining appropriate fee levels for refund transactions.

```typescript
const txid = 'your_deposit_txid'
const vout = 0
const destinationAddress = 'bc1qexample...' // Your Bitcoin address

// Set the fee for the refund transaction using the half-hour feerate
const recommendedFees = await sdk.recommendedFees()
const fee = new Fee.Rate({ satPerVbyte: recommendedFees.halfHourFee })
// or using a fixed amount
// const fee = new Fee.Fixed({ amount: BigInt(500) })
//

const request: RefundDepositRequest = {
  txid,
  vout,
  destinationAddress,
  fee
}

const response = await sdk.refundDeposit(request)
console.log('Refund transaction created:')
console.log('Transaction ID:', response.txId)
console.log('Transaction hex:', response.txHex)
```



**Developer note**

The total fee must cover at least 1 sat/vB of the refund transaction so it can be relayed by the Bitcoin network. The exact minimum depends on the size of the transaction, which varies with the destination address type (around 111 sats for a taproot address). If the fee is lower, the refund request is rejected and the error states the required minimum.

## Implementing a custom claim logic

For advanced use cases, you may want to implement a custom claim logic instead of relying on the SDK's automatic process. This gives you complete control over when and how deposits are claimed.

To disable automatic claims, unset the [maximum deposit claim fee](config.md#max-deposit-claim-fee). Then use the methods described above to manually claim deposits based on your business logic.

Common scenarios for custom claiming logic include:

- **Dynamic fee adjustment**: Adjust claiming fees based on market conditions or priority
- **Conditional claiming**: Only claim deposits that meet certain criteria (amount thresholds, time windows, etc.)
- **Integration with external systems**: Coordinate claims with other business processes

The [recommended fees](#recommended-fees) API is useful for determining appropriate fee levels for claiming deposits. For example, you can implement a custom claim logic to only claim deposits if the required fee rate is less than the fastest recommended fee (or any other).

```typescript
if (deposit.claimError?.tag === DepositClaimError_Tags.MaxDepositClaimFeeExceeded) {
  const requiredFeeRate = deposit.claimError.inner.requiredFeeRateSatPerVbyte

  const recommendedFees = await sdk.recommendedFees()

  if (requiredFeeRate <= recommendedFees.fastestFee) {
    const claimRequest: ClaimDepositRequest = {
      txid: deposit.txid,
      vout: deposit.vout,
      maxFee: new MaxFee.Rate({ satPerVbyte: requiredFeeRate }),
      maxInstantFeeBps: undefined
    }
    await sdk.claimDeposit(claimRequest)
  }
}
```



## Recommended fees

Get Bitcoin fee estimates for different confirmation targets to help determine appropriate fee levels for claiming or refunding deposits.

```typescript
const response = await sdk.recommendedFees()
console.log('Fastest fee:', response.fastestFee, 'sats/vByte')
console.log('Half-hour fee:', response.halfHourFee, 'sats/vByte')
console.log('Hour fee:', response.hourFee, 'sats/vByte')
console.log('Economy fee:', response.economyFee, 'sats/vByte')
console.log('Minimum fee:', response.minimumFee, 'sats/vByte')
```
