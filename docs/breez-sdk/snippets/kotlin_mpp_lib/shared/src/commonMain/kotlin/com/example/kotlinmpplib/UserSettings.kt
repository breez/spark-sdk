package com.example.kotlinmpplib

import breez_sdk_spark.*

class UserSettings {
    suspend fun getUserSettings(sdk: BreezSdk) {
        // ANCHOR: get-user-settings
        try {
            val userSettings = sdk.getUserSettings()
            println("User settings: $userSettings")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: get-user-settings
    }

    suspend fun updateUserSettings(sdk: BreezSdk) {
        // ANCHOR: update-user-settings
        try {
            val sparkPrivateModeEnabled = true
            sdk.updateUserSettings(UpdateUserSettingsRequest(sparkPrivateModeEnabled))
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: update-user-settings
    }

    suspend fun activateStableBalance(sdk: BreezSdk) {
        // ANCHOR: activate-stable-balance
        try {
            sdk.updateUserSettings(UpdateUserSettingsRequest(
                sparkPrivateModeEnabled = null,
                stableBalanceActiveLabel = StableBalanceActiveLabel.Set(label = "USDB")
            ))
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: activate-stable-balance
    }

    suspend fun deactivateStableBalance(sdk: BreezSdk) {
        // ANCHOR: deactivate-stable-balance
        try {
            sdk.updateUserSettings(UpdateUserSettingsRequest(
                sparkPrivateModeEnabled = null,
                stableBalanceActiveLabel = StableBalanceActiveLabel.Unset
            ))
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: deactivate-stable-balance
    }
}
