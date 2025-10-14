package com.example.kotlinmpplib

import breez_sdk_spark.*
import com.ionspin.kotlin.bignum.integer.BigInteger

class Tokens {
    suspend fun fetchTokenBalances(sdk: BreezSdk) {
        // ANCHOR: fetch-token-balances
        try {
            // ensureSynced: true will ensure the SDK is synced with the Spark network
            // before returning the balance
            val info = sdk.getInfo(GetInfoRequest(false))

            // Token balances are a map of token identifier to balance
            val tokenBalances = info.tokenBalances
            for ((tokenId, tokenBalance) in tokenBalances) {
                println("Token ID: $tokenId")
                println("Balance: ${tokenBalance.balance}")
                println("Name: ${tokenBalance.tokenMetadata.name}")
                println("Ticker: ${tokenBalance.tokenMetadata.ticker}")
                println("Decimals: ${tokenBalance.tokenMetadata.decimals}")
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: fetch-token-balances
    }

    suspend fun fetchTokenMetadata(sdk: BreezSdk) {
        // ANCHOR: fetch-token-metadata
        try {
            val response = 
                sdk.getTokensMetadata(
                    GetTokensMetadataRequest(
                        tokenIdentifiers = listOf("<token identifier 1>", "<token identifier 2>")
                )
            )   

            val tokensMetadata = response.tokensMetadata
            for (tokenMetadata in tokensMetadata) {
                println("Token ID: ${tokenMetadata.identifier}")
                println("Name: ${tokenMetadata.name}")
                println("Ticker: ${tokenMetadata.ticker}")
                println("Decimals: ${tokenMetadata.decimals}")
                println("Max Supply: ${tokenMetadata.maxSupply}")
                println("Is Freezable: ${tokenMetadata.isFreezable}")
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: fetch-token-metadata
    }

    suspend fun sendTokenPayment(sdk: BreezSdk) {
        // ANCHOR: send-token-payment
        try {
            val paymentRequest = "<spark address>"
            // The token identifier (e.g., asset ID or token contract)
            val tokenIdentifier = "<token identifier>"
            // Set the amount of tokens you wish to send
            // Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer, which is included in package)
            val amount = BigInteger.fromLong(1_000L)
            // Android (BigInteger from java.math)
            // val amount = BigInteger.valueOf(1_000L) // Android (BigInteger from java.math)

            val prepareResponse =
                sdk.prepareSendPayment(
                    PrepareSendPaymentRequest(
                        paymentRequest = paymentRequest,
                        amount = amount,
                        tokenIdentifier = tokenIdentifier
                    )
                )

            // If the fees are acceptable, continue to send the token payment
            when (val method = prepareResponse.paymentMethod) {
                is SendPaymentMethod.SparkAddress -> {
                    println("Token ID: ${method.tokenIdentifier}")
                    println("Fees: ${method.fee} sats")
                }
                else -> {}
            }

            // Send the token payment
            val sendResponse =
                sdk.sendPayment(
                    SendPaymentRequest(prepareResponse = prepareResponse, options = null)
                )
            val payment = sendResponse.payment
            println("Payment: $payment")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: send-token-payment
    }
}
