package com.example.kotlinmpplib

import breez_sdk_spark.*

@OptIn(kotlin.ExperimentalStdlibApi::class)
class UnilateralExit {
    suspend fun quoteExit(sdk: BreezSdk): PrepareUnilateralExitResponse {
        // ANCHOR: prepare-unilateral-exit
        val quote = sdk.prepareUnilateralExit(
            PrepareUnilateralExitRequest(
                feeRateSatPerVbyte = 2u,
                fundingKind = CpfpFundingKind.P2wpkh,
                destination = "bc1q...your-destination-address",
                selection = ExitLeafSelection.Auto
            )
        )

        println("Recovering ${quote.recoverableValueSat} sats for ${quote.totalFeeSat} sats in fees")
        println("Fund a single UTXO of at least ${quote.singleUtxoFundingSat} sats")
        // ANCHOR_END: prepare-unilateral-exit
        return quote
    }

    suspend fun buildExit(sdk: BreezSdk, quote: PrepareUnilateralExitResponse) {
        // ANCHOR: unilateral-exit
        try {
            val secretKeyBytes = "your-secret-key-hex".hexToByteArray()
            val signer = singleKeyCpfpSigner(secretKeyBytes)

            val response = sdk.unilateralExit(
                UnilateralExitRequest(
                    prepared = quote,
                    fundingInputs = listOf(
                        CpfpInput.P2wpkh(
                            txid = "your-utxo-txid",
                            vout = 0u,
                            value = 50_000u,
                            pubkey = "your-compressed-pubkey-hex"
                        )
                    )
                ),
                signer
            )

            for (tx in response.transactions) {
                tx.csvTimelockBlocks?.let { blocks ->
                    println("${tx.txid}: wait $blocks blocks after its parents confirm")
                }
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: unilateral-exit
    }

    // ANCHOR: custom-cpfp-signer
    class MyCpfpSigner : CpfpSigner {
        override suspend fun signPsbt(psbtBytes: ByteArray): ByteArray {
            return signPsbtWithYourKeys(psbtBytes)
        }

        private fun signPsbtWithYourKeys(psbtBytes: ByteArray): ByteArray {
            return psbtBytes
        }
    }
    // ANCHOR_END: custom-cpfp-signer
}
