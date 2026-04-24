package com.example.kotlinmpplib

import breez_sdk_spark.*

@OptIn(kotlin.ExperimentalStdlibApi::class)
class UnilateralExit {
    suspend fun listLeavesForExit(sdk: BreezSdk) {
        // ANCHOR: list-leaves
        try {
            val response = sdk.listLeaves(
                ListLeavesRequest(minValueSats = 10_000u)
            )

            for (leaf in response.leaves) {
                println("Leaf ${leaf.id}: ${leaf.value} sats")
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: list-leaves
    }

    suspend fun prepareExit(sdk: BreezSdk) {
        // ANCHOR: prepare-unilateral-exit
        try {
            val leafIds = listOf("leaf-id-1", "leaf-id-2")
            val secretKeyBytes = "your-secret-key-hex".hexToByteArray()
            val signer = SingleKeySigner(secretKeyBytes)

            val response = sdk.prepareUnilateralExit(
                PrepareUnilateralExitRequest(
                    feeRate = 2u,
                    leafIds = leafIds,
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

            for (leaf in response.leaves) {
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
