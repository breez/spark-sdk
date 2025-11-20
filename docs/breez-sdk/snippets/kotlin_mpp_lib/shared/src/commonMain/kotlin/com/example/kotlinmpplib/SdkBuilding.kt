package com.example.kotlinmpplib

import breez_sdk_spark.*
class SdkBuilding {
    suspend fun initSdkAdvanced() {
        // ANCHOR: init-sdk-advanced
        // Construct the seed using mnemonic words or entropy bytes
        val mnemonic = "<mnemonic words>"
        val seed = Seed.Mnemonic(mnemonic, null)

        // Create the default config
        val config = defaultConfig(Network.MAINNET)
        config.apiKey = "<breez api key>"

        try {
            // Build the SDK using the config, seed and default storage
            val builder = SdkBuilder(config, seed)
            builder.withDefaultStorage("./.data")
            // You can also pass your custom implementations:
            // builder.withStorage(<your storage implementation>)
            // builder.withRealTimeSyncStorage(<your real-time sync storage implementation>)
            // builder.withChainService(<your chain service implementation>)
            // builder.withRestClient(<your rest client implementation>)
            // builder.withKeySet(<your key set type>, <use address index>, <account number>)
            // builder.withPaymentObserver(<your payment observer implementation>)
            val sdk = builder.build()
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: init-sdk-advanced
    }

    suspend fun withRestChainService(builder: SdkBuilder) { 
        // ANCHOR: with-rest-chain-service
        val url = "<your REST chain service URL>"
        val chainApiType = ChainApiType.MEMPOOL_SPACE
        val optionalCredentials = Credentials(
            username = "<username>",
            password = "<password>"
        )
        builder.withRestChainService(
            url = url,
            apiType = chainApiType,
            credentials = optionalCredentials
        )
        // ANCHOR_END: with-rest-chain-service
    }

    suspend fun withKeySet(builder: SdkBuilder) {
        // ANCHOR: with-key-set
        val keySetType = KeySetType.DEFAULT
        val useAddressIndex = false
        val optionalAccountNumber = 21u
        builder.withKeySet(
            keySetType = keySetType,
            useAddressIndex = useAddressIndex,
            accountNumber = optionalAccountNumber
        )
        // ANCHOR_END: with-key-set
    }
}

// ANCHOR: with-storage
interface Storage {
    suspend fun `deleteCachedItem`(`key`: String)
    suspend fun `getCachedItem`(`key`: String): String?
    suspend fun `setCachedItem`(`key`: String, `value`: String)
    suspend fun `listPayments`(`request`: ListPaymentsRequest): List<Payment>
    suspend fun `insertPayment`(`payment`: Payment)
    suspend fun `setPaymentMetadata`(`paymentId`: String, `metadata`: PaymentMetadata)
    suspend fun `getPaymentById`(`id`: String): Payment
    suspend fun `getPaymentByInvoice`(`invoice`: String): Payment?
    suspend fun `addDeposit`(`txid`: String, `vout`: UInt, `amountSats`: ULong)
    suspend fun `deleteDeposit`(`txid`: String, `vout`: UInt)
    suspend fun `listDeposits`(): List<DepositInfo>
    suspend fun `updateDeposit`(`txid`: String, `vout`: UInt, `payload`: UpdateDepositPayload)
}
// ANCHOR_END: with-storage

// ANCHOR: with-sync-storage
interface SyncStorage {
    suspend fun `addOutgoingChange`(`record`: UnversionedRecordChange): ULong
    suspend fun `completeOutgoingSync`(`record`: Record)
    suspend fun `getPendingOutgoingChanges`(`limit`: UInt): List<OutgoingChange>
    suspend fun `getLastRevision`(): ULong
    suspend fun `insertIncomingRecords`(`records`: List<Record>)
    suspend fun `deleteIncomingRecord`(`record`: Record)
    suspend fun `rebasePendingOutgoingRecords`(`revision`: ULong)
    suspend fun `getIncomingRecords`(`limit`: UInt): List<IncomingChange>
    suspend fun `getLatestOutgoingChange`(): OutgoingChange?
    suspend fun `updateRecordFromIncoming`(`record`: Record)
}
// ANCHOR_END: with-sync-storage

// ANCHOR: with-chain-service
interface BitcoinChainService {
    suspend fun `getAddressUtxos`(`address`: String): List<Utxo>
    suspend fun `getTransactionStatus`(`txid`: String): TxStatus
    suspend fun `getTransactionHex`(`txid`: String): String
    suspend fun `broadcastTransaction`(`tx`: String)
}
// ANCHOR_END: with-chain-service

// ANCHOR: with-rest-client
interface RestClient {
    suspend fun `getRequest`(`url`: String, `headers`: Map<String, String>?): RestResponse
    suspend fun `postRequest`(`url`: String, `headers`: Map<String, String>?, `body`: String?): RestResponse
    suspend fun `deleteRequest`(`url`: String, `headers`: Map<String, String>?, `body`: String?): RestResponse
}
// ANCHOR_END: with-rest-client

// ANCHOR: with-fiat-service
interface FiatService {
    suspend fun `fetchFiatCurrencies`(): List<FiatCurrency>
    suspend fun `fetchFiatRates`(): List<Rate>
}
// ANCHOR_END: with-fiat-service

// ANCHOR: with-payment-observer
interface PaymentObserver {
    suspend fun `beforeSend`(`payments`: List<ProvisionalPayment>)
}
// ANCHOR_END: with-payment-observer
