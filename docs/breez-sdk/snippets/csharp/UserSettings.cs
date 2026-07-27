using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class UserSettings
    {
        async Task GetUserSettings(BreezSdk sdk)
        {
            // ANCHOR: get-user-settings
            var userSettings = await sdk.GetUserSettings();

            Console.WriteLine($"User settings: {userSettings}");
            // ANCHOR_END: get-user-settings
        }

        async Task UpdateUserSettings(BreezSdk sdk)
        {
            // ANCHOR: update-user-settings
            // Fields left as null are not changed. Settings that take an enum value
            // accept Set to assign one, or Unset to clear it.
            await sdk.UpdateUserSettings(
                request: new UpdateUserSettingsRequest(
                    sparkPrivateModeEnabled: true,
                    stableBalanceActiveLabel: null,
                    sparkMasterIdentityPublicKey: new SparkMasterIdentityPublicKey.Set(
                        publicKey: "<hex encoded public key>"
                    )
                )
            );
            // ANCHOR_END: update-user-settings
        }

        async Task ActivateStableBalance(BreezSdk sdk)
        {
            // ANCHOR: activate-stable-balance
            await sdk.UpdateUserSettings(
                request: new UpdateUserSettingsRequest(
                    sparkPrivateModeEnabled: null,
                    stableBalanceActiveLabel: new StableBalanceActiveLabel.Set(label: "USDB")
                )
            );
            // ANCHOR_END: activate-stable-balance
        }

        async Task DeactivateStableBalance(BreezSdk sdk)
        {
            // ANCHOR: deactivate-stable-balance
            await sdk.UpdateUserSettings(
                request: new UpdateUserSettingsRequest(
                    sparkPrivateModeEnabled: null,
                    stableBalanceActiveLabel: new StableBalanceActiveLabel.Unset()
                )
            );
            // ANCHOR_END: deactivate-stable-balance
        }
    }
}
