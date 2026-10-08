# Handling errors

When a `BreezSdk` method fails, it returns an `SdkError` (`SdkException` in Kotlin and C#). Each kind of failure is its own variant, so you can match the ones your app handles and fall back to the error's message for the rest. Some variants carry fields with the detail you need to recover, such as the fee a deposit claim requires. Some variants also carry a {{#name docs_url}} field linking to their section below.

## Catching errors

{{#tabs errors:handle-errors}}

How errors appear differs by language:

- **JavaScript (WASM)**: errors are thrown as standard `Error` objects, typed as `SdkError`. The message describes the error, and errors that link to the guide also have a `docsUrl` property. There are no per-variant error types to match on.
- **React Native**: each variant is a class under `SdkError`. Match it with `instanceOf` and read its fields from `inner`.
- **Other languages**: each variant is its own error type or case, with its fields available directly.

Deposits the SDK claims in the background don't throw. When one of those claims fails, the reason is recorded on the deposit's {{#name claim_error}} instead. It uses the same [MaxDepositClaimFeeExceeded](#max-deposit-claim-fee-exceeded), [MissingUtxo](#missing-utxo) and [DepositTooSmall](#deposit-too-small) cases described below, and any other failure is recorded as a generic message. See [Handling claim outcomes](./onchain_claims.md#handling-claim-outcomes).

## Error reference

<h3 id="spark-error">
    <a class="header" href="#spark-error">SparkError</a>
</h3>

An operation on the Spark network failed, such as a transfer or a call to the Spark operators. The message carries the underlying cause.

<h3 id="insufficient-funds">
    <a class="header" href="#insufficient-funds">InsufficientFunds</a>
</h3>

The balance can't cover the payment and its fees. When the shortfall is in a token, {{#name token_identifier}} names the token. It is unset when the shortfall is in sats or no single token can be named. You can check balances with [{{#name get_info}}](./get_info.md).

<h3 id="invalid-uuid">
    <a class="header" href="#invalid-uuid">InvalidUuid</a>
</h3>

An identifier passed to the SDK is not a valid UUID.

<h3 id="invalid-input">
    <a class="header" href="#invalid-input">InvalidInput</a>
</h3>

The request was rejected as invalid: a malformed address or invoice, a missing or out-of-range value, an option that doesn't apply to the request, or a lookup for something that doesn't exist. The message names the problem. Fix the request rather than retrying it unchanged.

<h3 id="cross-chain-amount-out-of-range">
    <a class="header" href="#cross-chain-amount-out-of-range">CrossChainAmountOutOfRange</a>
</h3>

The USDC/USDT provider rejected the amount for the route. {{#name too_small}} tells you whether it fell below the minimum or above the maximum, and the published bound is included when the provider publishes one. See [Amount limits](./cross_chain.md#amount-limits).

<h3 id="cross-chain-route-unavailable">
    <a class="header" href="#cross-chain-route-unavailable">CrossChainRouteUnavailable</a>
</h3>

The USDC/USDT provider won't serve the route. When {{#name temporary}} is set, the provider expects the route back shortly and the same request may succeed later. Otherwise pick another route. See [Amount limits](./cross_chain.md#amount-limits).

<h3 id="cross-chain-disabled">
    <a class="header" href="#cross-chain-disabled">CrossChainDisabled</a>
</h3>

USDC/USDT payments are not enabled on this SDK instance, so every cross-chain call fails, including {{#name get_cross_chain_routes}}. Enable them by setting {{#name cross_chain_config}}, as described in [USDC/USDT configuration](./config.md#usdc-usdt).

<h3 id="network-error">
    <a class="header" href="#network-error">NetworkError</a>
</h3>

A request to a remote service failed, for example an LNURL server, the Lightning address service, a fiat rate provider or a USDC/USDT provider. The message carries the service's response.

<h3 id="storage-error">
    <a class="header" href="#storage-error">StorageError</a>
</h3>

Reading or writing the SDK's local storage failed.

<h3 id="chain-service-error">
    <a class="header" href="#chain-service-error">ChainServiceError</a>
</h3>

The Bitcoin chain service failed, for example while fetching a deposit transaction or the recommended fees. Calls that read the chain, such as {{#name claim_deposit}}, {{#name refund_deposit}}, {{#name recommended_fees}} and unilateral exits, can return it.

<h3 id="max-deposit-claim-fee-exceeded">
    <a class="header" href="#max-deposit-claim-fee-exceeded">MaxDepositClaimFeeExceeded</a>
</h3>

Claiming a deposit costs more than the maximum fee allows, or no maximum fee is set. {{#name required_fee_sats}} and {{#name required_fee_rate_sat_per_vbyte}} give the fee the claim needs, so you can ask the user to approve it and claim again with a higher maximum. See [Handling claim outcomes](./onchain_claims.md#handling-claim-outcomes).

<h3 id="missing-utxo">
    <a class="header" href="#missing-utxo">MissingUtxo</a>
</h3>

The deposit transaction has no output at the given index, so the transaction id or output index doesn't match a real deposit.

<h3 id="deposit-too-small">
    <a class="header" href="#deposit-too-small">DepositTooSmall</a>
</h3>

After the claim fee, the deposit would credit less than the dust limit. No maximum fee makes it claimable, but a drop in on-chain fees can. See [Handling claim outcomes](./onchain_claims.md#handling-claim-outcomes).

<h3 id="deposit-claim-in-progress">
    <a class="header" href="#deposit-claim-in-progress">DepositClaimInProgress</a>
</h3>

Another claim on this deposit is already running, or an early claim was already made. It is not a failure to show the user, and it needs nothing from you. See [Claiming a deposit manually](./onchain_claims.md#claiming-a-deposit-manually).

<h3 id="refund-replacement-fee-too-low">
    <a class="header" href="#refund-replacement-fee-too-low">RefundReplacementFeeTooLow</a>
</h3>

A refund for this deposit is already on the network, and the new one doesn't pay enough to replace it. Retry with a fee of at least {{#name required_fee_sats}}. See [Tracking a refund](./onchain_claims.md#tracking-a-refund).

<h3 id="lnurl-error">
    <a class="header" href="#lnurl-error">LnurlError</a>
</h3>

The LNURL service returned an error while paying or withdrawing. The message carries the service's reason. See [LNURL-Pay](./lnurl_pay.md) and [LNURL-Withdraw](./lnurl_withdraw.md).

<h3 id="signer">
    <a class="header" href="#signer">Signer</a>
</h3>

The signer failed, for example while signing or deriving a key. With an external signer, the message carries the error your signer returned. See [Using an External Signer](./external_signer.md#implementing-a-custom-signer).

<h3 id="optimization-already-running">
    <a class="header" href="#optimization-already-running">OptimizationAlreadyRunning</a>
</h3>

{{#name optimize_leaves}} was called while another optimization, automatic or manual, was still running. See [Controlling optimization timing](./optimize.md#controlling-optimization-timing).

<h3 id="optimization-cancelled">
    <a class="header" href="#optimization-cancelled">OptimizationCancelled</a>
</h3>

The SDK stopped the optimization to free funds for a higher-priority operation, usually a payment. It is not a fault. See [Controlling optimization timing](./optimize.md#controlling-optimization-timing).

<h3 id="insufficient-cpfp-funds">
    <a class="header" href="#insufficient-cpfp-funds">InsufficientCpfpFunds</a>
</h3>

The funding provided for a unilateral exit can't cover its on-chain fees. {{#name required_sat}} is the minimum needed. See [Troubleshooting](./unilateral_exit.md#troubleshooting).

<h3 id="generic">
    <a class="header" href="#generic">Generic</a>
</h3>

Any other failure. The message describes what went wrong.
