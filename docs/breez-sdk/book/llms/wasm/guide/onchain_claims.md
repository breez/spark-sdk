# Claiming on-chain deposits

On-chain deposits go through three stages. Once detected, the deposit is visible in the SDK and each deposit includes a `isMature` field. After **3 on-chain confirmations** the deposit has sufficient confirmations (`isMature` is true) and the SDK [automatically attempts](#setting-a-max-fee-for-automatic-claims) to claim it. The SDK also claims automatically [before maturity](#claiming-before-maturity) when the configured ceiling covers the provider's spread, so a deposit can be credited sooner than 3 confirmations. If the maximum deposit claim fee is too low for either, the deposit won't be automatically claimed and should be [manually claimed](#manually-claiming-deposits).

## Setting a max fee for automatic claims

The [maximum deposit claim fee](config.md#max-deposit-claim-fee) setting in the SDK configuration defines the maximum fee the SDK uses when automatically claiming an on-chain deposit. The SDK's default fee limit is set to 1 sats/vbyte, which is low and requires manual claiming when fees exceed this threshold. You can set a higher fee, either in sats/vbyte, in absolute sats, or to the fastest recommended fee at the time of claim, with a leeway in sats/vbyte.

This ceiling is not only an on-chain fee tolerance. It also caps what the provider may take to credit a deposit [before it matures](#claiming-before-maturity), so the value you choose decides both how much on-chain fee the SDK will pay and whether deposits are claimed early at all.

To increase the likelihood of automatically claiming deposits, you may set the maximum fee to the fastest recommended rate at the time of claim, which can result in higher fees.

```typescript
// Create the default config
const config = defaultConfig('mainnet')
config.apiKey = '<breez api key>'

// Set the maximum fee to the fastest network recommended fee at the time of claim
// with a leeway of 1 sats/vbyte
config.maxDepositClaimFee = { type: 'networkRecommended', leewaySatPerVbyte: 1 }
```



However, even when setting a high fee, the SDK might still fail to automatically claim deposits. In these cases, it's recommended to manually claim them by letting the end user accept the required fees. When [manual intervention](#manually-claiming-deposits) is required, the SDK emits an `SdkEvent.UnclaimedDeposits` event containing information about the deposit. See [Listening to events](events.md) for how to subscribe to events.

## Claiming before maturity

A deposit does not have to wait for maturity. The Spark Service Provider will front the credited amount earlier and take a spread for carrying the risk, and the SDK claims this way automatically whenever the spread fits within the [maximum deposit claim fee](config.md#max-deposit-claim-fee). The default of 1 sat/vbyte works out to about 99 sats, below any spread the provider charges, so deposits are claimed at maturity until the ceiling is raised enough to cover one. The same applies to `claimDeposit`, which claims a not-yet-mature deposit early when its own `maxFee` allows.

The spread is largely the on-chain cost of the provider's claim plus a percentage of the deposit, so it grows with the deposit.

## Manually claiming deposits

When a deposit cannot be automatically claimed due to the configured maximum fee being too low, you can manually claim it by specifying a higher fee limit. The recommended approach is to display a user interface showing the required fee amount and request user approval before proceeding with manual claiming.

Claiming a deposit the SDK is already claiming, whether from a background attempt or another call, returns `SdkError.DepositClaimInProgress`. The claim already running may still succeed, so treat this as transient rather than as a failure to show the user.

```typescript
if (deposit.claimError?.type === 'maxDepositClaimFeeExceeded') {
  const requiredFee = deposit.claimError.requiredFeeSats

  // Show UI to user with the required fee and get approval
  const userApproved = true // Replace with actual user approval logic

  if (userApproved) {
    const claimRequest: ClaimDepositRequest = {
      txid: deposit.txid,
      vout: deposit.vout,
      maxFee: { type: 'fixed', amount: requiredFee }
    }
    await sdk.claimDeposit(claimRequest)
  }
}
```



### Showing the choice to the user

`fetchClaimDepositQuote` prices both ways of claiming a deposit, so an app can offer the choice rather than deciding for the user. It returns the deposit's current `confirmations` alongside a quote for claiming early and one for claiming at maturity, each carrying the fee and the `confirmationsRequired`, which is the depth it becomes claimable at rather than a count of blocks still to wait. Subtract the deposit's current confirmations for that: an early claim claimable at 1 confirmation, on a deposit with 0, is available a block from now.

The early quote is absent when the provider will not front this particular deposit, and when claiming early would not actually be earlier: once a deposit has matured, or when the provider would only credit at maturity's own depth, waiting is both cheaper and no slower, so there is no choice left to offer. The early quote is priced whether or not the configured [maximum deposit claim fee](config.md#max-deposit-claim-fee) would allow it, since that ceiling is usually far below a spread and the point of the quote is to let the user decide. Acting on it therefore means passing a `maxFee` to `claimDeposit` of at least the quoted `feeSats`; with a lower one the call returns `SdkError.MaxDepositClaimFeeExceeded` and the deposit waits for maturity instead. The quote for maturity is always present, but may be flagged `isEstimate` when the provider will not quote a deposit this early, in which case the fee is derived from current on-chain fees and the final one may differ.

A claim made before maturity settles asynchronously, so `claimDeposit` returns no payment. Watch for it via `listPayments` or the [payment events](events.md).

```typescript
const quote = await sdk.fetchClaimDepositQuote({
  txid: deposit.txid,
  vout: deposit.vout
})

// Claiming once the deposit matures, and how many blocks that is away.
const blocksToWait = Math.max(
  0,
  quote.mature.confirmationsRequired - quote.confirmations
)
console.log(`Wait ${blocksToWait} blocks and pay ${quote.mature.feeSats} sats`)

// Claiming earlier, when the provider offers it.
if (quote.instant != null) {
  const instantBlocksToWait = Math.max(
    0,
    quote.instant.confirmationsRequired - quote.confirmations
  )
  console.log(
    `Or wait ${instantBlocksToWait} blocks and ` +
    `pay ${quote.instant.feeSats} sats`
  )
}
```



## Listing unclaimed deposits

Retrieve all deposits that have not yet been claimed. This includes pending deposits that do not yet have sufficient confirmations, as well as deposits with sufficient confirmations that failed to claim (with the specific failure reason). Pending deposits will be automatically claimed once they have sufficient confirmations, or sooner if the configured ceiling covers an early claim.

A deposit claimed before maturity stays in the list with its `instantClaimStatus` set to `InstantClaimStatus.Submitted` for a short time after submission, and is removed once the claim settles. When the SDK claims automatically it emits `SdkEvent.ClaimedDeposits` at submission, so a deposit can briefly appear both in that event and in this list.

```typescript
const request: ListUnclaimedDepositsRequest = {}
const response = await sdk.listUnclaimedDeposits(request)

for (const deposit of response.deposits) {
  console.log(`Unclaimed deposit: ${deposit.txid}:${deposit.vout}`)
  console.log(`Amount: ${deposit.amountSats} sats`)

  if (deposit.claimError != null) {
    switch (deposit.claimError.type) {
      case 'maxDepositClaimFeeExceeded': {
        let maxFeeStr = 'none'
        if (deposit.claimError.maxFee != null) {
          if (deposit.claimError.maxFee.type === 'fixed') {
            maxFeeStr = `${deposit.claimError.maxFee.amount} sats`
          } else if (deposit.claimError.maxFee.type === 'rate') {
            maxFeeStr = `${deposit.claimError.maxFee.satPerVbyte} sats/vByte`
          }
        }
        console.log(
          `Max claim fee exceeded. Max: ${maxFeeStr}, ` +
          `Required: ${deposit.claimError.requiredFeeSats} sats or ` +
          `${deposit.claimError.requiredFeeRateSatPerVbyte} sats/vByte`
        )
        break
      }
      case 'missingUtxo':
        console.log('UTXO not found when claiming deposit')
        break
      case 'generic':
        console.log(`Claim failed: ${deposit.claimError.message}`)
        break
    }
  }
}
```



## Refunding deposits

When a deposit cannot be successfully claimed you can refund it to an external Bitcoin address. This creates a transaction that sends the amount (minus transaction fees) to the specified destination address.

The [recommended fees](#recommended-fees) API is useful for determining appropriate fee levels for refund transactions.

A deposit can only be refunded once it has enough confirmations. Calling `refundDeposit` earlier fails, reporting the deposit as unknown while it is unconfirmed and as having too few confirmations for a block or so after that. Nothing is signed or stored when this happens, so retry after a few more blocks.

```typescript
const txid = 'your_deposit_txid'
const vout = 0
const destinationAddress = 'bc1qexample...' // Your Bitcoin address

// Set the fee for the refund transaction using the half-hour feerate
const recommendedFees = await sdk.recommendedFees()
const fee: Fee = { type: 'rate', satPerVbyte: recommendedFees.halfHourFee }
// or using a fixed amount
// const fee: Fee = { type: 'fixed', amount: 500 }
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

The total fee must cover at least 1 sat/vB of the refund transaction so it can be relayed by the Bitcoin network. The exact minimum depends on the size of the transaction, which varies with the destination address type: around 99 sats to a native segwit address and 111 sats to a taproot one. If the fee is lower, the refund request is rejected and the error states the required minimum.

### Tracking a refund

`refundState` on `DepositInfo` reports how far the refund has got:

- **`RefundState.BroadcastPending`**: the refund is signed and stored but has not been seen on the network. The SDK rebroadcasts it on every sync until the deposit is spent, so a refund that failed to send because of a temporary network problem recovers on its own.
- **`RefundState.Broadcast`**: the network has accepted the refund and it is waiting to confirm. The deposit disappears from `listUnclaimedDeposits` once it does.

A refund created near the 1 sat/vB minimum can stay at `RefundState.BroadcastPending` indefinitely if the network's minimum relay fee later rises above what it pays. Rebroadcasting cannot fix this, because the network keeps refusing the same transaction. Read `lastError` for the reason the network gave, then call `refundDeposit` again at a higher fee to replace it.

Replacing a refund that is already on the network costs more than the original fee, because the replacement also pays to relay its own size. When the fee offered is too low, the call is rejected and the error states the minimum required.

## Implementing a custom claim logic

For advanced use cases, you may want to implement a custom claim logic instead of relying on the SDK's automatic process. This gives you complete control over when and how deposits are claimed.

To disable automatic claims, unset the [maximum deposit claim fee](config.md#max-deposit-claim-fee). Then use the methods described above to manually claim deposits based on your business logic.

Common scenarios for custom claiming logic include:

- **Dynamic fee adjustment**: Adjust claiming fees based on market conditions or priority
- **Conditional claiming**: Only claim deposits that meet certain criteria (amount thresholds, time windows, etc.)
- **Integration with external systems**: Coordinate claims with other business processes

The [recommended fees](#recommended-fees) API is useful for determining appropriate fee levels for claiming deposits. For example, you can implement a custom claim logic to only claim deposits if the required fee rate is less than the fastest recommended fee (or any other).

```typescript
if (deposit.claimError?.type === 'maxDepositClaimFeeExceeded') {
  const requiredFeeRate = deposit.claimError.requiredFeeRateSatPerVbyte

  const recommendedFees = await sdk.recommendedFees()

  if (requiredFeeRate <= recommendedFees.fastestFee) {
    const claimRequest: ClaimDepositRequest = {
      txid: deposit.txid,
      vout: deposit.vout,
      maxFee: { type: 'rate', satPerVbyte: requiredFeeRate }
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
