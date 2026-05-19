using Breez.Sdk.Spark;

namespace BreezCli;

/// <summary>
/// Represents a single stable-balance subcommand.
/// </summary>
public class StableBalanceCliCommand
{
    public required string Name { get; init; }
    public required string Description { get; init; }
    public required Func<BreezSdk, Func<string, string?>, string[], Task> Run { get; init; }
}

/// <summary>
/// Stable balance subcommand names (used for REPL completion).
/// </summary>
public static class StableBalanceCommandNames
{
    public static readonly string[] All =
    {
        "stable-balance get",
        "stable-balance set",
        "stable-balance unset",
    };
}

/// <summary>
/// Stable balance subcommand handlers.
/// </summary>
public static class StableBalanceCommands
{
    /// <summary>
    /// Builds the stable-balance subcommand registry.
    /// </summary>
    public static Dictionary<string, StableBalanceCliCommand> BuildRegistry()
    {
        return new Dictionary<string, StableBalanceCliCommand>
        {
            ["get"] = new()
            {
                Name = "get",
                Description = "Get the stable balance active label",
                Run = HandleGet
            },
            ["set"] = new()
            {
                Name = "set",
                Description = "Set the stable balance active label",
                Run = HandleSet
            },
            ["unset"] = new()
            {
                Name = "unset",
                Description = "Unset stable balance",
                Run = HandleUnset
            },
        };
    }

    /// <summary>
    /// Dispatches a stable-balance subcommand.
    /// </summary>
    public static async Task DispatchCommand(
        string[] args,
        BreezSdk sdk,
        Func<string, string?> readline)
    {
        var registry = BuildRegistry();

        if (args.Length == 0 || args[0] == "help")
        {
            Console.WriteLine();
            Console.WriteLine("Stable balance subcommands:");
            var names = registry.Keys.OrderBy(k => k).ToList();
            foreach (var name in names)
            {
                Console.WriteLine($"  stable-balance {name,-24} {registry[name].Description}");
            }
            Console.WriteLine();
            return;
        }

        var subName = args[0];
        var subArgs = args.Skip(1).ToArray();

        if (!registry.TryGetValue(subName, out var cmd))
        {
            Console.WriteLine($"Unknown stable-balance subcommand: {subName}. Use 'stable-balance help' for available commands.");
            return;
        }

        await cmd.Run(sdk, readline, subArgs);
    }

    // --- get ---

    private static async Task HandleGet(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var settings = await sdk.GetUserSettings();
        Serialization.PrintValue(settings.stableBalanceActiveLabel);
    }

    // --- set ---

    private static async Task HandleSet(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        if (args.Length < 1)
        {
            Console.WriteLine("Usage: stable-balance set <label>");
            return;
        }

        await sdk.UpdateUserSettings(new UpdateUserSettingsRequest(
            sparkPrivateModeEnabled: null,
            stableBalanceActiveLabel: new StableBalanceActiveLabel.Set(label: args[0])
        ));
        var settings = await sdk.GetUserSettings();
        Serialization.PrintValue(settings);
    }

    // --- unset ---

    private static async Task HandleUnset(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        await sdk.UpdateUserSettings(new UpdateUserSettingsRequest(
            sparkPrivateModeEnabled: null,
            stableBalanceActiveLabel: new StableBalanceActiveLabel.Unset()
        ));
        var settings = await sdk.GetUserSettings();
        Serialization.PrintValue(settings);
    }
}
