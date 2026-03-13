using Breez.Sdk.Spark;
using BreezCli;

// ---------------------------------------------------------------------------
// CLI argument parsing
// ---------------------------------------------------------------------------

string dataDir = "./.data";
string network = "regtest";
uint? accountNumber = null;
string? postgresConnectionString = null;
string? stableBalanceTokenIdentifier = null;
ulong? stableBalanceThreshold = null;
string? passkeyProviderStr = null;
string? label = null;
bool listLabels = false;
bool storeLabel = false;
string? rpId = null;

for (int i = 0; i < args.Length; i++)
{
    switch (args[i])
    {
        case "-d":
        case "--data-dir":
            if (i + 1 < args.Length) dataDir = args[++i];
            break;
        case "--network":
            if (i + 1 < args.Length) network = args[++i];
            break;
        case "--account-number":
            if (i + 1 < args.Length) accountNumber = uint.Parse(args[++i]);
            break;
        case "--postgres-connection-string":
            if (i + 1 < args.Length) postgresConnectionString = args[++i];
            break;
        case "--stable-balance-token-identifier":
            if (i + 1 < args.Length) stableBalanceTokenIdentifier = args[++i];
            break;
        case "--stable-balance-threshold":
            if (i + 1 < args.Length) stableBalanceThreshold = ulong.Parse(args[++i]);
            break;
        case "--passkey":
            if (i + 1 < args.Length) passkeyProviderStr = args[++i];
            break;
        case "--label":
            if (i + 1 < args.Length) label = args[++i];
            break;
        case "--list-labels":
            listLabels = true;
            break;
        case "--store-label":
            storeLabel = true;
            break;
        case "--rpid":
            if (i + 1 < args.Length) rpId = args[++i];
            break;
        case "--help":
        case "-h":
            PrintUsage();
            return;
    }
}

// ---------------------------------------------------------------------------
// Validate passkey flag combinations
// ---------------------------------------------------------------------------

if (listLabels && passkeyProviderStr == null)
{
    Console.Error.WriteLine("Error: --list-labels requires --passkey");
    return;
}

if (storeLabel && passkeyProviderStr == null)
{
    Console.Error.WriteLine("Error: --store-label requires --passkey");
    return;
}

if (storeLabel && label == null)
{
    Console.Error.WriteLine("Error: --store-label requires --label");
    return;
}

if (label != null && passkeyProviderStr == null)
{
    Console.Error.WriteLine("Error: --label requires --passkey");
    return;
}

if (rpId != null && passkeyProviderStr == null)
{
    Console.Error.WriteLine("Error: --rpid requires --passkey");
    return;
}

if (listLabels && (label != null || storeLabel))
{
    Console.Error.WriteLine("Error: --list-labels conflicts with --label and --store-label");
    return;
}

// ---------------------------------------------------------------------------
// Resolve data directory
// ---------------------------------------------------------------------------

string resolvedDir = ExpandPath(dataDir);
Directory.CreateDirectory(resolvedDir);

// ---------------------------------------------------------------------------
// Parse network
// ---------------------------------------------------------------------------

Network networkEnum = network.ToLower() switch
{
    "regtest" => Network.Regtest,
    "mainnet" => Network.Mainnet,
    _ => throw new ArgumentException($"Invalid network '{network}'. Use 'regtest' or 'mainnet'")
};

// ---------------------------------------------------------------------------
// Stable balance config
// ---------------------------------------------------------------------------

StableBalanceConfig? stableBalanceConfig = null;
if (stableBalanceTokenIdentifier != null)
{
    stableBalanceConfig = new StableBalanceConfig(
        tokens: new StableBalanceToken[] {
            new StableBalanceToken(
                ticker: "USDB",
                tokenIdentifier: stableBalanceTokenIdentifier
            )
        },
        defaultActiveTicker: "USDB",
        thresholdSats: stableBalanceThreshold,
        maxSlippageBps: null
    );
}

// ---------------------------------------------------------------------------
// Passkey config
// ---------------------------------------------------------------------------

PasskeyConfig? passkeyConfig = null;
if (passkeyProviderStr != null)
{
    passkeyConfig = new PasskeyConfig
    {
        Provider = PasskeyProviderExtensions.ParseProvider(passkeyProviderStr),
        Label = label,
        ListLabels = listLabels,
        StoreLabel = storeLabel,
        RpId = rpId,
    };
}

// ---------------------------------------------------------------------------
// Run interactive mode
// ---------------------------------------------------------------------------

