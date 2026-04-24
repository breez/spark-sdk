package com.example.kotlinmpplib

import breez_sdk_spark.*

@OptIn(kotlin.ExperimentalStdlibApi::class)
class UnilateralExit {
    suspend fun prepareExit(sdk: BreezSdk) {
        // ANCHOR: prepare-unilateral-exit
        try {
            val secretKeyBytes = "your-secret-key-hex".hexToByteArray()
            val signer = SingleKeySigner(secretKeyBytes)

            val response = sdk.prepareUnilateralExit(
                PrepareUnilateralExitRequest(
                    feeRate = 2u,
                    inputs = listOf(
                        UnilateralExitCpfpInput.P2wpkh(
                            txid = "your-utxo-txid",
                            vout = 0u,
                            value = 50_000u,
                            pubkey = "your-compressed-pubkey-hex"
                        )
                    ),
                    destination = "bc1q...your-destination-address"
                ),
                signer
            )

            // The SDK automatically selects which leaves are profitable to exit.
            for (leaf in response.selectedLeaves) {
                println("Leaf ${leaf.id}: ${leaf.value} sats (exit cost: ~${leaf.estimatedCost} sats)")
            }

            for (leaf in response.transactions) {
                for (pair in leaf.txCpfpPairs) {
                    pair.csvTimelockBlocks?.let { blocks ->
                        println("Timelock: wait $blocks blocks")
                    }
                    // pair.parentTxHex: pre-signed Spark transaction
                    // pair.childTxHex: signed CPFP transaction — broadcast alongside parent
                }
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: prepare-unilateral-exit
    }
}
