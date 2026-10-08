package com.example.kotlinmpplib

import breez_sdk_spark.*

class Errors {
    suspend fun handleErrors(sdk: BreezSdk, request: PrepareSendPaymentRequest) {
        // ANCHOR: handle-errors
        try {
            val prepareResponse = sdk.prepareSendPayment(request)
            // Log.v("Breez", "Payment prepared: ${prepareResponse.paymentMethod}")
        } catch (e: SdkException.InsufficientFunds) {
            // Log.v("Breez", "Not enough funds for this payment")
        } catch (e: SdkException.CrossChainDisabled) {
            // Log.v("Breez", "Cross-chain payments are not enabled, see ${e.docsUrl}")
        } catch (e: SdkException) {
            // Log.v("Breez", "Failed to prepare the payment: ${e.message}")
        }
        // ANCHOR_END: handle-errors
    }
}
