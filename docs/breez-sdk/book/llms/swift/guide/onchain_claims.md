# Claiming on-chain deposits

On-chain funds do not have to wait for 3 confirmations. With instant and expedited claims, a deposit can reach the wallet's balance while it is still in the mempool, or a block or two after it confirms. The SDK detects a deposit as soon as it is in the mempool, follows it through its confirmations, and claims it on its own as soon as the cost of doing so fits the fee limits you set.

A claim comes in three speeds. For the two faster ones the Spark Service Provider fronts the funds and charges an instant claim fee for doing so. The standard claim costs an ordinary on-chain fee.

| Claim | When funds arrive | Cost | Claimed automatically when |
|---|---|---|---|
| Instant | At 0 confirmations, while the deposit is still in the mempool. Offered for some deposits only. | Instant claim fee | The instant claim fee fits the deposit's max fee |
| Expedited | At 1 or 2 confirmations | Instant claim fee | The instant claim fee fits the deposit's max fee |
| Standard | At 3 confirmations on mainnet (1 on regtest) | On-chain fee only | The on-chain fee fits the deposit's max fee |

A deposit's max fee is the configured [max claim fee](#setting-a-max-claim-fee), unless the deposit has been given [a max fee of its own](#giving-one-deposit-its-own-max-fee). Its own max fee then governs the instant and expedited claims, and the standard claim runs under the larger of the two.

