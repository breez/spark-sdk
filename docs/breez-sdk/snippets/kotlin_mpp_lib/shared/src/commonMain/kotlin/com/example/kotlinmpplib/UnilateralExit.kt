package com.example.kotlinmpplib

import breez_sdk_spark.*

@OptIn(kotlin.ExperimentalStdlibApi::class)
class UnilateralExit {
    suspend fun prepareExit(sdk: BreezSdk) {
        // ANCHOR: prepare-unilateral-exit
        try {
            val secretKeyBytes = "your-secret-key-hex".hexToByteArray()
            val signer = singleKeyCpfpSigner(secretKeyBytes)

            val response = sdk.prepareUnilateralExit(
                PrepareUnilateralExitRequest(
                    feeRateSatPerVbyte = 2u,
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
            for (leaf in response.leaves) {
                println("Leaf ${leaf.leafId}: ${leaf.value} sats (exit cost: ~${leaf.estimatedCost} sats)")
                for (tx in leaf.transactions) {
                    tx.csvTimelockBlocks?.let { blocks ->
                        println("Timelock: wait $blocks blocks")
                    }
                    // tx.txHex: pre-signed Spark transaction
                    // tx.cpfpTxHex: signed CPFP transaction — broadcast alongside parent
                }
            }

            if (response.unverifiedNodeIds.isNotEmpty()) {
                println("Warning: could not verify confirmation status for ${response.unverifiedNodeIds.size} nodes")
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: prepare-unilateral-exit
    }
}
