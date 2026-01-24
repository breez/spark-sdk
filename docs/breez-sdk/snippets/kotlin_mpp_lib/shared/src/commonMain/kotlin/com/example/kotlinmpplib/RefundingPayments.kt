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
                            val maxFee = claimError.maxFee
                            val maxFeeStr = when (maxFee) {
                                is Fee.Fixed -> "${maxFee.amount} sats"
                                is Fee.Rate -> "${maxFee.satPerVbyte} sats/vByte"
                                null -> "none"
                            }
                            // Log.v("Breez", "Max claim fee exceeded. Max: $maxFeeStr, Required: ${claimError.requiredFeeSats} sats or ${claimError.requiredFeeRateSatPerVbyte} sats/vByte")
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
                val requiredFee = claimError.requiredFeeSats

                // Show UI to user with the required fee and get approval
                val userApproved = true // Replace with actual user approval logic

                if (userApproved) {
                    val claimRequest = ClaimDepositRequest(
                        txid = deposit.txid,
                        vout = deposit.vout,
                        maxFee = MaxFee.Fixed(requiredFee)
                    )
                    sdk.claimDeposit(claimRequest)
                }
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: handle-fee-exceeded
    }

    suspend fun refundDeposit(sdk: BreezSdk) {
        // ANCHOR: refund-deposit
        try {
            val txid = "your_deposit_txid"
            val vout = 0u
            val destinationAddress = "bc1qexample..." // Your Bitcoin address
            
            // Set the fee for the refund transaction using the half-hour feerate
            val recommendedFees = sdk.recommendedFees()
            val fee = Fee.Rate(recommendedFees.halfHourFee)
            // or using a fixed amount
            //val fee = Fee.Fixed(500u)
            //

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

    suspend fun setMaxFeeToRecommendedFees() {
        // ANCHOR: set-max-fee-to-recommended-fees
        // Create the default config
        val config = defaultConfig(Network.MAINNET)
        config.apiKey = "<breez api key>"

        // Set the maximum fee to the fastest network recommended fee at the time of claim
        // with a leeway of 1 sats/vbyte
        config.maxDepositClaimFee = MaxFee.NetworkRecommended(leewaySatPerVbyte = 1u)
        // ANCHOR_END: set-max-fee-to-recommended-fees
        println("Config: $config")
    }

    suspend fun customClaimLogic(sdk: BreezSdk, deposit: DepositInfo) {
        // ANCHOR: custom-claim-logic
        try {
            val claimError = deposit.claimError
            if (claimError is DepositClaimError.MaxDepositClaimFeeExceeded) {
                val requiredFeeRate = claimError.requiredFeeRateSatPerVbyte

                val recommendedFees = sdk.recommendedFees()

                if (requiredFeeRate <= recommendedFees.fastestFee) {
                    val claimRequest = ClaimDepositRequest(
                        txid = deposit.txid,
                        vout = deposit.vout,
                        maxFee = MaxFee.Rate(requiredFeeRate)
                    )
                    sdk.claimDeposit(claimRequest)
                }
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: custom-claim-logic
    }
}

suspend fun recommendedFees(sdk: BreezSdk) {
    // ANCHOR: recommended-fees
    val response = sdk.recommendedFees()
    println("Fastest fee: ${response.fastestFee} sats/vByte")
    println("Half-hour fee: ${response.halfHourFee} sats/vByte")
    println("Hour fee: ${response.hourFee} sats/vByte")
    println("Economy fee: ${response.economyFee} sats/vByte")
    println("Minimum fee: ${response.minimumFee} sats/vByte")
    // ANCHOR_END: recommended-fees
}