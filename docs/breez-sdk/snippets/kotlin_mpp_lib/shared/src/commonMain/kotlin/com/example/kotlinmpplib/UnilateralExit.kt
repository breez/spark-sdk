package com.example.kotlinmpplib

import breez_sdk_spark.*

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

            val response = sdk.prepareUnilateralExit(
                PrepareUnilateralExitRequest(
                    feeRate = 2u,
                    leafIds = leafIds,
                    utxos = listOf(
                        UnilateralExitCpfpUtxo(
                            txid = "your-utxo-txid",
                            vout = 0u,
                            value = 50_000u,
                            pubkey = "your-compressed-pubkey-hex",
                            utxoType = UnilateralExitCpfpUtxoType.P2WPKH
                        )
                    ),
                    destination = "bc1q...your-destination-address"
                )
            )

            // The response contains:
            // - response.leaves: transaction/PSBT pairs to sign and broadcast
            // - response.sweepTxHex: signed sweep transaction for the final step
            for (leaf in response.leaves) {
                for (pair in leaf.txCpfpPsbts) {
                    pair.csvTimelockBlocks?.let { blocks ->
                        println("Timelock: wait $blocks blocks")
                    }
                    // pair.parentTxHex: pre-signed Spark transaction
                    // pair.childPsbtHex: unsigned CPFP PSBT — sign with your UTXO key
                }
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: prepare-unilateral-exit
    }
}