What each speed would cost for a particular deposit can be checked before claiming it, by [fetching a fee quote](#pricing-a-claim-with-fee-quotes).

If the deposit's max fee is too low for any of the three, the deposit is not claimed automatically and should be [claimed manually](#claiming-a-deposit-manually).

**Developer note**

The SDK attempts an instant or expedited claim whenever the provider offers one, and takes it when its fee fits the max claim fee. The default max claim fee of 1 sat/vbyte (about 99 sats) is below any instant claim fee, so with the default setting deposits are credited by the standard claim. Raise it to have deposits credited early.

## Setting a max claim fee

The [max claim fee](config.md#max-deposit-claim-fee) in the SDK configuration is the most the SDK will pay when it claims a deposit on its own. It takes the form of an absolute amount in sats, a rate in sats/vbyte, or the fastest recommended fee with a leeway, as described on the [configuration page](config.md#max-deposit-claim-fee).

The max claim fee is one dial for two things. It caps the on-chain fee of a standard claim, and it caps the instant claim fee the provider may take for an [instant or expedited claim](#instant-expedited-claims). The value you choose therefore decides both how much on-chain fee the SDK will pay and whether deposits are claimed early at all.

To make automatic claims more likely, set the max claim fee to the fastest recommended rate at the time of the claim. This can result in higher fees.

```swift
// Create the default config
var config = defaultConfig(network: Network.mainnet)
config.apiKey = "<breez api key>"

// Set the maximum fee to the fastest network recommended fee at the time of claim
// with a leeway of 1 sats/vbyte
config.maxDepositClaimFee = MaxFee.networkRecommended(leewaySatPerVbyte: 1)
```



Even with a high max claim fee the SDK might still fail to claim a deposit on its own. When that happens it emits `SdkEvent.unclaimedDeposits` with the deposit's details, and the recommended approach is to [claim it manually](#claiming-a-deposit-manually) once the user has accepted the required fee. See [Listening to events](events.md) for how to subscribe.

### Giving one deposit its own max fee

A `maxFee` passed to `claimDeposit` is recorded on that deposit and carried into the SDK's later automatic attempts on it. That is how one deposit is treated differently from the rest without changing the configuration for all of them.

- Raising it above the instant claim fee lets that single deposit be claimed early. The SDK keeps applying it on later sync passes, so the app does not have to keep calling `claimDeposit` until the claim lands.
- Lowering it below the instant claim fee holds that one deposit back from an instant or expedited claim. It waits for the standard claim while the others keep claiming early under the configured max claim fee.

The standard claim runs under whichever is larger, the deposit's own max fee or the configured one. A raised max fee therefore applies to the standard claim too, should the early claim never happen, so raise it to what you are willing to pay for the deposit, not only for the early claim. A lowered one restricts only the early claim.

A few more rules to keep in mind:

- Calling `claimDeposit` without a `maxFee` claims under the configured max claim fee and clears any max fee standing on the deposit.
- The max fee is recorded before the claim is attempted, so it stands whatever the attempt reports. What the attempt reports is covered under [Handling claim outcomes](#handling-claim-outcomes).
- The recorded value is readable as `maxClaimFee` on each deposit from `listUnclaimedDeposits`, unset while the configured max claim fee applies.
- Deposits are not part of the synced wallet records, so a max fee set on one device stays on that device. The same deposit seen from another device runs under whatever that device has configured.

## Seeing deposits before they confirm

A deposit is visible in the SDK from the moment it reaches the mempool, so an app can show it to the user, or claim it, before the first confirmation. The Spark operators only report a deposit once it has a confirmation, so the SDK also asks its chain service about the deposit addresses it has handed out. A deposit found this way arrives through `SdkEvent.newDeposits` and appears in `listUnclaimedDeposits` with `isMature` false, whether or not the SDK goes on to claim it automatically.

Each watched address costs one chain-service request per sync. Requesting a receive address starts a 24-hour window on it, and requesting it again restarts that window, so a wallet that is not expecting an on-chain payment settles at no requests at all. An address that has taken a deposit keeps being watched past its window until that deposit confirms.

## Listing unclaimed deposits

`listUnclaimedDeposits` returns every deposit the SDK is tracking:

- Pending deposits that have not yet reached the standard claim depth (`isMature` is false). These are claimed automatically once they reach the standard claim depth, or sooner if the max claim fee covers an instant or expedited claim.
- Deposits with enough confirmations whose claim failed, with the specific failure reason.
- Deposits already claimed whose output the provider has not yet spent.

```swift
let request = ListUnclaimedDepositsRequest()
let response = try await sdk.listUnclaimedDeposits(request: request)

for deposit in response.deposits {
    print("Unclaimed deposit: \(deposit.txid):\(deposit.vout)")
    print("Amount: \(deposit.amountSats) sats")

    if let claimError = deposit.claimError {
        switch claimError {
        case .maxDepositClaimFeeExceeded(
            let tx, let vout, let maxFee, let requiredFeeSats, let requiredFeeRateSatPerVbyte):
            let maxFeeStr: String
            if let maxFee = maxFee {
                switch maxFee {
                case .fixed(let amount):
                    maxFeeStr = "\(amount) sats"
                case .rate(let satPerVbyte):
                    maxFeeStr = "\(satPerVbyte) sats/vByte"
                }
            } else {
                maxFeeStr = "none"
            }
            print(
                "Max claim fee exceeded. Max: \(maxFeeStr), "
                    + "Required: \(requiredFeeSats) sats or "
                    + "\(requiredFeeRateSatPerVbyte) sats/vByte"
            )
        case .missingUtxo(let tx, let vout):
            print("UTXO not found when claiming deposit")
        case .generic(let message):
            print("Claim failed: \(message)")
        }
    }
}
```



### Deposits that are already claimed

A deposit taken by an instant or expedited claim carries `InstantClaimStatus.submitted` in its `instantClaimStatus` while the claim settles, and `InstantClaimStatus.claimed` once the amount is credited. It stays in the list until the provider spends the deposit output, some time after the credit. Treat `InstantClaimStatus.claimed` as settled and branch on it rather than showing the deposit as awaiting action. When the SDK claims automatically it emits `SdkEvent.claimedDeposits` at submission, so a deposit can appear both in that event and in this list.

A deposit claimed elsewhere, by another instance sharing the wallet or on another device, reaches `InstantClaimStatus.claimed` the next time the SDK tries to claim it and the provider reports it as already claimed. No `SdkEvent.claimedDeposits` event is emitted, because the claim was not made here. The credit still arrives as a payment, so follow it through `listPayments` or the [payment events](events.md).

## Instant & expedited claims {#instant-expedited-claims}

A deposit does not have to wait for 3 confirmations. The Spark Service Provider fronts the credited amount as soon as it is willing to carry the risk, which brings a deposit to the balance while it is still in the mempool (an instant claim) or after 1 or 2 confirmations (an expedited claim). Both kinds are reported through the `instant` quote and `instantClaimStatus`.

What it costs, and when it is offered:

- The provider charges the instant claim fee for fronting the funds. It is roughly the on-chain cost of the provider's own claim plus a percentage of the deposit, so it grows with the deposit.
- A claim at 0 confirmations is offered for some deposits only. Whether a deposit is fronted at all, and at which depth, is decided by the provider per deposit. It is not a setting you control.
- The SDK re-attempts the early claim as confirmations arrive, so a deposit the provider will not yet front in the mempool is often claimed expedited a block or two later.

The SDK claims early on its own whenever the provider offers an early claim and its fee fits the [max claim fee](#setting-a-max-claim-fee), so the max claim fee decides whether deposits are credited early. To credit a single deposit early instead, call `claimDeposit` with a higher `maxFee`, as described under [Giving one deposit its own max fee](#giving-one-deposit-its-own-max-fee).

## Claiming a deposit manually

When a deposit is not claimed automatically because the max claim fee is too low, claim it with `claimDeposit` and a higher `maxFee`. The recommended approach is to show the user the required fee and ask for approval before claiming.

Claiming a deposit that already has a claim returns `SdkError.depositClaimInProgress`. That covers a claim still running, from a background attempt or another call, and one that has already credited the deposit and is waiting for the provider to spend the output. Neither is a failure to show the user, and neither needs anything from you. Check `instantClaimStatus` to tell them apart.

```swift
if case .maxDepositClaimFeeExceeded(_, _, _, let requiredFeeSats, _) = deposit.claimError {
    // Show UI to user with the required fee and get approval
    let userApproved = true  // Replace with actual user approval logic

    if userApproved {
        let claimRequest = ClaimDepositRequest(
            txid: deposit.txid,
            vout: deposit.vout,
            maxFee: MaxFee.fixed(amount: requiredFeeSats)
        )
        try await sdk.claimDeposit(request: claimRequest)
    }
}
```



### Writing your own claim logic

For advanced use cases you may want to write your own claim logic instead of relying on the SDK's automatic process. This gives you complete control over when and how deposits are claimed.

To disable automatic claims, unset the [max claim fee](config.md#max-deposit-claim-fee). Then use the methods on this page to claim deposits manually according to your business logic. Common scenarios include:

- **Dynamic fee adjustment**: Adjust claiming fees based on market conditions or priority
- **Conditional claiming**: Only claim deposits that meet certain criteria (amount thresholds, time windows, etc.)
- **Integration with external systems**: Coordinate claims with other business processes

The [recommended fees](#recommended-fees) API is useful for determining appropriate fee levels for claiming deposits. For example, you can claim a deposit only if the required fee rate is less than the fastest recommended fee (or any other).

```swift
if case .maxDepositClaimFeeExceeded(_, _, _, _, let requiredFeeRateSatPerVbyte) =
    deposit.claimError
{
    let recommendedFees = try await sdk.recommendedFees()

    if requiredFeeRateSatPerVbyte <= recommendedFees.fastestFee {
        let claimRequest = ClaimDepositRequest(
            txid: deposit.txid,
            vout: deposit.vout,
            maxFee: MaxFee.rate(satPerVbyte: requiredFeeRateSatPerVbyte)
        )
        try await sdk.claimDeposit(request: claimRequest)
    }
}
```



## Pricing a claim with fee quotes

`fetchClaimDepositQuote` prices both ways of claiming a deposit, so an app can offer the choice rather than deciding for the user. It returns the deposit's current `confirmations` alongside two quotes: `instant` for the instant or expedited claim, and `mature` for the standard claim. Each quote carries the fee and `confirmationsRequired`.

Reading the quotes:

- `confirmationsRequired` is the depth the deposit becomes claimable at, not a count of blocks still to wait. Subtract the deposit's current confirmations to get the wait: an early claim claimable at 1 confirmation, on a deposit with 0, is available a block from now.
- On the `instant` quote, `confirmationsRequired` also tells instant from expedited: 0 is an instant claim, 1 or 2 is an expedited one.
- The `instant` quote is absent when the provider will not front this particular deposit. It is also absent when claiming early would not actually be earlier: once the deposit has reached the standard claim depth, or when the provider would only credit at that same depth, waiting is both cheaper and no slower, so there is no choice left to offer. It is absent, too, when the provider could not be reached for a quote, which is the one case worth retrying. An absent quote means no early claim is offered for this deposit right now, not that early claiming is unavailable.
- The `instant` quote is priced whether or not the configured max claim fee would allow it, so the fee it shows is the provider's price rather than what the configured limit permits.
- The `mature` quote is always present, but may be flagged `isEstimate` when the provider will not quote a deposit this early. The fee is then derived from current on-chain fees and the final one may differ.

Acting on the `instant` quote yourself means passing a `maxFee` to `claimDeposit` of at least the quoted `feeSats`. With a lower one the call returns `ClaimDepositOutcome.deferred` with `ClaimDeferredReason.maxFeeExceeded`, carrying what the early claim would have cost, and the deposit waits for the standard claim unless the max fee is raised (see [Handling claim outcomes](#handling-claim-outcomes)).

What to offer follows from the quote and the configured max claim fee. Check the middle column first: where the SDK claims by itself, a dialog defaulting to the standard claim shows the user one outcome and delivers another.

| Quote state | What the SDK will do | What to present |
|---|---|---|
| No `instant` quote | Standard claim | The standard claim only |
| `instant` quote, depth not yet reached | Claim early once the depth arrives, if the fee fits the max claim fee | The standard claim, with the early claim shown as available in N blocks |
| `instant` quote at a reachable depth, fee above the max claim fee | Wait for the standard claim | Both, the early claim requiring an explicit higher max fee |
| `instant` quote at a reachable depth, fee within the max claim fee | Claim early by itself | Default to the early claim, or offer no choice |

```swift
let request = FetchClaimDepositQuoteRequest(txid: deposit.txid, vout: deposit.vout)
let quote = try await sdk.fetchClaimDepositQuote(request: request)

// The standard claim, and how many blocks away it is.
var blocksToWait: UInt32 = 0
if quote.mature.confirmationsRequired > quote.confirmations {
    blocksToWait = quote.mature.confirmationsRequired - quote.confirmations
}
print("Wait \(blocksToWait) blocks and pay \(quote.mature.feeSats) sats")

// An instant or expedited claim, when the provider offers one.
if let instant = quote.instant {
    var instantBlocks: UInt32 = 0
    if instant.confirmationsRequired > quote.confirmations {
        instantBlocks = instant.confirmationsRequired - quote.confirmations
    }
    print("Or wait \(instantBlocks) blocks and pay \(instant.feeSats) sats")
}
```



## Handling claim outcomes

`claimDeposit` reports what it did as `outcome`, which is worth handling in full:

- `ClaimDepositOutcome.settled`: a standard claim that settled, carrying the payment it produced.
- `ClaimDepositOutcome.submitted`: an instant or expedited claim is settling asynchronously. Watch for the payment via `listPayments` or the [payment events](events.md).
- `ClaimDepositOutcome.deferred`: nothing was claimed yet, and no further call is needed.

Which outcome occurs follows from the deposit's depth and the max fee rather than from anything you ask for. A `maxFee` below what an early claim costs returns `ClaimDepositOutcome.deferred` rather than failing. A deposit that has already reached the standard claim depth and whose claim exceeds the max fee is a different matter and returns `SdkError.maxDepositClaimFeeExceeded`, because nothing will claim it until the max fee rises or on-chain fees fall.

Whether a deferred deposit actually waits for the standard claim depends on its `reason`. The SDK re-attempts an early claim as the deposit gains confirmations, so a claim declined at a depth the provider will not yet front is often claimed early a block or two later.

- `ClaimDeferredReason.noEarlyClaimAvailable` usually clears with the next confirmation.
- `ClaimDeferredReason.maxFeeExceeded` does not clear on its own. The deposit waits for the standard claim unless the max fee is raised.
- `ClaimDeferredReason.providerDeclined` means the provider refused or could not be reached, which another confirmation does not address.

Only the first is a wait you can put a time on, so showing a user "claimed in about 30 minutes" for the others would be wrong.

## Refunding deposits

When a deposit cannot be claimed you can refund it to an external Bitcoin address. This creates a transaction that sends the amount, minus transaction fees, to the destination address.

A deposit that has already been claimed is not a candidate: its `instantClaimStatus` is `InstantClaimStatus.claimed`, so check that before offering a refund.

The [recommended fees](#recommended-fees) API is useful for choosing a fee for the refund transaction.

A deposit can only be refunded once it has enough confirmations. Calling `refundDeposit` earlier fails, reporting the deposit as unknown while it is unconfirmed and as having too few confirmations for a block or so after that. Nothing is signed or stored when this happens, so retry after a few more blocks.

```swift
let txid = "your_deposit_txid"
let vout: UInt32 = 0
let destinationAddress = "bc1qexample..."  // Your Bitcoin address

// Set the fee for the refund transaction using the half-hour feerate
let recommendedFees = try await sdk.recommendedFees()
let fee = Fee.rate(satPerVbyte: recommendedFees.halfHourFee)
// or using a fixed amount
//let fee = Fee.fixed(amount: 500) // 500 sats
//

let request = RefundDepositRequest(
    txid: txid,
    vout: vout,
    destinationAddress: destinationAddress,
    fee: fee
)

let response = try await sdk.refundDeposit(request: request)
print("Refund transaction created:")
print("Transaction ID: \(response.txId)")
print("Transaction hex: \(response.txHex)")
```



**Developer note**

The total fee must cover at least 1 sat/vB of the refund transaction so it can be relayed by the Bitcoin network. The exact minimum depends on the size of the transaction, which varies with the destination address type: around 99 sats to a native segwit address and 111 sats to a taproot one. If the fee is lower, the refund request is rejected and the error states the required minimum.

### Tracking a refund

`refundState` on `DepositInfo` reports how far the refund has got:

- **`RefundState.broadcastPending`**: the refund is signed and stored but has not been seen on the network. The SDK rebroadcasts it on every sync until the deposit is spent, so a refund that failed to send because of a temporary network problem recovers on its own.
- **`RefundState.broadcast`**: the network has accepted the refund and it is waiting to confirm. The deposit disappears from `listUnclaimedDeposits` once it does.

A refund created near the 1 sat/vB minimum can stay at `RefundState.broadcastPending` indefinitely if the network's minimum relay fee later rises above what it pays. Rebroadcasting cannot fix this, because the network keeps refusing the same transaction. Read `lastError` for the reason the network gave, then call `refundDeposit` again at a higher fee to replace it.

Replacing a refund that is already on the network costs more than the original fee, because the replacement also pays to relay its own size. When the fee offered is too low, the call is rejected and the error states the minimum required.

## Recommended fees

Get Bitcoin fee estimates for different confirmation targets to help determine appropriate fee levels for claiming or refunding deposits.

```swift
let response = try await sdk.recommendedFees()
print("Fastest fee: \(response.fastestFee) sats/vByte")
print("Half-hour fee: \(response.halfHourFee) sats/vByte")
print("Hour fee: \(response.hourFee) sats/vByte")
print("Economy fee: \(response.economyFee) sats/vByte")
print("Minimum fee: \(response.minimumFee) sats/vByte")
```
