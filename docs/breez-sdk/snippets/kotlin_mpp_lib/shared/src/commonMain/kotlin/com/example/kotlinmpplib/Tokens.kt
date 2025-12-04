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

    suspend fun prepareConvertTokenToBitcoin(sdk: BreezSdk) {
        // ANCHOR: prepare-convert-token-to-bitcoin
        try {
            val tokenIdentifier = "<token identifier>"
            // Amount in token base units
            // Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer, which is included in
            // package)
            val amount = BigInteger.fromLong(10_000_000L)
            // Android (BigInteger from java.math)
            // val amount = BigInteger.valueOf(10_000_000L)

            val prepareResponse =
                sdk.prepareConvertToken(
                    PrepareConvertTokenRequest(
                        convertType = ConvertType.TO_BITCOIN,
                        tokenIdentifier = tokenIdentifier,
                        amount = amount,
                    )
                )

            val estimatedReceiveAmount = prepareResponse.estimatedReceiveAmount
            val fee = prepareResponse.fee
            println("Estimated receive amount: $estimatedReceiveAmount sats")
            println("Fees: $fee token base units")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: prepare-convert-token-to-bitcoin
    }

    suspend fun prepareConvertTokenFromBitcoin(sdk: BreezSdk) {
        // ANCHOR: prepare-convert-token-from-bitcoin
        try {
            val tokenIdentifier = "<token identifier>"
            // Amount in satoshis
            // Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer, which is included in
            // package)
            val amount = BigInteger.fromLong(10_000L)
            // Android (BigInteger from java.math)
            // val amount = BigInteger.valueOf(10_000L)

            val prepareResponse =
                sdk.prepareConvertToken(
                    PrepareConvertTokenRequest(
                        convertType = ConvertType.FROM_BITCOIN,
                        tokenIdentifier = tokenIdentifier,
                        amount = amount,
                    )
                )

            val estimatedReceiveAmount = prepareResponse.estimatedReceiveAmount
            val fee = prepareResponse.fee
            println("Estimated receive amount: $estimatedReceiveAmount token base units")
            println("Fees: $fee sats")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: prepare-convert-token-from-bitcoin
    }

    suspend fun convertToken(sdk: BreezSdk, prepareResponse: PrepareConvertTokenResponse) {
        // ANCHOR: convert-token
        try {
            // Set the maximum slippage to 1% in basis points
            val optionalMaxSlippageBps = 100U

            val response =
                sdk.convertToken(
                    ConvertTokenRequest(
                        prepareResponse = prepareResponse,
                        maxSlippageBps = optionalMaxSlippageBps
                    )
                )

            val sentPayment = response.sentPayment
            val receivedPayment = response.receivedPayment
            println("Sent payment: $sentPayment")
            println("Received payment: $receivedPayment")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: convert-token
    }
}
