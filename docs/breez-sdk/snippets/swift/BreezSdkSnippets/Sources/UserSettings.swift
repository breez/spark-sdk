import BreezSdkSpark

func getUserSettings(sdk: BreezSdk) async throws {
    // ANCHOR: get-user-settings
    let userSettings = try await sdk.getUserSettings()
    print("User settings: \(userSettings)")
    // ANCHOR_END: get-user-settings
}

func updateUserSettings(sdk: BreezSdk) async throws {
    // ANCHOR: update-user-settings
    // Fields left as nil are not changed. Settings that take an enum value
    // accept set to assign one, or unset to clear it.
    try await sdk.updateUserSettings(
        request: UpdateUserSettingsRequest(
            sparkPrivateModeEnabled: true,
            stableBalanceActiveLabel: nil,
            sparkMasterIdentityPublicKey: .set(publicKey: "<hex encoded public key>")
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
