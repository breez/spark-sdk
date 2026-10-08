package com.example.kotlinmpplib

import breez_sdk_spark.*
import com.ionspin.kotlin.bignum.integer.BigInteger

class TokenAllowances {
    suspend fun createTokenAllowance(sdk: BreezSdk) {
        // ANCHOR: create-token-allowance
        val response = sdk.createTokenAllowance(
            CreateTokenAllowanceRequest(
                spenderAddress = "<spender spark address>",
                tokenIdentifier = "<token identifier>",
                maxPerPayment = TokenAllowanceLimit.Amount(BigInteger.fromLong(5_000_000L)),
                maxTotal = TokenAllowanceLimit.Amount(BigInteger.fromLong(100_000_000L)),
                expiryTime = 1_798_761_600uL,
                allowedRecipients = emptyList(),
            )
        )
        // Log.v("Breez", "Allowance id: ${response.allowance.id}")
        // ANCHOR_END: create-token-allowance
    }

    suspend fun listTokenAllowances(sdk: BreezSdk) {
        // ANCHOR: list-token-allowances
        val response = sdk.listTokenAllowances(
            ListTokenAllowancesRequest(
                role = TokenAllowanceRole.OWNER,
                counterpartyAddress = null,
                tokenIdentifier = null,
                includeInactive = false,
                offset = null,
                limit = null,
            )
        )
        for (allowance in response.allowances) {
            // Log.v("Breez", "${allowance.id}: spent ${allowance.spentAmount}")
        }
        // ANCHOR_END: list-token-allowances
    }

    suspend fun revokeTokenAllowance(sdk: BreezSdk) {
        // ANCHOR: revoke-token-allowance
        sdk.revokeTokenAllowance(RevokeTokenAllowanceRequest(allowanceId = "<allowance id>"))
        // ANCHOR_END: revoke-token-allowance
    }

    suspend fun pullPayment(sdk: BreezSdk) {
        // ANCHOR: pull-payment
        val prepareResponse = sdk.preparePullPayment(
            PreparePullPaymentRequest(
                payerAddress = "<payer spark address>",
                tokenIdentifier = "<token identifier>",
                receivers = listOf(
                    PullReceiver(
                        amount = BigInteger.fromLong(5_000_000L),
                        receiverAddress = null,
                    )
                ),
            )
        )
        // Log.v("Breez", "Pulling ${prepareResponse.amount}")

        val response = sdk.pullPayment(PullPaymentRequest(prepareResponse))
        // Log.v("Breez", "Pull transaction: ${response.txHash}")
        // ANCHOR_END: pull-payment
    }
}
