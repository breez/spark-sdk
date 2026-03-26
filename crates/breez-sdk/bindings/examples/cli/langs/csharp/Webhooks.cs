using Breez.Sdk.Spark;

namespace BreezCli;

/// <summary>
/// Represents a single webhooks subcommand.
/// </summary>
public class WebhookCliCommand
{
    public required string Name { get; init; }
    public required string Description { get; init; }
    public required Func<BreezSdk, Func<string, string?>, string[], Task> Run { get; init; }
}

/// <summary>
/// Webhook subcommand names (used for REPL completion).
/// </summary>
public static class WebhookCommandNames
{
    public static readonly string[] All =
    {
        "webhooks register",
        "webhooks unregister",
        "webhooks list",
    };
}

/// <summary>
/// Webhook subcommand handlers.
/// </summary>
public static class WebhookCommands
{
    /// <summary>
    /// Builds the webhooks subcommand registry.
    /// </summary>
    public static Dictionary<string, WebhookCliCommand> BuildRegistry()
    {
        return new Dictionary<string, WebhookCliCommand>
        {
            ["register"] = new()
            {
                Name = "register",
                Description = "Register a new webhook",
                Run = HandleRegister
            },
            ["unregister"] = new()
            {
                Name = "unregister",
                Description = "Unregister a webhook",
                Run = HandleUnregister
            },
            ["list"] = new()
            {
                Name = "list",
                Description = "List all registered webhooks",
                Run = HandleList
            },
        };
    }

    /// <summary>
    /// Dispatches a webhooks subcommand.
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
            Console.WriteLine("Webhook subcommands:");
            var names = registry.Keys.OrderBy(k => k).ToList();
            foreach (var name in names)
            {
                Console.WriteLine($"  webhooks {name,-30} {registry[name].Description}");
            }
            Console.WriteLine();
            return;
        }

        var subName = args[0];
        var subArgs = args.Skip(1).ToArray();

        if (!registry.TryGetValue(subName, out var cmd))
        {
            Console.WriteLine($"Unknown webhooks subcommand: {subName}. Use 'webhooks help' for available commands.");
            return;
        }

        await cmd.Run(sdk, readline, subArgs);
    }

    // -----------------------------------------------------------------------
    // Webhook command handlers
    // -----------------------------------------------------------------------

    private static WebhookEventType? ParseEventType(string s)
    {
        return s.ToLower() switch
        {
            "lightning-receive" => new WebhookEventType.LightningReceiveFinished(),
            "lightning-send" => new WebhookEventType.LightningSendFinished(),
            "coop-exit" => new WebhookEventType.CoopExitFinished(),
            "static-deposit" => new WebhookEventType.StaticDepositFinished(),
            _ => null
        };
    }

    // --- register ---

    private static async Task HandleRegister(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        if (args.Length < 3)
        {
            Console.WriteLine("Usage: webhooks register <url> <secret> <event_type> [<event_type> ...]");
            Console.WriteLine("Event types: lightning-receive, lightning-send, coop-exit, static-deposit");
            return;
        }

        var url = args[0];
        var secret = args[1];
        var eventTypes = new List<WebhookEventType>();

        for (int i = 2; i < args.Length; i++)
        {
            var eventType = ParseEventType(args[i]);
            if (eventType == null)
            {
                Console.WriteLine($"Unknown event type: {args[i]}. Valid values: lightning-receive, lightning-send, coop-exit, static-deposit");
                return;
            }
            eventTypes.Add(eventType);
        }

        var result = await sdk.RegisterWebhook(request: new RegisterWebhookRequest(
            url: url,
            secret: secret,
            eventTypes: eventTypes.ToArray()
        ));
        Serialization.PrintValue(result);
    }

    // --- unregister ---

    private static async Task HandleUnregister(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        if (args.Length < 1)
        {
            Console.WriteLine("Usage: webhooks unregister <webhook_id>");
            return;
        }

        await sdk.UnregisterWebhook(request: new UnregisterWebhookRequest(webhookId: args[0]));
        Console.WriteLine("Webhook unregistered successfully");
    }

    // --- list ---

    private static async Task HandleList(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var webhooks = await sdk.ListWebhooks();
        Serialization.PrintValue(webhooks);
    }
}