await RunInteractiveMode(
    resolvedDir,
    networkEnum,
    accountNumber,
    postgresConnectionString,
    stableBalanceConfig,
    passkeyConfig
);

return;

// ===========================================================================
// Functions
// ===========================================================================

static string ExpandPath(string path)
{
    if (path.StartsWith("~/"))
    {
        var home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        return Path.Combine(home, path[2..]);
    }
    return Path.GetFullPath(path);
}

static void PrintUsage()
{
    Console.WriteLine("Breez SDK CLI (C# / .NET)");
    Console.WriteLine();
    Console.WriteLine("Usage: dotnet run -- [OPTIONS]");
    Console.WriteLine();
    Console.WriteLine("Options:");
    Console.WriteLine("  -d, --data-dir <PATH>                       Path to data directory (default: ./.data)");
    Console.WriteLine("  --network <NETWORK>                         Network: regtest or mainnet (default: regtest)");
    Console.WriteLine("  --account-number <N>                        Account number for the Spark signer");
    Console.WriteLine("  --postgres-connection-string <CONN>         PostgreSQL connection string (SQLite by default)");
    Console.WriteLine("  --stable-balance-token-identifier <TOKEN>   Stable balance token identifier");
    Console.WriteLine("  --stable-balance-threshold <SATS>           Stable balance threshold in sats");
    Console.WriteLine("  --passkey <PROVIDER>                        Use passkey with file, yubikey, or fido2 provider");
    Console.WriteLine("  --label <NAME>                              Label for seed derivation (requires --passkey)");
    Console.WriteLine("  --list-labels                               List and select from labels on Nostr (requires --passkey)");
    Console.WriteLine("  --store-label                               Publish label to Nostr (requires --passkey and --label)");
    Console.WriteLine("  --rpid <RPID>                               Relying party ID for FIDO2 provider (requires --passkey)");
    Console.WriteLine("  -h, --help                                  Show this help");
}

