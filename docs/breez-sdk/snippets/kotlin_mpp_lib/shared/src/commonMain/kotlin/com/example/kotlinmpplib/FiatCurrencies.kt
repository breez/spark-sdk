package com.example.kotlinmpplib

import breez_sdk_spark.*

class FiatCurrencies {
    suspend fun listFiatCurrencies(client: BreezClient) {
        // ANCHOR: list-fiat-currencies
        try {
            val response = client.listFiatCurrencies()
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: list-fiat-currencies
    }

    suspend fun listFiatRates(client: BreezClient) {
        // ANCHOR: list-fiat-rates
        try {
            val response = client.listFiatRates()
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: list-fiat-rates
    }
}