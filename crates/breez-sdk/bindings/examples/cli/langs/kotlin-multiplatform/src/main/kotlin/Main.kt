import breez_sdk_spark.*
import kotlinx.coroutines.runBlocking
import org.jline.reader.EndOfFileException
import org.jline.reader.LineReader
import org.jline.reader.LineReaderBuilder
import org.jline.reader.UserInterruptException
import org.jline.reader.impl.completer.StringsCompleter
import org.jline.terminal.TerminalBuilder
import java.io.File
import java.nio.file.Paths

/**
 * CLI event listener that logs SDK events as JSON.
 */
class CliEventListener : EventListener {
    override suspend fun onEvent(e: SdkEvent) {
        println("Event: ${serialize(e)}")
    }
}

/**
 * Splits a command line into arguments, handling double-quoted strings.
 */
fun splitArgs(line: String): List<String> {
    val args = mutableListOf<String>()
    val current = StringBuilder()
    var inQuote = false

    for (ch in line) {
        when {
            ch == '"' -> inQuote = !inQuote
            ch == ' ' && !inQuote -> {
                if (current.isNotEmpty()) {
                    args.add(current.toString())
                    current.clear()
                }
            }
            else -> current.append(ch)
        }
    }
    if (current.isNotEmpty()) {
        args.add(current.toString())
    }
    return args
}

/**
 * Expands a leading ~/ to the user's home directory.
 */
fun expandPath(path: String): String {
    if (path.startsWith("~/")) {
        val home = System.getProperty("user.home")
            ?: error("Could not find home directory")
        return File(home, path.substring(2)).absolutePath
    }
    return path
}

fun main(args: Array<String>) {
    var dataDir = "./.data"
    var network = "regtest"
    var accountNumber: String? = null
    var postgresConnectionString: String? = null
    var stableBalanceTokenIdentifier: String? = null
    var stableBalanceThreshold: ULong? = null
    var passkeyProviderStr: String? = null
    var label: String? = null
    var listLabels = false
    var storeLabel = false
    var rpId: String? = null

    // Simple argument parsing
    var i = 0
    while (i < args.size) {
        when (args[i]) {
            "-d", "--data-dir" -> {
                i++
                if (i < args.size) dataDir = args[i]
            }
            "--network" -> {
                i++
                if (i < args.size) network = args[i]
            }
            "--account-number" -> {
                i++
                if (i < args.size) accountNumber = args[i]
            }
            "--postgres-connection-string" -> {
                i++
                if (i < args.size) postgresConnectionString = args[i]
            }
            "--stable-balance-token-identifier" -> {
                i++
                if (i < args.size) stableBalanceTokenIdentifier = args[i]
            }
            "--stable-balance-threshold" -> {
                i++
                if (i < args.size) stableBalanceThreshold = args[i].toULongOrNull()
            }
            "--passkey" -> {
                i++
                if (i < args.size) passkeyProviderStr = args[i]
            }
            "--label" -> {
                i++
                if (i < args.size) label = args[i]
            }
            "--list-labels" -> {
                listLabels = true
            }
            "--store-label" -> {
                storeLabel = true
            }
            "--rpid" -> {
                i++
                if (i < args.size) rpId = args[i]
            }
            "--help", "-h" -> {
                println("Usage: breez-sdk-spark-cli [OPTIONS]")
                println()
                println("Options:")
                println("  -d, --data-dir <DIR>                         Path to the data directory (default: ./.data)")
                println("  --network <NETWORK>                          Network to use: regtest, mainnet (default: regtest)")
                println("  --account-number <NUM>                       Account number for the Spark signer")
                println("  --postgres-connection-string <CONN>          PostgreSQL connection string (uses SQLite by default)")
                println("  --stable-balance-token-identifier <ID>       Stable balance token identifier")
                println("  --stable-balance-threshold <SATS>            Stable balance threshold in sats")
                println("  --passkey <PROVIDER>                         Use passkey with PRF provider (file, yubikey, or fido2)")
                println("  --label <NAME>                               Label for seed derivation (requires --passkey)")
                println("  --list-labels                                List and select from labels on Nostr (requires --passkey)")
                println("  --store-label                                Publish the label to Nostr (requires --passkey and --label)")
                println("  --rpid <ID>                                  Relying party ID for FIDO2 provider (requires --passkey)")
                println("  -h, --help                                   Show this help message")
                return
            }
            else -> {
                System.err.println("Unknown option: ${args[i]}")
                return
            }
        }
        i++
    }

    // Validate flag combinations
    if (passkeyProviderStr == null) {
        if (label != null || listLabels || storeLabel || rpId != null) {
            System.err.println("Error: --label, --list-labels, --store-label, and --rpid require --passkey")
            return
        }
    }
    if (storeLabel && label == null) {
        System.err.println("Error: --store-label requires --label")
        return
    }
    if (listLabels && (label != null || storeLabel)) {
        System.err.println("Error: --list-labels conflicts with --label and --store-label")
        return
    }

    val resolvedDir = expandPath(dataDir)
    File(resolvedDir).mkdirs()

    val networkEnum = when (network.lowercase()) {
        "regtest" -> Network.REGTEST
        "mainnet" -> Network.MAINNET
        else -> {
            System.err.println("Invalid network. Use 'regtest' or 'mainnet'")
            return
        }
    }

    // Build stable balance config
    val stableBalanceConfig = if (stableBalanceTokenIdentifier != null) {
        StableBalanceConfig(
            tokenIdentifier = stableBalanceTokenIdentifier,
            thresholdSats = stableBalanceThreshold,
            maxSlippageBps = null,
            reservedSats = null,
        )
    } else null

    // Build passkey config
    val passkeyConfig = if (passkeyProviderStr != null) {
        try {
            val provider = PasskeyProvider.fromString(passkeyProviderStr)
            PasskeyConfig(
                provider = provider,
                label = label,
                listLabels = listLabels,
                storeLabel = storeLabel,
                rpId = rpId,
            )
        } catch (e: IllegalArgumentException) {
            System.err.println("Error: ${e.message}")
            return
        }
    } else null

    runBlocking {
        runInteractiveMode(
            resolvedDir,
            networkEnum,
            accountNumber,
            postgresConnectionString,
            stableBalanceConfig,
            passkeyConfig,
        )
    }
}

