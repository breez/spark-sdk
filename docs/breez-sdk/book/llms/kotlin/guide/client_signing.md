# Client signing

Client signing lets a server drive payments while the key that approves them stays with the user. The server prepares the payment and builds a small package that describes it, the user reviews and signs the package on their side, and the server publishes it to complete the payment.

Use it when the SDK runs on your server, for example hosting wallets for many users, and the server must not be able to send payments on its own. It works for Spark addresses and invoices, Lightning invoices, token payments, Bitcoin addresses and LNURL payments.

Client signing is fully opt-in. Without it, `sendPayment` works as described in [Sending payments](send_payment.md).

## How it works

1. **Prepare** on the server with `prepareSendPayment`, exactly as in [Sending payments](send_payment.md). This validates the input and returns the fees.
2. **Build** on the server with `buildUnsignedTransferPackage`. This returns the one item the user needs to sign. It carries the amount, fee and destination of the payment.
3. **Sign** on the user's side. The user reviews the package and signs it with their signer.
4. **Publish** on the server with `publishSignedTransferPackage` to complete the payment.

Sometimes the wallet first needs to re-shape its funds so it can send the exact amount (a denomination swap). That swap also needs the user's signature, so it arrives as its own package: publishing it returns `PublishSignedTransferPackageResponse.SwapCompleted`, and you build again from the same prepare response. Repeat until publishing returns `PublishSignedTransferPackageResponse.PaymentSent`.

The server keeps no state between these steps. Everything needed to complete the payment travels inside the requests and responses, so building and publishing can happen in different processes or on different instances. This fits [Server mode](server_mode.md) deployments, where an SDK instance is built per request.

## Signing on the user's side

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/signer/trait.ExternalSparkSigner.html

The user's side does not need a connected SDK, only a signer that holds the user's key: any `ExternalSparkSigner` implementation (see [Using an External Signer](external_signer.md)), whether it runs on the user's device or fronts a remote signing service.

The package tells the user exactly what they are approving: the amount, the fee and the destination. Show these to the user before signing. Sign Transfer and Swap packages with `prepareTransfer`, and Token packages with `prepareTokenTransaction`:

```kotlin
val signature = when (unsigned) {
    is UnsignedTransferPackage.Transfer -> {
        // Show the user what they are approving before signing
        val destination = when (val target = unsigned.target) {
            is TransferTarget.Spark -> target.address
            is TransferTarget.Lightning -> target.bolt11
            is TransferTarget.CoopExit -> target.address
        }
        // Log.v("Breez", "Approve sending ${unsigned.amountSat} sats " +
        //     "(fee ${unsigned.feeSat} sats) to $destination")
        TransferSignature.Transfer(
            signer.prepareTransfer(unsigned.prepareTransfer)
        )
    }
    is UnsignedTransferPackage.Swap -> {
        // Log.v("Breez", "Approve re-shaping funds for a ${unsigned.amountSat} " +
        //     "sat send (fee ${unsigned.feeSat} sats)")
        TransferSignature.Transfer(
            signer.prepareTransfer(unsigned.prepareTransfer)
        )
    }
    is UnsignedTransferPackage.Token -> {
        if (unsigned.isSwap) {
            // Log.v("Breez", "Approve combining token outputs for a " +
            //     "${unsigned.tokenIdentifier} send")
        } else {
            // Log.v("Breez", "Approve sending ${unsigned.amount} of token " +
            //     "${unsigned.tokenIdentifier} (fee ${unsigned.fee})")
        }
        TransferSignature.Token(
            signer.prepareTokenTransaction(unsigned.prepareTokenTransaction)
        )
    }
    is UnsignedTransferPackage.TokenBatch -> {
        if (unsigned.isSwap) {
            // Log.v("Breez", "Approve combining token outputs before the batch is sent")
        } else {
            for (total in unsigned.totals) {
                // Log.v("Breez", "Approve sending ${total.amount} of token " +
                //     "${total.tokenIdentifier}")
            }
        }
        TransferSignature.Token(
            signer.prepareTokenTransaction(unsigned.prepareTokenTransaction)
        )
    }
}

val signedPackage = SignedTransferPackage(unsigned, signature)
```



## Driving the send from the server

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.build_unsigned_transfer_package

Prepare once, then repeat build, sign and publish until the payment is sent:

