# Handling errors

When a `BreezSdk` method fails, it returns an `SdkError` (`SdkException` in Kotlin and C#). Each kind of failure is its own variant, so you can match the ones your app handles and fall back to the error's message for the rest. Some variants carry fields with the detail you need to recover, such as the fee a deposit claim requires. Some variants also carry a `docsUrl` field linking to their section below.

## Catching errors

```kotlin
try {
    val prepareResponse = sdk.prepareSendPayment(request)
    // Log.v("Breez", "Payment prepared: ${prepareResponse.paymentMethod}")
} catch (e: SdkException.InsufficientFunds) {
    // Log.v("Breez", "Not enough funds for this payment")
} catch (e: SdkException.CrossChainDisabled) {
    // Log.v("Breez", "Cross-chain payments are not enabled, see ${e.docsUrl}")
} catch (e: SdkException) {
    // Log.v("Breez", "Failed to prepare the payment: ${e.message}")
}
```



How errors appear differs by language:

- **JavaScript (WASM)**: errors are thrown as standard `Error` objects, typed as `SdkError`. The message describes the error, and errors that link to the guide also have a `docsUrl` property. There are no per-variant error types to match on.
- **React Native**: each variant is a class under `SdkError`. Match it with `instanceOf` and read its fields from `inner`.
- **Other languages**: each variant is its own error type or case, with its fields available directly.

Deposits the SDK claims in the background don't throw. When one of those claims fails, the reason is recorded on the deposit's `claimError` instead. It uses the same [MaxDepositClaimFeeExceeded](#max-deposit-claim-fee-exceeded), [MissingUtxo](#missing-utxo) and [DepositTooSmall](#deposit-too-small) cases described below, and any other failure is recorded as a generic message. See [Handling the outcome](./onchain_manual_claims.md#handling-the-outcome).

## Error reference

### SparkError

An operation on the Spark network failed, such as a transfer or a call to the Spark operators. The message carries the underlying cause.

### InsufficientFunds

The balance can't cover the payment and its fees. `tokenIdentifier` is set when there are insufficient funds to make a token payment. You can check balances with [`getInfo`](./get_info.md).

### InvalidUuid

An identifier passed to the SDK is not a valid UUID.

### InvalidInput

The request was rejected as invalid: a malformed address or invoice, a missing or out-of-range value, an option that doesn't apply to the request, or a lookup for something that doesn't exist. The message names the problem. Fix the request rather than retrying it unchanged.

### CrossChainAmountOutOfRange

The USDC/USDT provider rejected the amount for the route. `tooSmall` tells you whether it fell below the minimum or above the maximum, and the published bound is included when the provider publishes one. See [Amount limits](./cross_chain.md#amount-limits).

### CrossChainRouteUnavailable

The USDC/USDT provider won't serve the route. When `temporary` is set, the provider expects the route back shortly and the same request may succeed later. Otherwise pick another route. See [Amount limits](./cross_chain.md#amount-limits).

### CrossChainDisabled

USDC/USDT payments are not enabled on this SDK instance, so every cross-chain call fails, including `getCrossChainRoutes`. Enable them by setting `crossChainConfig`, as described in [USDC/USDT configuration](./config.md#usdc-usdt).

### NetworkError

A request to a remote service failed, for example an LNURL server, the Lightning address service, a fiat rate provider or a USDC/USDT provider. The message carries the service's response.

### StorageError

Reading or writing the SDK's local storage failed.

### ChainServiceError

The Bitcoin chain service failed, for example while fetching a deposit transaction or the recommended fees. Calls that read the chain, such as `claimDeposit`, `refundDeposit`, `recommendedFees` and unilateral exits, can return it.

### MaxDepositClaimFeeExceeded

Claiming a deposit costs more than the maximum fee allows, or no maximum fee is set. `requiredFeeSats` gives the fee the claim needs, so you can ask the user to approve it and claim again with a higher maximum. See [Handling the outcome](./onchain_manual_claims.md#handling-the-outcome).

### MissingUtxo

The deposit transaction has no output at the given index, so the transaction id or output index doesn't match a real deposit.

### DepositTooSmall

After the claim fee, the deposit would credit less than the dust limit. No maximum fee makes it claimable, but a drop in on-chain fees can. See [Handling the outcome](./onchain_manual_claims.md#handling-the-outcome).

### DepositClaimInProgress

Another claim on this deposit is already running, or an early claim was already made. It is not a failure to show the user, and it needs nothing from you. See [Claiming a deposit manually](./onchain_manual_claims.md#claiming).

### RefundReplacementFeeTooLow

A refund for this deposit is already on the network, and the new one doesn't pay enough to replace it. Retry with a fee of at least `requiredFeeSats`. See [Tracking a refund](./onchain_refunds.md#tracking-a-refund).

### LnurlError

The LNURL service returned an error while paying or withdrawing. The message carries the service's reason. See [LNURL-Pay](./lnurl_pay.md) and [LNURL-Withdraw](./lnurl_withdraw.md).

### Signer

The signer failed, for example while signing or deriving a key. With an external signer, the message carries the error your signer returned. See [Using an External Signer](./external_signer.md#implementing-a-custom-signer).

### OptimizationAlreadyRunning

`optimizeLeaves` was called while another optimization, automatic or manual, was still running. See [Controlling optimization timing](./optimize.md#controlling-optimization-timing).

### OptimizationCancelled

The SDK stopped the optimization to free funds for a higher-priority operation, usually a payment. It is not a fault. See [Controlling optimization timing](./optimize.md#controlling-optimization-timing).

### InsufficientCpfpFunds

The funding provided for a unilateral exit can't cover its on-chain fees. `requiredSats` is the minimum needed. See [Troubleshooting](./recover_funds.md#troubleshooting).

### TokenAllowance

A token allowance request was refused. `reason` says why, for example the amount is over a limit, the allowance was revoked or has expired, or a receiver isn't on its recipient list, and `details` explains it further. See [Token allowances](./token_allowances.md#errors).

### Generic

Any other failure. The message describes what went wrong.
