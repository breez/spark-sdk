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
            var sparkPrivateModeEnabled = true;
            await sdk.UpdateUserSettings(
                request: new UpdateUserSettingsRequest(
                    sparkPrivateModeEnabled: sparkPrivateModeEnabled
                )
            );
            // ANCHOR_END: update-user-settings
        }
    }
}
