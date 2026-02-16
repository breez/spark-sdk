using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class UserSettings
    {
        async Task GetUserSettings(BreezClient client)
        {
            // ANCHOR: get-user-settings
            var userSettings = await client.Settings().Get();

            Console.WriteLine($"User settings: {userSettings}");
            // ANCHOR_END: get-user-settings
        }

        async Task UpdateUserSettings(BreezClient client)
        {
            // ANCHOR: update-user-settings
            var sparkPrivateModeEnabled = true;
            await client.Settings().Update(
                request: new UpdateUserSettingsRequest(
                    sparkPrivateModeEnabled: sparkPrivateModeEnabled
                )
            );
            // ANCHOR_END: update-user-settings
        }
    }
}
