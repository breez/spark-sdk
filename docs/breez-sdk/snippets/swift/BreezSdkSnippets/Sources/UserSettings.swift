import BreezSdkSpark

func getUserSettings(client: BreezClient) async throws {
    // ANCHOR: get-user-settings
    let userSettings = try await client.getUserSettings()
    print("User settings: \(userSettings)")
    // ANCHOR_END: get-user-settings
}

func updateUserSettings(client: BreezClient) async throws {
    // ANCHOR: update-user-settings
    let sparkPrivateModeEnabled = true
    try await client.updateUserSettings(
        request: UpdateUserSettingsRequest(
            sparkPrivateModeEnabled: sparkPrivateModeEnabled
        ))
    // ANCHOR_END: update-user-settings
}