```kotlin
val prepareResponse = sdk.prepareSendPayment(
    PrepareSendPaymentRequest(
        paymentRequest = PaymentRequest.Input(input = "<spark address or invoice>"),
        amount = BigInteger.fromLong(5_000L),
        tokenIdentifier = null,
        conversionOptions = null,
        feePolicy = null,
    )
)

while (true) {
    val unsigned = sdk.buildUnsignedTransferPackage(
        BuildUnsignedTransferPackageRequest(
            prepareResponse = prepareResponse,
            options = null,
        )
    )

    // Send the package to the user, who reviews and signs it
    val signedPackage = signPackage(signer, unsigned)

    val result = sdk.publishSignedTransferPackage(
        PublishSignedTransferPackageRequest(signedPackage)
    )
    when (result) {
        // The wallet's funds were re-shaped first: build the payment again
        is PublishSignedTransferPackageResponse.SwapCompleted -> continue
        is PublishSignedTransferPackageResponse.PaymentSent -> return result.payment
        // Only a batch package pays several recipients at once
        is PublishSignedTransferPackageResponse.PaymentsSent ->
            throw IllegalStateException("unexpected batch response for a single payment")
    }
}
```



### Bitcoin

For Bitcoin addresses, choose the confirmation speed when building the package. The fee, and therefore what the user signs, depends on it:

```kotlin
// For Bitcoin address sends, the confirmation speed is chosen when
// building the package: the fee depends on it
val unsigned = sdk.buildUnsignedTransferPackage(
    BuildUnsignedTransferPackageRequest(
        prepareResponse = prepareResponse,
        options = BuildTransferPackageOptions.BitcoinAddress(
            confirmationSpeed = OnchainConfirmationSpeed.MEDIUM
        ),
    )
)
```



### Lightning

For BOLT11 invoices the build options work like the send options in [Sending payments](send_payment.md#lightning-1): `preferSpark` sends via a direct Spark transfer when the invoice also contains a Spark address, and `completionTimeoutSecs` controls how long publishing waits for the payment to complete before returning it while still pending:

```kotlin
val unsigned = sdk.buildUnsignedTransferPackage(
    BuildUnsignedTransferPackageRequest(
        prepareResponse = prepareResponse,
        options = BuildTransferPackageOptions.Bolt11Invoice(
            preferSpark = true,
            completionTimeoutSecs = 10u,
        ),
    )
)
```



### Tokens

Token payments follow the same loop. Prepare with a token identifier as in [Token payments](token_payments.md). The package amounts are in the token's base units, and the user signs with `prepareTokenTransaction`. A Token package with `isSwap` set means the wallet first needs to combine token outputs: publishing it returns `PublishSignedTransferPackageResponse.SwapCompleted`, just like the Bitcoin case.

## LNURL-Pay

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.build_unsigned_lnurl_pay_package

LNURL payments have their own pair of methods, because completing them includes the LNURL exchange with the recipient's service. Prepare with `prepareLnurlPay` as in [LNURL-Pay](lnurl_pay.md), then run the same loop with `buildUnsignedLnurlPayPackage` and `publishSignedLnurlPayPackage`. The result carries the LNURL response, including any success action:

```kotlin
while (true) {
    val unsigned = sdk.buildUnsignedLnurlPayPackage(
        BuildUnsignedLnurlPayPackageRequest(prepareResponse)
    )

    val signedPackage = signPackage(signer, unsigned)

    val result = sdk.publishSignedLnurlPayPackage(
        PublishSignedLnurlPayPackageRequest(signedPackage)
    )
    when (result) {
        is PublishSignedLnurlPayResponse.SwapCompleted -> continue
        is PublishSignedLnurlPayResponse.PaymentSent -> return result.response
    }
}
```



## Failures and retries

- Publishing the same signed package twice returns the same result, so it is safe to retry after a lost response or a network error.
- If publishing fails because the wallet's funds moved or fees changed since the package was built, prepare again and restart the loop with a fresh package.
- Never reuse a signature for a changed payment. Any change to the amount, fee or destination needs a new package, reviewed and signed by the user.

## Remote signers

The signature does not have to come from a device holding the mnemonic. Any `ExternalSparkSigner` implementation can sign the package, including one backed by a remote signing service. With Turnkey, a policy can require the end user to approve the transfer signing while the server runs the rest; see [Using Turnkey](turnkey.md#user-approved-payments).

## Limitations

- Payments with a conversion step (see [Converting tokens](token_conversion.md)) are not supported.
- USDC/USDT cross-chain sends are not supported.
