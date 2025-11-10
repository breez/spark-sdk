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
            val enableSparkPrivateMode = true
            sdk.updateUserSettings(UpdateUserSettingsRequest(enableSparkPrivateMode))
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: update-user-settings
    }
}