suspend fun runInteractiveMode(
    dataDir: String,
    network: Network,
    accountNumber: String?,
    postgresConnectionString: String?,
    stableBalanceConfig: StableBalanceConfig?,
    passkeyConfig: PasskeyConfig?,
) {
    // Init logging
    try {
        initLogging(dataDir, null, null)
    } catch (e: Exception) {
        System.err.println("Warning: Failed to init logging: ${e.message}")
    }

    // Persistence
    val persistence = CliPersistence(dataDir)

    // Config
    val config = defaultConfig(network)
    val apiKey = System.getenv("BREEZ_API_KEY")
    if (!apiKey.isNullOrEmpty()) {
        config.apiKey = apiKey
    }
    config.stableBalanceConfig = stableBalanceConfig

    // Resolve seed: passkey or mnemonic
    val seed: Seed = if (passkeyConfig != null) {
        val prfProvider = buildPrfProvider(passkeyConfig.provider, dataDir, passkeyConfig.rpId)
        resolvePasskeySeed(
            prfProvider,
            if (!apiKey.isNullOrEmpty()) apiKey else null,
            passkeyConfig.label,
            passkeyConfig.listLabels,
            passkeyConfig.storeLabel,
        )
    } else {
        val mnemonic = persistence.getOrCreateMnemonic()
        Seed.Mnemonic(mnemonic, null)
    }

    // Build SDK
    val builder = SdkBuilder(config, seed)
    if (postgresConnectionString != null) {
        val pgConfig = defaultPostgresStorageConfig(postgresConnectionString)
        builder.withPostgresStorage(pgConfig)
    } else {
        builder.withDefaultStorage(dataDir)
    }
    if (accountNumber != null) {
        val acctNum = accountNumber.toUIntOrNull()
            ?: error("Invalid account number: $accountNumber")
        builder.withKeySet(
            KeySetConfig(
                keySetType = KeySetType.DEFAULT,
                useAddressIndex = false,
                accountNumber = acctNum,
            )
        )
    }

    val sdk = builder.build()

    // Event listener
    sdk.addEventListener(CliEventListener())

    // Token issuer
    val tokenIssuer = sdk.getTokenIssuer()

    // Build command registries
    val commandRegistry = buildCommandRegistry()
    val issuerRegistry = buildIssuerRegistry()
    val contactsRegistry = buildContactsRegistry()

    // Build completion list
    val allCommands = mutableListOf<String>()
    allCommands.addAll(COMMAND_NAMES)
    allCommands.addAll(ISSUER_COMMAND_NAMES.map { "issuer $it" })
    allCommands.add("issuer")
    allCommands.addAll(CONTACTS_COMMAND_NAMES.map { "contacts $it" })
    allCommands.add("contacts")
    allCommands.addAll(listOf("exit", "quit", "help"))

    val networkLabel = when (network) {
        Network.MAINNET -> "mainnet"
        Network.REGTEST -> "regtest"
    }
    val prompt = "breez-spark-cli [$networkLabel]> "

    // Set up JLine3 terminal and line reader
    val terminal = TerminalBuilder.builder()
        .system(true)
        .build()

    val lineReader = LineReaderBuilder.builder()
        .terminal(terminal)
        .completer(StringsCompleter(allCommands))
        .variable(LineReader.HISTORY_FILE, Paths.get(persistence.historyFile()))
        .build()

    println("Breez SDK CLI Interactive Mode")
    println("Type 'help' for available commands or 'exit' to quit")

    loop@ while (true) {
        val line: String
        try {
            line = lineReader.readLine(prompt)
        } catch (e: UserInterruptException) {
            println("CTRL-C")
            break
        } catch (e: EndOfFileException) {
            println("CTRL-D")
            break
        }

        val trimmed = line.trim()
        if (trimmed.isEmpty()) continue

        if (trimmed == "exit" || trimmed == "quit") {
            break
        }

        if (trimmed == "help") {
            printHelp(commandRegistry, issuerRegistry, contactsRegistry)
            continue
        }

        val parts = splitArgs(trimmed)
        val cmdName = parts[0]
        val cmdArgs = parts.drop(1)

        try {
            when (cmdName) {
                "issuer" -> dispatchIssuerCommand(cmdArgs, tokenIssuer, issuerRegistry, lineReader)
                "contacts" -> dispatchContactsCommand(cmdArgs, sdk, contactsRegistry, lineReader)
                else -> {
                    val cmd = commandRegistry[cmdName]
                    if (cmd != null) {
                        cmd.run(sdk, lineReader, cmdArgs)
                    } else {
                        println("Unknown command: $cmdName. Type 'help' for available commands.")
                    }
                }
            }
        } catch (e: Exception) {
            println("Error: ${e.message}")
        }
    }

    try {
        sdk.disconnect()
    } catch (e: Exception) {
        System.err.println("Warning: disconnect error: ${e.message}")
    }

    println("Goodbye!")
}

fun printHelp(
    commandRegistry: Map<String, CliCommand>,
    issuerRegistry: Map<String, IssuerCliCommand>,
    contactsRegistry: Map<String, ContactsCliCommand>
) {
    println()
    println("Available commands:")
    commandRegistry.keys.sorted().forEach { name ->
        val cmd = commandRegistry[name]!!
        println("  %-40s %s".format(name, cmd.description))
    }
    println("  %-40s %s".format("issuer <subcommand>", "Token issuer commands (use 'issuer help' for details)"))
    println("  %-40s %s".format("contacts <subcommand>", "Contacts commands (use 'contacts help' for details)"))
    println("  %-40s %s".format("exit / quit", "Exit the CLI"))
    println("  %-40s %s".format("help", "Show this help message"))
    println()
}
