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

    suspend fun receiveTokenPaymentSparkInvoice(sdk: BreezSdk) {
        // ANCHOR: receive-token-payment-spark-invoice
        try {
            val tokenIdentifier = "<token identifier>"
            val optionalDescription = "<invoice description>"
            // Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer, which is included in
            // package)
            val optionalAmount = BigInteger.fromLong(5_000L)
            // Android (BigInteger from java.math)
            // val optionalAmount = BigInteger.valueOf(5_000L)
            val optionalExpiryTimeSeconds = 1716691200.toULong()
            val optionalSenderPublicKey = "<sender public key>"

            val request = ReceivePaymentRequest(
                ReceivePaymentMethod.SparkInvoice(
                    tokenIdentifier = tokenIdentifier,
                    description = optionalDescription,
                    amount = optionalAmount,
                    expiryTime = optionalExpiryTimeSeconds,
                    senderPublicKey = optionalSenderPublicKey
                )
            )
            val response = sdk.receivePayment(request)

            val paymentRequest = response.paymentRequest
            println("Payment request: $paymentRequest")
            val receiveFee = response.fee
            println("Fees: $receiveFee token base units")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: receive-token-payment-spark-invoice
    }

    suspend fun sendTokenPayment(sdk: BreezSdk) {
        // ANCHOR: send-token-payment
        try {
            val paymentRequest = "<spark address or invoice>"
            // Token identifier must match the invoice in case it specifies one.
            val tokenIdentifier = "<token identifier>"
            // Set the amount of tokens you wish to send.
            // Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer, which is included in
            // package)
            val optionalAmount = BigInteger.fromLong(1_000L)
            // Android (BigInteger from java.math)
            // val optionalAmount = BigInteger.valueOf(1_000L)

            val prepareResponse =
                sdk.prepareSendPayment(
                    PrepareSendPaymentRequest(
                        paymentRequest = paymentRequest,
                        amount = optionalAmount,
                        tokenIdentifier = tokenIdentifier
                    )
                )

            // If the fees are acceptable, continue to send the token payment
            when (val method = prepareResponse.paymentMethod) {
                is SendPaymentMethod.SparkAddress -> {
                    println("Token ID: ${method.tokenIdentifier}")
                    println("Fees: ${method.fee} token base units")
                }
                is SendPaymentMethod.SparkInvoice -> {
                    println("Token ID: ${method.tokenIdentifier}")
                    println("Fees: ${method.fee} token base units")
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

    suspend fun fetchTokenConversionLimits(sdk: BreezSdk) {
        // ANCHOR: fetch-token-conversion-limits
        try {
            // Fetch limits for converting Bitcoin to a token
            val fromBitcoinResponse = sdk.fetchTokenConversionLimits(
                FetchTokenConversionLimitsRequest(
                    conversionType = TokenConversionType.FromBitcoin,
                    tokenIdentifier = "<token identifier>"
                )
            )

            if (fromBitcoinResponse.minFromAmount != null) {
                println("Minimum BTC to convert: ${fromBitcoinResponse.minFromAmount} sats")
            }
            if (fromBitcoinResponse.minToAmount != null) {
                println("Minimum tokens to receive: ${fromBitcoinResponse.minToAmount} base units")
            }

            // Fetch limits for converting a token to Bitcoin
            val toBitcoinResponse = sdk.fetchTokenConversionLimits(
                FetchTokenConversionLimitsRequest(
                    conversionType = TokenConversionType.ToBitcoin(
                        fromTokenIdentifier = "<token identifier>"
                    ),
                    tokenIdentifier = null
                )
            )

            if (toBitcoinResponse.minFromAmount != null) {
                println("Minimum tokens to convert: ${toBitcoinResponse.minFromAmount} base units")
            }
            if (toBitcoinResponse.minToAmount != null) {
                println("Minimum BTC to receive: ${toBitcoinResponse.minToAmount} sats")
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: fetch-token-conversion-limits
    }

    suspend fun prepareSendPaymentTokenConversion(sdk: BreezSdk) {
        // ANCHOR: prepare-send-payment-token-conversion
        try {
            val paymentRequest = "<spark address or invoice>"
            // Token identifier must match the invoice in case it specifies one.
            val tokenIdentifier = "<token identifier>"
            // Set the amount of tokens you wish to send.
            // Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer, which is included in
            // package)
            val optionalAmount = BigInteger.fromLong(1_000L)
            // Android (BigInteger from java.math)
            // val optionalAmount = BigInteger.valueOf(1_000L)
            // set to use Bitcoin funds to pay via token conversion
            val optionalMaxSlippageBps = 50u
            val optionalCompletionTimeoutSecs = 30u
            val tokenConversionOptions = TokenConversionOptions(
                conversionType = TokenConversionType.FromBitcoin,
                maxSlippageBps = optionalMaxSlippageBps,
                completionTimeoutSecs = optionalCompletionTimeoutSecs
            )

            val prepareResponse =
                sdk.prepareSendPayment(
                    PrepareSendPaymentRequest(
                        paymentRequest = paymentRequest,
                        amount = optionalAmount,
                        tokenIdentifier = tokenIdentifier,
                        tokenConversionOptions = tokenConversionOptions
                    )
                )

            // If the fees are acceptable, continue to send the token payment
            if (prepareResponse.tokenConversionFee != null) {
                val tokenConversionFee = prepareResponse.tokenConversionFee
                println("Estimated token conversion fee: ${tokenConversionFee} sats")
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: prepare-send-payment-token-conversion
    }
}
