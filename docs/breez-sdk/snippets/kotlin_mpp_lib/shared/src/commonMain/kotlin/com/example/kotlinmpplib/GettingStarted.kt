package com.example.kotlinmpplib

import breez_sdk_spark.*
class GettingStarted {
    suspend fun initSdk() {
        // ANCHOR: init-sdk
        // Construct the seed using mnemonic words or entropy bytes
        val mnemonic = "<mnemonic words>"
        val seed = Seed.Mnemonic(mnemonic, null)

        // Create the default config
        val config = defaultConfig(Network.MAINNET)
        config.apiKey = "<breez api key>"

        try {
            // Connect to the SDK using the simplified connect method
            val sdk = connect(ConnectRequest(
                config = config,
                seed = seed,
                storageDir = "./.data"
            ))
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: init-sdk
    }

    suspend fun fetchBalance(sdk: BreezSdk) {
        // ANCHOR: fetch-balance
        try {
            // ensureSynced: true will ensure the SDK is synced with the Spark network
            // before returning the balance
            val info = sdk.getInfo(GetInfoRequest(false))
            val identityPubkey = info.identityPubkey
            val balanceSats = info.balanceSats
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: fetch-balance
    }

    // ANCHOR: logging
    class SdkLogger : Logger {
        override fun log(l: LogEntry) {
            // Log.v("SDKListener", "Received log [${l.level}]: ${l.line}")
        }
    }

    fun setLogger(logger: SdkLogger) {
        try {
            initLogging(null, logger, null)
        } catch (e: Exception) {
            // handle error
        }
    }
    // ANCHOR_END: logging

    // ANCHOR: add-event-listener
    class SdkListener : EventListener {
        override suspend fun onEvent(e: SdkEvent) {
            when (e) {
                is SdkEvent.Synced -> {
                    // Data has been synchronized with the network. When this event is received,
                    // it is recommended to refresh the payment list and wallet balance.
                }
                is SdkEvent.UnclaimedDeposits -> {
                    // SDK was unable to claim some deposits automatically
                    val unclaimedDeposits = e.unclaimedDeposits
                }
                is SdkEvent.ClaimedDeposits -> {
                    // Deposits were successfully claimed
                    val claimedDeposits = e.claimedDeposits
                }
                is SdkEvent.PaymentSucceeded -> {
                    // A payment completed successfully
                    val payment = e.payment
                }
                is SdkEvent.PaymentPending -> {
                    // A payment is pending (waiting for confirmation)
                    val pendingPayment = e.payment
                }
                is SdkEvent.PaymentFailed -> {
                    // A payment failed
                    val failedPayment = e.payment
                }
                is SdkEvent.Optimization -> {
                    // An optimization event occurred
                    val optimizationEvent = e.optimizationEvent
                }
                else -> {
                    // Handle any future event types
                }
            }
        }
    }

    suspend fun addEventListener(sdk: BreezSdk, listener: SdkListener): String? {
        try {
            val listenerId = sdk.addEventListener(listener)
            return listenerId
        } catch (e: Exception) {
            // handle error
            return null
        }
    }
    // ANCHOR_END: add-event-listener

    // ANCHOR: remove-event-listener
    suspend fun removeEventListener(sdk: BreezSdk, listenerId: String)  {
        try {
            sdk.removeEventListener(listenerId)
        } catch (e: Exception) {
            // handle error
        }
    }
    // ANCHOR_END: remove-event-listener

    // ANCHOR: disconnect
    suspend fun disconnect(sdk: BreezSdk)  {
        try {
            sdk.disconnect()
        } catch (e: Exception) {
            // handle error
        }
    }
    // ANCHOR_END: disconnect
}