static async Task RunInteractiveMode(
    string dataDir,
    Network network,
    uint? accountNumber,
    string? postgresConnectionString,
    StableBalanceConfig? stableBalanceConfig,
    PasskeyConfig? passkeyConfig)
{
    // Init logging
    try
    {
        BreezSdkSparkMethods.InitLogging(logDir: dataDir, appLogger: null, logFilter: null);
    }
    catch
    {
        // Logging may already be initialized; ignore
    }

    // Persistence
    var persistence = new CliPersistence(dataDir);
    Directory.CreateDirectory(dataDir);

    // Config
    var config = BreezSdkSparkMethods.DefaultConfig(network);
    var apiKey = Environment.GetEnvironmentVariable("BREEZ_API_KEY");
    if (!string.IsNullOrEmpty(apiKey))
    {
        config = config with { apiKey = apiKey };
    }
    if (stableBalanceConfig != null)
    {
        config = config with { stableBalanceConfig = stableBalanceConfig };
    }

    // Resolve seed: passkey or mnemonic
    Seed seed;
    if (passkeyConfig != null)
    {
        var prfProvider = PasskeyProviderExtensions.BuildPrfProvider(
            passkeyConfig.Provider,
            dataDir,
            passkeyConfig.RpId);
        seed = await PasskeyResolver.ResolvePasskeySeed(
            prfProvider,
            apiKey,
            passkeyConfig.Label,
            passkeyConfig.ListLabels,
            passkeyConfig.StoreLabel);
    }
    else
    {
        var mnemonic = persistence.GetOrCreateMnemonic();
        seed = new Seed.Mnemonic(mnemonic: mnemonic, passphrase: null);
    }

    // Build SDK
    var builder = new SdkBuilder(config: config, seed: seed);
    if (postgresConnectionString != null)
    {
        var pgConfig = BreezSdkSparkMethods.DefaultPostgresStorageConfig(postgresConnectionString);
        await builder.WithPostgresStorage(config: pgConfig);
    }
    else
    {
        await builder.WithDefaultStorage(storageDir: dataDir);
    }

    if (accountNumber != null)
    {
        await builder.WithKeySet(new KeySetConfig(
            keySetType: KeySetType.Default,
            useAddressIndex: false,
            accountNumber: accountNumber
        ));
    }

    var sdk = await builder.Build();

    // Event listener
    var listener = new CliEventListener();
    await sdk.AddEventListener(listener: listener);

    // Token issuer
    var tokenIssuer = sdk.GetTokenIssuer();

    // Build command registry
    var registry = Commands.BuildRegistry();

    // Set up tab-completion
    var allCommands = new List<string>();
    allCommands.AddRange(CommandNames.All);
    allCommands.AddRange(IssuerCommandNames.All);
    allCommands.AddRange(ContactCommandNames.All);
    allCommands.Add("exit");
    allCommands.Add("quit");
    allCommands.Add("help");

    ReadLine.AutoCompletionHandler = new AutoCompletionHandler(allCommands);
    ReadLine.HistoryEnabled = false;

    // Load history
    var history = persistence.LoadHistory();
    foreach (var entry in history)
    {
        ReadLine.AddHistory(entry);
    }

    Console.WriteLine("Breez SDK CLI Interactive Mode");
    Console.WriteLine("Type 'help' for available commands or 'exit' to quit");

    var promptStr = network switch
    {
        Network.Mainnet => "breez-spark-cli [mainnet]> ",
        Network.Regtest => "breez-spark-cli [regtest]> ",
        _ => "breez-spark-cli> "
    };

    // Readline function for interactive prompts within commands
    string? Readline(string prompt)
    {
        return ReadLine.Read(prompt);
    }

    // REPL loop
    while (true)
    {
        string? line;
        try
        {
            line = ReadLine.Read(promptStr);
        }
        catch (Exception)
        {
            Console.WriteLine("CTRL-D");
            break;
        }

        if (line == null)
        {
            Console.WriteLine("CTRL-D");
            break;
        }

        var trimmed = line.Trim();
        if (string.IsNullOrEmpty(trimmed))
        {
            continue;
        }

        // Add to history (manual management since HistoryEnabled is off)
        ReadLine.AddHistory(trimmed);
        history.Add(trimmed);

        // Exit commands
        if (trimmed == "exit" || trimmed == "quit")
        {
            break;
        }

        // Help
        if (trimmed == "help")
        {
            Commands.PrintHelp(registry);
            continue;
        }

        // Split arguments (handle quoted strings)
        var splitArgs = SplitArgs(trimmed);
        if (splitArgs.Length == 0) continue;

        var cmdName = splitArgs[0];
        var cmdArgs = splitArgs.Skip(1).ToArray();

        try
        {
            if (cmdName == "issuer")
            {
                await IssuerCommands.DispatchCommand(cmdArgs, tokenIssuer, Readline);
            }
            else if (cmdName == "contacts")
            {
                await ContactCommands.DispatchCommand(cmdArgs, sdk, Readline);
            }
            else if (registry.TryGetValue(cmdName, out var cmd))
            {
                await cmd.Run(sdk, Readline, cmdArgs);
            }
            else
            {
                Console.WriteLine($"Unknown command: {cmdName}. Type 'help' for available commands.");
            }
        }
        catch (Exception ex)
        {
            Console.WriteLine($"Error: {ex.Message}");
        }
    }

    // Cleanup
    try
    {
        await sdk.Disconnect();
    }
    catch (Exception ex)
    {
        Console.Error.WriteLine($"Warning: disconnect error: {ex.Message}");
    }

    // Save history
    persistence.SaveHistory(history);

    Console.WriteLine("Goodbye!");
}

/// <summary>
/// Splits a command line into arguments, handling double-quoted strings.
/// </summary>
static string[] SplitArgs(string line)
{
    var args = new List<string>();
    var current = new System.Text.StringBuilder();
    bool inQuote = false;

    foreach (char c in line)
    {
        switch (c)
        {
            case '"':
                inQuote = !inQuote;
                break;
            case ' ' when !inQuote:
                if (current.Length > 0)
                {
                    args.Add(current.ToString());
                    current.Clear();
                }
                break;
            default:
                current.Append(c);
                break;
        }
    }

    if (current.Length > 0)
    {
        args.Add(current.ToString());
    }

    return args.ToArray();
}

// ===========================================================================
// Auto-completion handler for ReadLine
// ===========================================================================

class AutoCompletionHandler : IAutoCompleteHandler
{
    private readonly List<string> _commands;

    public AutoCompletionHandler(List<string> commands)
    {
        _commands = commands;
    }

    public char[] Separators { get; set; } = new char[] { ' ' };

    public string[] GetSuggestions(string text, int index)
    {
        return _commands
            .Where(c => c.StartsWith(text, StringComparison.OrdinalIgnoreCase))
            .ToArray();
    }
}

// ===========================================================================
// Event listener
// ===========================================================================

class CliEventListener : EventListener
{
    public async Task OnEvent(SdkEvent sdkEvent)
    {
        var serialized = Serialization.Serialize(sdkEvent);
        Console.WriteLine($"Event: {serialized}");
        await Task.CompletedTask;
    }
}
