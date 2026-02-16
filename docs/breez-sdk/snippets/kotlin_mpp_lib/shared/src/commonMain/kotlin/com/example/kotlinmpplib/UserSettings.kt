package com.example.kotlinmpplib

import breez_sdk_spark.*

class UserSettings {
    suspend fun getUserSettings(client: BreezClient) {
        // ANCHOR: get-user-settings
        try {
            val userSettings = client.settings().get()
            println("User settings: $userSettings")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: get-user-settings
    }

    suspend fun updateUserSettings(client: BreezClient) {
        // ANCHOR: update-user-settings
        try {
            val sparkPrivateModeEnabled = true
            client.settings().update(UpdateUserSettingsRequest(sparkPrivateModeEnabled))
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: update-user-settings
    }
}