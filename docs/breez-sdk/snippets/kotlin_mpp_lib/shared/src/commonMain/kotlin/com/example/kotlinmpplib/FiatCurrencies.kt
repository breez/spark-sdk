package com.example.kotlinmpplib

import breez_sdk_spark.*

class FiatCurrencies {
    suspend fun listFiatCurrencies(sdk: BreezSdk) {
        // ANCHOR: list-fiat-currencies
        try {
            val response = sdk.listFiatCurrencies()
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: list-fiat-currencies
    }

    suspend fun listFiatRates(sdk: BreezSdk) {
        // ANCHOR: list-fiat-rates
        try {
            val response = sdk.listFiatRates()
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: list-fiat-rates
    }
}