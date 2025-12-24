package com.example.kotlinmpplib

import breez_sdk_spark.*
import com.ionspin.kotlin.bignum.integer.BigInteger

class IssuingTokens {
    fun getTokenIssuer(sdk: BreezSdk): TokenIssuer {
        // ANCHOR: get-token-issuer
        val tokenIssuer = sdk.getTokenIssuer()
        // ANCHOR_END: get-token-issuer
        return tokenIssuer
    }

    suspend fun createToken(tokenIssuer: TokenIssuer) {
        // ANCHOR: create-token
        try {
            val request =
                    CreateIssuerTokenRequest(
                            name = "My Token",
                            ticker = "MTK",
                            decimals = 6.toUInt(),
                            isFreezable = false,
                            maxSupply = BigInteger.fromLong(1_000_000L)
                    )
            val tokenMetadata = tokenIssuer.createIssuerToken(request)
            // Log.v("Breez", "Token identifier: ${tokenMetadata.identifier}")
        } catch (e: Exception) {
            // Handle exception
        }
        // ANCHOR_END: create-token
    }

    suspend fun createTokenWithCustomAccountNumber() {
        // ANCHOR: custom-account-number
        val accountNumber = 21u

        val mnemonic = "<mnemonic words>"
        val seed = Seed.Mnemonic(mnemonic, null)
        val config = defaultConfig(Network.MAINNET)
        config.apiKey = "<breez api key>"

        try {
            val builder = SdkBuilder(config, seed)
            builder.withDefaultStorage("./.data")

            // Set the account number for the SDK
            val keySetConfig = KeySetConfig(
                keySetType = KeySetType.DEFAULT,
                useAddressIndex = false,
                accountNumber = accountNumber
            )
            builder.withKeySet(keySetConfig)

            val sdk = builder.build()
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: custom-account-number
    }

    suspend fun mintToken(tokenIssuer: TokenIssuer) {
        // ANCHOR: mint-token
        try {
            val request =
                    MintIssuerTokenRequest(
                            amount = BigInteger.fromLong(1_000L),
                    )
            val payment = tokenIssuer.mintIssuerToken(request)
        } catch (e: Exception) {
            // Handle exception
        }
        // ANCHOR_END: mint-token
    }

    suspend fun burnToken(tokenIssuer: TokenIssuer) {
        // ANCHOR: burn-token
        try {
            val request =
                    BurnIssuerTokenRequest(
                            amount = BigInteger.fromLong(1_000L),
                    )
            val payment = tokenIssuer.burnIssuerToken(request)
        } catch (e: Exception) {
            // Handle exception
        }
        // ANCHOR_END: burn-token
    }

    suspend fun getTokenMetadata(tokenIssuer: TokenIssuer) {
        // ANCHOR: get-token-metadata
        try {
            val tokenBalance = tokenIssuer.getIssuerTokenBalance()
            // Log.v("Breez", "Token balance: ${tokenBalance.balance}")

            val tokenMetadata = tokenIssuer.getIssuerTokenMetadata()
            // Log.v("Breez", "Token ticker: ${tokenMetadata.ticker}")
        } catch (e: Exception) {
            // Handle exception
        }
        // ANCHOR_END: get-token-metadata
    }

    suspend fun freezeToken(tokenIssuer: TokenIssuer) {
        // ANCHOR: freeze-token
        try {
            val sparkAddress = "<spark address>"
            // Freeze the tokens held at the specified Spark address
            val freezeRequest =
                    FreezeIssuerTokenRequest(
                            address = sparkAddress,
                    )
            val freezeResponse = tokenIssuer.freezeIssuerToken(freezeRequest)

            // Unfreeze the tokens held at the specified Spark address
            val unfreezeRequest =
                    UnfreezeIssuerTokenRequest(
                            address = sparkAddress,
                    )
            val unfreezeResponse = tokenIssuer.unfreezeIssuerToken(unfreezeRequest)
        } catch (e: Exception) {
            // Handle exception
        }
        // ANCHOR_END: freeze-token
    }
}
