using Breez.Sdk.Spark;

namespace BreezCli;

/// <summary>
/// Represents a single contacts subcommand.
/// </summary>
public class ContactCliCommand
{
    public required string Name { get; init; }
    public required string Description { get; init; }
    public required Func<BreezSdk, Func<string, string?>, string[], Task> Run { get; init; }
}

/// <summary>
/// Contacts subcommand names (used for REPL completion).
/// </summary>
public static class ContactCommandNames
{
    public static readonly string[] All =
    {
        "contacts add",
        "contacts update",
        "contacts delete",
        "contacts list",
    };
}

/// <summary>
/// Contact subcommand handlers.
/// </summary>
public static class ContactCommands
{
    /// <summary>
    /// Builds the contacts subcommand registry.
    /// </summary>
    public static Dictionary<string, ContactCliCommand> BuildRegistry()
    {
        return new Dictionary<string, ContactCliCommand>
        {
            ["add"] = new()
            {
                Name = "add",
                Description = "Add a new contact",
                Run = HandleAdd
            },
            ["update"] = new()
            {
                Name = "update",
                Description = "Update an existing contact",
                Run = HandleUpdate
            },
            ["delete"] = new()
            {
                Name = "delete",
                Description = "Delete a contact",
                Run = HandleDelete
            },
            ["list"] = new()
            {
                Name = "list",
                Description = "List contacts",
                Run = HandleList
            },
        };
    }

    /// <summary>
    /// Dispatches a contacts subcommand.
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
            Console.WriteLine("Contacts subcommands:");
            var names = registry.Keys.OrderBy(k => k).ToList();
            foreach (var name in names)
            {
                Console.WriteLine($"  contacts {name,-30} {registry[name].Description}");
            }
            Console.WriteLine();
            return;
        }

        var subName = args[0];
        var subArgs = args.Skip(1).ToArray();

        if (!registry.TryGetValue(subName, out var cmd))
        {
            Console.WriteLine($"Unknown contacts subcommand: {subName}. Use 'contacts help' for available commands.");
            return;
        }

        await cmd.Run(sdk, readline, subArgs);
    }

    // -----------------------------------------------------------------------
    // Contacts command handlers
    // -----------------------------------------------------------------------

    // --- add ---

    private static async Task HandleAdd(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        if (args.Length < 2)
        {
            Console.WriteLine("Usage: contacts add <name> <payment_identifier>");
            return;
        }

        var result = await sdk.AddContact(request: new AddContactRequest(
            name: args[0],
            paymentIdentifier: args[1]
        ));
        Serialization.PrintValue(result);
    }

    // --- update ---

    private static async Task HandleUpdate(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        if (args.Length < 3)
        {
            Console.WriteLine("Usage: contacts update <id> <name> <payment_identifier>");
            return;
        }

        var result = await sdk.UpdateContact(request: new UpdateContactRequest(
            id: args[0],
            name: args[1],
            paymentIdentifier: args[2]
        ));
        Serialization.PrintValue(result);
    }

    // --- delete ---

    private static async Task HandleDelete(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        if (args.Length < 1)
        {
            Console.WriteLine("Usage: contacts delete <id>");
            return;
        }

        await sdk.DeleteContact(id: args[0]);
        Console.WriteLine("Contact deleted successfully");
    }

    // --- list ---

    private static async Task HandleList(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        uint? offset = null;
        uint? limit = null;

        if (args.Length >= 1)
        {
            offset = uint.Parse(args[0]);
        }
        if (args.Length >= 2)
        {
            limit = uint.Parse(args[1]);
        }

        var result = await sdk.ListContacts(request: new ListContactsRequest(
            offset: offset,
            limit: limit
        ));
        Serialization.PrintValue(result);
    }
}
