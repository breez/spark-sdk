package com.example.kotlinmpplib

import breez_sdk_spark.*
class GettingStarted {
    suspend fun initSdk() {
        // ANCHOR: init-sdk
        // Construct the seed using a mnemonic, entropy or passkey
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
                is SdkEvent.NewDeposits -> {
                    // Detected deposits, as DepositInfo. Only those with isMature set
                    // have enough confirmations to be claimed. Show the rest as pending.
                    val newDeposits = e.newDeposits
                }
                is SdkEvent.UnclaimedDeposits -> {
                    // Deposits the SDK could not claim. Each claimError says why,
                    // most often the fee exceeded the configured maximum.
                    val unclaimedDeposits = e.unclaimedDeposits
                }
                is SdkEvent.ClaimedDeposits -> {
                    // Deposits claimed into the wallet. An instant (0-conf) claim is
                    // reported here on submission and settles shortly after.
                    val claimedDeposits = e.claimedDeposits
                }
                is SdkEvent.PaymentSucceeded -> {
                    // A payment completed. The cached balance is already refreshed,
                    // so getInfo returns the new value.
                    val payment = e.payment
                }
                is SdkEvent.PaymentPending -> {
                    // A payment is awaiting confirmation. It arrives again as
                    // succeeded or failed once it settles.
                    val pendingPayment = e.payment
                }
                is SdkEvent.PaymentFailed -> {
                    // A payment failed. payment.details carries the method-specific
                    // context to show the user.
                    val failedPayment = e.payment
                }
                is SdkEvent.AutoOptimization -> {
                    // Background optimizer progress: started, round completed, or a
                    // terminal outcome. Manual optimizeLeaves calls do not emit these.
                    val optimizationEvent = e.optimizationEvent
                }
                is SdkEvent.LightningAddressChanged -> {
                    // The lightning address changed on another device. Unset when the
                    // address was deleted.
                    val lightningAddress = e.lightningAddress
                }
                is SdkEvent.UnilateralExitStateChanged -> {
                    // The unilateral exit state changed, so a previously exported
                    // one is now out of date. Export it again.
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

    // ANCHOR: spark-status
    suspend fun gettingStartedSparkStatus() {
        try {
            val sparkStatus = getSparkStatus()

            when (sparkStatus.status) {
                ServiceStatus.OPERATIONAL -> {
                    // Log.v("Breez", "Spark is fully operational")
                }
                ServiceStatus.DEGRADED -> {
                    // Log.v("Breez", "Spark is experiencing degraded performance")
                }
                ServiceStatus.PARTIAL -> {
                    // Log.v("Breez", "Spark is partially unavailable")
                }
                ServiceStatus.MAJOR -> {
                    // Log.v("Breez", "Spark is experiencing a major outage")
                }
                ServiceStatus.UNKNOWN -> {
                    // Log.v("Breez", "Spark status is unknown")
                }
            }

            // Log.v("Breez", "Last updated: ${sparkStatus.lastUpdated}")
        } catch (e: Exception) {
            // handle error
        }
    }
    // ANCHOR_END: spark-status

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
