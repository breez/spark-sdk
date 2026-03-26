import BreezSdkSpark

func getUserSettings(sdk: BreezSdk) async throws {
    // ANCHOR: get-user-settings
    let userSettings = try await sdk.getUserSettings()
    print("User settings: \(userSettings)")
    // ANCHOR_END: get-user-settings
}

func updateUserSettings(sdk: BreezSdk) async throws {
    // ANCHOR: update-user-settings
    let sparkPrivateModeEnabled = true
    try await sdk.updateUserSettings(
        request: UpdateUserSettingsRequest(
            sparkPrivateModeEnabled: sparkPrivateModeEnabled
        ))
    // ANCHOR_END: update-user-settings
}

func activateStableBalance(sdk: BreezSdk) async throws {
    // ANCHOR: activate-stable-balance
    try await sdk.updateUserSettings(
        request: UpdateUserSettingsRequest(
            sparkPrivateModeEnabled: nil,
            stableBalanceActiveLabel: .set(label: "USDB")
        ))
    // ANCHOR_END: activate-stable-balance
}

func deactivateStableBalance(sdk: BreezSdk) async throws {
    // ANCHOR: deactivate-stable-balance
    try await sdk.updateUserSettings(
        request: UpdateUserSettingsRequest(
            sparkPrivateModeEnabled: nil,
            stableBalanceActiveLabel: .unset
        ))
    // ANCHOR_END: deactivate-stable-balance
}
