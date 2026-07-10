package com.example.kotlinmpplib

import breez_sdk_spark.*
import com.ionspin.kotlin.bignum.integer.BigInteger

class ClientSigning {
    suspend fun signPackage(
        signer: ExternalSparkSigner,
        unsigned: UnsignedTransferPackage,
    ): SignedTransferPackage {
        // ANCHOR: client-signing-sign-package
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
        }

        val signedPackage = SignedTransferPackage(unsigned, signature)
        // ANCHOR_END: client-signing-sign-package
        return signedPackage
    }

    suspend fun sendWithClientSigning(sdk: BreezSdk, signer: ExternalSparkSigner): Payment {
        // ANCHOR: client-signing-send
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
            }
        }
        // ANCHOR_END: client-signing-send
    }

    suspend fun buildOnchainPackage(
        sdk: BreezSdk,
        prepareResponse: PrepareSendPaymentResponse,
    ) {
        // ANCHOR: client-signing-build-onchain-options
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
        // ANCHOR_END: client-signing-build-onchain-options
        // Log.v("Breez", "Unsigned package: $unsigned")
    }

    suspend fun buildBolt11Package(
        sdk: BreezSdk,
        prepareResponse: PrepareSendPaymentResponse,
    ) {
        // ANCHOR: client-signing-build-bolt11-options
        val unsigned = sdk.buildUnsignedTransferPackage(
            BuildUnsignedTransferPackageRequest(
                prepareResponse = prepareResponse,
                options = BuildTransferPackageOptions.Bolt11Invoice(
                    preferSpark = true,
                    completionTimeoutSecs = 10u,
                ),
            )
        )
        // ANCHOR_END: client-signing-build-bolt11-options
        // Log.v("Breez", "Unsigned package: $unsigned")
    }

    suspend fun lnurlPayWithClientSigning(
        sdk: BreezSdk,
        signer: ExternalSparkSigner,
        prepareResponse: PrepareLnurlPayResponse,
    ): LnurlPayResponse {
        // ANCHOR: client-signing-lnurl-pay
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
        // ANCHOR_END: client-signing-lnurl-pay
    }
}
