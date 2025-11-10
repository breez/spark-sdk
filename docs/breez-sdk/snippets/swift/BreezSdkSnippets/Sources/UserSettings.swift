import BreezSdkSpark

func getUserSettings(sdk: BreezSdk) async throws {
    // ANCHOR: get-user-settings
    let userSettings = try await sdk.getUserSettings()
    print("User settings: \(userSettings)")
    // ANCHOR_END: get-user-settings
}

func updateUserSettings(sdk: BreezSdk) async throws {
    // ANCHOR: update-user-settings
    let enableSparkPrivateMode = true
    try await sdk.updateUserSettings(
        request: UpdateUserSettingsRequest(
            enableSparkPrivateMode: enableSparkPrivateMode
        ))
    // ANCHOR_END: update-user-settings
}
