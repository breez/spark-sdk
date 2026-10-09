package com.example.kotlinmpplib

import breez_sdk_spark.*

@OptIn(kotlin.ExperimentalStdlibApi::class)
class RecoverFunds {
    suspend fun fetchRecoverableFunds(sdk: BreezSdk) {
        // ANCHOR: recoverable-funds
        val info = sdk.getInfo(GetInfoRequest(ensureSynced = false))

        if (info.recoverableFundsSats > 0u) {
            // Log.v("Breez", "${info.recoverableFundsSats} sats can be recovered on-chain")
        }
        // ANCHOR_END: recoverable-funds
    }

    suspend fun prepareRecovery(sdk: BreezSdk): PrepareRecoverFundsResponse {
        // ANCHOR: prepare-recover-funds
        val quote = sdk.prepareRecoverFunds(
            PrepareRecoverFundsRequest(
                feeRateSatPerVbyte = 2u,
                fundingKind = CpfpFundingKind.P2wpkh,
                destination = "bc1q...your-destination-address",
                selection = ExitLeafSelection.RecoverableOnly
            )
        )

        if (quote.leaves.isEmpty()) {
            // Log.v("Breez", "Nothing to recover")
            return quote
        }
        for (leaf in quote.leaves) {
            // Log.v("Breez", "${leaf.leafId}: ${leaf.valueSats} sats, ${leaf.method}")
        }
        // Log.v(
        //     "Breez",
        //     "Recovering ${quote.recoverableValueSats} sats " +
        //         "for ${quote.totalFeeSats} sats in fees"
        // )
        val funding = quote.funding
        if (funding != null) {
            // Log.v("Breez", "Fund one UTXO of at least ${funding.singleUtxoSats} sats")
        }
        // ANCHOR_END: prepare-recover-funds
        return quote
    }

    suspend fun recoverCooperatively(sdk: BreezSdk, quote: PrepareRecoverFundsResponse) {
        // ANCHOR: recover-cooperatively
        // A quote with funding holds a unilateral exit: prepare the
        // cooperative leaves alone to recover them without it.
        val prepared = if (quote.funding != null) {
            val leafIds = quote.leaves
                .filter { leaf -> leaf.method == RecoveryMethod.COOPERATIVE }
                .map { leaf -> leaf.leafId }
            if (leafIds.isEmpty()) {
                return
            }
            sdk.prepareRecoverFunds(
                PrepareRecoverFundsRequest(
                    feeRateSatPerVbyte = quote.feeRateSatPerVbyte,
                    destination = quote.destination,
                    selection = ExitLeafSelection.Specific(leafIds)
                )
            )
        } else {
            quote
        }
        val response = sdk.recoverFunds(
            RecoverFundsRequest(prepared = prepared),
            signer = null
        )

        // Keep the whole response: checkRecoverFunds follows the recovery from it.
        for (tx in response.transactions) {
            // Log.v("Breez", "Broadcast ${tx.txid}: ${tx.txHex}")
        }
        for (failure in response.failed) {
            // Log.v("Breez", "Leaf ${failure.leafId} was not recovered: ${failure.error}")
        }
        // ANCHOR_END: recover-cooperatively
    }

    suspend fun recoverWithFunding(sdk: BreezSdk, quote: PrepareRecoverFundsResponse) {
        // ANCHOR: recover-funds
        try {
            val secretKeyBytes = "your-secret-key-hex".hexToByteArray()
            val signer = singleKeyCpfpSigner(secretKeyBytes)

            val response = sdk.recoverFunds(
                RecoverFundsRequest(
                    prepared = quote,
                    fundingInputs = listOf(
                        CpfpInput.P2wpkh(
                            txid = "your-utxo-txid",
                            vout = 0u,
                            valueSats = 50_000u,
                            pubkey = "your-compressed-pubkey-hex"
                        )
                    )
                ),
                signer
            )

            // Keep the whole response: checkRecoverFunds follows the recovery from it.
            for (tx in response.transactions) {
                val blocks = tx.csvTimelockBlocks
                if (blocks != null) {
                    // Log.v("Breez", "${tx.txid}: wait $blocks blocks after its parents confirm")
                }
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: recover-funds
    }

    suspend fun checkRecovery(sdk: BreezSdk, stored: RecoverFundsResponse) {
        // ANCHOR: check-recover-funds
        val checked = sdk.checkRecoverFunds(
            CheckRecoverFundsRequest(recovery = stored)
        )

        // Store this one in place of the one you had.
        val recovery = checked.recovery

        when (val verdict = checked.verdict) {
            is RecoveryVerdict.Valid -> {
                for (tx in recovery.transactions) {
                    if (tx.status is ExitTransactionStatus.Ready) {
                        // Log.v("Breez", "ready to broadcast: ${tx.txid}")
                    }
                }
            }
            is RecoveryVerdict.Done -> {
                // Log.v("Breez", "Every transaction confirmed: the recovery is done")
            }
            is RecoveryVerdict.Redo -> {
                // Prepare and build again, naming the same leaves. Pass recovery.fundingInputs
                // back and the SDK follows them to whatever they have become.
                // Log.v("Breez", "Build the recovery again: ${verdict.reason}")
            }
        }
        // ANCHOR_END: check-recover-funds
    }

    suspend fun backUpExitState(sdk: BreezSdk): String {
        // ANCHOR: export-exit-state
        val exported = sdk.exportUnilateralExitState()

        // Keep the state somewhere the wallet's own storage cannot take with it.
        // Log.v("Breez", "Exit state is ${exported.exitState.length} bytes")
        // ANCHOR_END: export-exit-state

        return exported.exitState
    }

    suspend fun restoreExitState(sdk: BreezSdk, exitState: String) {
        // ANCHOR: import-exit-state
        val imported = sdk.importUnilateralExitState(
            ImportUnilateralExitStateRequest(exitState)
        )

        // Log.v(
        //     "Breez",
        //     "Imported ${imported.importedLeaves} leaves, " +
        //         "skipped ${imported.skippedForeignLeaves}"
        // )
        // ANCHOR_END: import-exit-state
    }

    suspend fun collectExitData(sdk: BreezSdk) {
        // ANCHOR: sync-exit-data
        // With automatic collection off, an explicit sync is what collects the data
        // a unilateral exit needs, and it waits for the collection to finish. Needs
        // the Spark operators reachable, so run it on a schedule rather than at the
        // moment an exit is needed.
        sdk.syncWallet(SyncWalletRequest)
        // ANCHOR_END: sync-exit-data
    }

    // ANCHOR: custom-cpfp-signer
    class MyFundingSigner : CpfpSigner {
        override suspend fun signPsbt(psbtBytes: ByteArray): ByteArray {
            val signedPsbtBytes = signWithFundingKeys(psbtBytes)
            return signedPsbtBytes
        }

        private fun signWithFundingKeys(psbtBytes: ByteArray): ByteArray {
            return psbtBytes
        }
    }
    // ANCHOR_END: custom-cpfp-signer
}
