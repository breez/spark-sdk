package com.example.kotlinmpplib

import breez_sdk_spark.*
import com.ionspin.kotlin.bignum.integer.BigInteger

class IssuingTokens {
    fun getIssuerSdk(sdk: BreezSdk): BreezIssuerSdk {
        // ANCHOR: get-issuer-sdk
        val issuerSdk = sdk.getIssuerSdk()
        // ANCHOR_END: get-issuer-sdk
        return issuerSdk
    }

    suspend fun createToken(issuerSdk: BreezIssuerSdk) {
        // ANCHOR: create-token
        try {
            val request = CreateIssuerTokenRequest(
                name = "My Token",
                ticker = "MTK",
                decimals = 6.toUInt(),
                isFreezable = false,
                maxSupply = BigInteger.fromLong(1_000_000L)
            )
            val tokenMetadata = issuerSdk.createIssuerToken(request)
            // Log.v("Breez", "Token identifier: ${tokenMetadata.identifier}")
        } catch (e: Exception) {
            // Handle exception
        }
        // ANCHOR_END: create-token
    }

    suspend fun mintToken(issuerSdk: BreezIssuerSdk) {
        // ANCHOR: mint-token
        try {
            val request = MintIssuerTokenRequest(
                amount = BigInteger.fromLong(1_000L),
            )
            val payment = issuerSdk.mintIssuerToken(request)
        } catch (e: Exception) {
            // Handle exception
        }
        // ANCHOR_END: mint-token
    }

    suspend fun burnToken(issuerSdk: BreezIssuerSdk) {
        // ANCHOR: burn-token
        try {
            val request = BurnIssuerTokenRequest(
                amount = BigInteger.fromLong(1_000L),
            )
            val payment = issuerSdk.burnIssuerToken(request)
        } catch (e: Exception) {
            // Handle exception
        }
        // ANCHOR_END: burn-token
    }

    suspend fun getTokenMetadata(issuerSdk: BreezIssuerSdk) {
        // ANCHOR: get-token-metadata
        try {
            val tokenBalance = issuerSdk.getIssuerTokenBalance()
            // Log.v("Breez", "Token balance: ${tokenBalance.balance}")

            val tokenMetadata = issuerSdk.getIssuerTokenMetadata()
            // Log.v("Breez", "Token ticker: ${tokenMetadata.ticker}")
        } catch (e: Exception) {
            // Handle exception
        }
        // ANCHOR_END: get-token-metadata
    }

    suspend fun freezeToken(issuerSdk: BreezIssuerSdk) {
        // ANCHOR: freeze-token
        try {
            val sparkAddress = "<spark address>"
            // Freeze the tokens held at the specified Spark address
            val freezeRequest = FreezeIssuerTokenRequest(
                address = sparkAddress,
            )
            val freezeResponse = issuerSdk.freezeIssuerToken(freezeRequest)

            // Unfreeze the tokens held at the specified Spark address
            val unfreezeRequest = UnfreezeIssuerTokenRequest(
                address = sparkAddress,
            )
            val unfreezeResponse = issuerSdk.unfreezeIssuerToken(unfreezeRequest)
        } catch (e: Exception) {
            // Handle exception
        }
        // ANCHOR_END: freeze-token
    }
}