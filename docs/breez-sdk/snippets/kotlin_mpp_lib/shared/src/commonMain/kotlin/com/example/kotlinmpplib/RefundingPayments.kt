package com.example.kotlinmpplib

import breez_sdk_spark.*

class RefundingPayments {
    suspend fun listUnclaimedDeposits(sdk: BreezSdk) {
        // ANCHOR: list-unclaimed-deposits
        try {
            val request = ListUnclaimedDepositsRequest
            val response = sdk.listUnclaimedDeposits(request)
            
            for (deposit in response.deposits) {
                // Log.v("Breez", "Unclaimed deposit: ${deposit.txid}:${deposit.vout}")
                // Log.v("Breez", "Amount: ${deposit.amountSats} sats")
                
                deposit.claimError?.let { claimError ->
                    when (claimError) {
                        is DepositClaimError.MaxDepositClaimFeeExceeded -> {
                            val maxFeeStr = claimError.maxFee?.let { "${it} sats" } ?: "none"
                            // Log.v("Breez", "Max claim fee exceeded. Max: $maxFeeStr, Required: ${claimError.requiredFee} sats")
                        }
                        is DepositClaimError.MissingUtxo -> {
                            // Log.v("Breez", "UTXO not found when claiming deposit")
                        }
                        is DepositClaimError.Generic -> {
                            // Log.v("Breez", "Claim failed: ${claimError.message}")
                        }
                    }
                }
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: list-unclaimed-deposits
    }

    suspend fun handleFeeExceeded(sdk: BreezSdk, deposit: DepositInfo) {
        // ANCHOR: handle-fee-exceeded
        try {
            val claimError = deposit.claimError
            if (claimError is DepositClaimError.MaxDepositClaimFeeExceeded) {
                val requiredFee = claimError.requiredFee

                // Show UI to user with the required fee and get approval
                val userApproved = true // Replace with actual user approval logic

                if (userApproved) {
                    val claimRequest = ClaimDepositRequest(
                        txid = deposit.txid,
                        vout = deposit.vout,
                        maxFee = Fee.Fixed(requiredFee)
                    )
                    sdk.claimDeposit(claimRequest)
                }
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: handle-fee-exceeded
    }

    suspend fun claimDeposit(sdk: BreezSdk) {
        // ANCHOR: claim-deposit
        try {
            val txid = "your_deposit_txid"
            val vout = 0u
            
            // Set a higher max fee to retry claiming
            val maxFee = Fee.Fixed(5000u)
            
            val request = ClaimDepositRequest(
                txid = txid,
                vout = vout,
                maxFee = maxFee
            )
            
            val response = sdk.claimDeposit(request)
            // Log.v("Breez", "Deposit claimed successfully. Payment: ${response.payment}")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: claim-deposit
    }

    suspend fun refundDeposit(sdk: BreezSdk) {
        // ANCHOR: refund-deposit
        try {
            val txid = "your_deposit_txid"
            val vout = 0u
            val destinationAddress = "bc1qexample..." // Your Bitcoin address
            
            // Set the fee for the refund transaction using a rate
            val fee = Fee.Rate(5u)
            // or using a fixed amount
            //val fee = Fee.Fixed(500u)
            
            val request = RefundDepositRequest(
                txid = txid,
                vout = vout,
                destinationAddress = destinationAddress,
                fee = fee
            )
            
            val response = sdk.refundDeposit(request)
            // Log.v("Breez", "Refund transaction created:")
            // Log.v("Breez", "Transaction ID: ${response.txId}")
            // Log.v("Breez", "Transaction hex: ${response.txHex}")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: refund-deposit
    }
}

suspend fun recommendedFees(sdk: BreezSdk) {
    // ANCHOR: recommended-fees
    val response = sdk.recommendedFees()
    println("Fastest fee: ${response.fastestFee} sats")
    println("Half-hour fee: ${response.halfHourFee} sats")
    println("Hour fee: ${response.hourFee} sats")
    println("Economy fee: ${response.economyFee} sats")
    println("Minimum fee: ${response.minimumFee} sats")
    // ANCHOR_END: recommended-fees
}