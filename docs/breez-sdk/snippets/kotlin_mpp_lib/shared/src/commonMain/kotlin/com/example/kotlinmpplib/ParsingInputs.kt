package com.example.kotlinmpplib

import breez_sdk_spark.*

class ParsingInputs {
    suspend fun parseInput() {
        // ANCHOR: parse-inputs
        val input = "an input to be parsed..."

        try {
            val inputType = parse(input)
            when (inputType) {
                is InputType.BitcoinAddress -> {
                    println("Input is Bitcoin address ${inputType.v1.address}")
                }
                is InputType.Bolt11Invoice -> {
                    val amountStr = inputType.v1.amountMsat?.toString() ?: "unknown"
                    println("Input is BOLT11 invoice for $amountStr msats")
                }
                is InputType.LnurlPay -> {
                    println("Input is LNURL-Pay/Lightning address accepting min/max " +
                           "${inputType.v1.minSendable}/${inputType.v1.maxSendable} msats}")
                }
                is InputType.LnurlWithdraw -> {
                    println("Input is LNURL-Withdraw for min/max " +
                           "${inputType.v1.minWithdrawable}/${inputType.v1.maxWithdrawable} msats")
                }
                else -> {
                    // Handle other input types
                } 
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: parse-inputs
    }
}
