package com.example.kotlinmpplib

import breez_sdk_spark.*

class ParsingInputs {
    suspend fun parseInput(sdk: BreezSdk) {
        // ANCHOR: parse-inputs
        val input = "an input to be parsed..."

        try {
            val inputType = sdk.parse(input)
            when (inputType) {
                is InputType.BitcoinAddress -> {
                    println("Input is Bitcoin address ${inputType.v1.address}")
                }
                is InputType.Bolt11Invoice -> {
                    val amountStr = inputType.v1.amountMsat?.toString() ?: "unknown"
                    println("Input is BOLT11 invoice for $amountStr msats")
                }
                is InputType.LnurlPay -> {
                    println(
                            "Input is LNURL-Pay/Lightning address accepting min/max " +
                                    "${inputType.v1.minSendable}/${inputType.v1.maxSendable} msats}"
                    )
                }
                is InputType.LnurlWithdraw -> {
                    println(
                            "Input is LNURL-Withdraw for min/max " +
                                    "${inputType.v1.minWithdrawable}/${inputType.v1.maxWithdrawable} msats"
                    )
                }
                is InputType.SparkAddress -> {
                    println("Input is Spark address ${inputType.v1.address}")
                }
                is InputType.SparkInvoice -> {
                    val invoice = inputType.v1
                    println("Input is Spark invoice:")
                    if (invoice.tokenIdentifier != null) {
                        println(
                                "  Amount: ${invoice.amount} base units of token with id ${invoice.tokenIdentifier}"
                        )
                    } else {
                        println("  Amount: ${invoice.amount} sats")
                    }

                    if (invoice.description != null) {
                        println("  Description: ${invoice.description}")
                    }

                    if (invoice.expiryTime != null) {
                        println("  Expiry time: ${invoice.expiryTime}")
                    }

                    if (invoice.senderPublicKey != null) {
                        println("  Sender public key: ${invoice.senderPublicKey}")
                    }
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

    suspend fun setExternalInputParsers() {
        // ANCHOR: set-external-input-parsers
        // Create the default config
        val config = defaultConfig(Network.MAINNET)
        config.apiKey = "<breez api key>"

        // Configure external parsers
        config.externalInputParsers = listOf(
            ExternalInputParser(
                providerId = "provider_a",
                inputRegex = "^provider_a",
                parserUrl = "https://parser-domain.com/parser?input=<input>"
            ),
            ExternalInputParser(
                providerId = "provider_b",
                inputRegex = "^provider_b",
                parserUrl = "https://parser-domain.com/parser?input=<input>"
            )
        )
        // ANCHOR_END: set-external-input-parsers
    }
}
