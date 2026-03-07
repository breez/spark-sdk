import Foundation
import BreezSdkSpark
import CEditLine

// MARK: - CLI flags

struct CliOptions {
    var dataDir: String = "./.data"
    var network: String = "regtest"
    var accountNumber: UInt32?
    var postgresConnectionString: String?
    var stableBalanceTokenIdentifier: String?
    var stableBalanceThreshold: UInt64?
    var passkey: String?
    var walletName: String?
    var listWalletNames: Bool = false
    var storeWalletName: Bool = false
    var rpid: String?
}

func parseCliFlags() -> CliOptions {
    var opts = CliOptions()
    let args = CommandLine.arguments
    var i = 1
    while i < args.count {
        switch args[i] {
        case "-d", "--data-dir":
            i += 1
            if i < args.count { opts.dataDir = args[i] }
        case "--network":
            i += 1
            if i < args.count { opts.network = args[i] }
        case "--account-number":
            i += 1
            if i < args.count { opts.accountNumber = UInt32(args[i]) }
        case "--postgres-connection-string":
            i += 1
            if i < args.count { opts.postgresConnectionString = args[i] }
        case "--stable-balance-token-identifier":
            i += 1
            if i < args.count { opts.stableBalanceTokenIdentifier = args[i] }
        case "--stable-balance-threshold":
            i += 1
            if i < args.count { opts.stableBalanceThreshold = UInt64(args[i]) }
        case "--passkey":
            i += 1
            if i < args.count { opts.passkey = args[i] }
        case "--wallet-name":
            i += 1
            if i < args.count { opts.walletName = args[i] }
        case "--list-wallet-names":
            opts.listWalletNames = true
        case "--store-wallet-name":
            opts.storeWalletName = true
        case "--rpid":
            i += 1
            if i < args.count { opts.rpid = args[i] }
        default:
            break
        }
        i += 1
    }
    return opts
}

// MARK: - Path expansion

func expandPath(_ path: String) -> String {
    if path.hasPrefix("~/") {
        return NSString(string: path).expandingTildeInPath
    }
    return path
}

// MARK: - Argument splitting (shell-like)

func splitArgs(_ line: String) -> [String] {
    var args: [String] = []
    var current = ""
    var inQuote = false

    for ch in line {
        switch ch {
        case "\"":
            inQuote.toggle()
        case " " where !inQuote:
            if !current.isEmpty {
                args.append(current)
                current = ""
            }
        default:
            current.append(ch)
        }
    }
    if !current.isEmpty {
        args.append(current)
    }
    return args
}

// MARK: - Event listener

/// True while libedit's `readline()` is blocking for input.
var readlineActive = false

class CliEventListener: EventListener {
    func onEvent(event: SdkEvent) {
        let msg = "Event: \(serialize(event))"
        if readlineActive {
            // Clear current line (prompt + any typed text), print event above,
            // then ask libedit to re-display the prompt and partial input.
            FileHandle.standardError.write(Data("\r\u{1b}[K\(msg)\n".utf8))
            rl_forced_update_display()
        } else {
            // Command is running — just print the event normally.
            FileHandle.standardError.write(Data("\(msg)\n".utf8))
        }
    }
}

// MARK: - Readline (libedit) with history and tab completion

/// All command names for tab completion (populated before REPL starts).
var allCompletionCommands: [String] = []

/// Single-match generator for rl_completion_matches. Returns one match per call.
func completionEntryGenerator(_ text: UnsafePointer<CChar>?, _ state: Int32) -> UnsafeMutablePointer<CChar>? {
    struct Static {
        static var matches: [String] = []
        static var index = 0
    }
    if state == 0 {
        let prefix = text.map { String(cString: $0) } ?? ""
        Static.matches = allCompletionCommands.filter { $0.hasPrefix(prefix) }
        Static.index = 0
    }
    guard Static.index < Static.matches.count else { return nil }
    let match = Static.matches[Static.index]
    Static.index += 1
    return strdup(match)
}

/// Attempted completion callback for libedit. Only completes command names at start of line.
func attemptedCompletion(_ text: UnsafePointer<CChar>?, _ start: Int32, _ end: Int32) -> UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>? {
    // Only complete command names (at the beginning of the line)
    if start == 0 {
        return rl_completion_matches(text, completionEntryGenerator)
    }
    return nil
}

/// Reads a line using libedit (with history and editing support).
func editlineRead(_ prompt: String) -> String? {
    guard let cLine = readline(prompt) else { return nil }
    defer { free(cLine) }
    let line = String(cString: cLine)
    if !line.trimmingCharacters(in: .whitespaces).isEmpty {
        add_history(cLine)
    }
    return line
}

func readlinePrompt(_ prompt: String) -> String? {
    editlineRead(prompt)
}

func readlineWithDefault(_ prompt: String, defaultValue: String) -> String {
    let line = editlineRead(prompt) ?? ""
    return line.trimmingCharacters(in: .whitespaces).isEmpty ? defaultValue : line
}

// MARK: - Main

let opts = parseCliFlags()
let resolvedDir = expandPath(opts.dataDir)

// Create data directory
try FileManager.default.createDirectory(atPath: resolvedDir, withIntermediateDirectories: true)

// Parse network
let network: Network
switch opts.network.lowercased() {
case "regtest":
    network = .regtest
case "mainnet":
    network = .mainnet
default:
    print("Invalid network. Use 'regtest' or 'mainnet'")
    exit(1)
}

// Init logging
try initLogging(logDir: resolvedDir, appLogger: nil, logFilter: nil)

// Persistence
let persistence = CliPersistence(dataDir: resolvedDir)

// Config
var config = defaultConfig(network: network)
let breezApiKey: String? = {
    if let key = ProcessInfo.processInfo.environment["BREEZ_API_KEY"], !key.isEmpty {
        return key
    }
    return nil
}()
config.apiKey = breezApiKey
if let tokenIdentifier = opts.stableBalanceTokenIdentifier {
    config.stableBalanceConfig = StableBalanceConfig(
        tokenIdentifier: tokenIdentifier,
        thresholdSats: opts.stableBalanceThreshold,
        maxSlippageBps: nil,
        reservedSats: nil
    )
}

// Resolve seed (passkey or mnemonic)
let seed: Seed
if let passkeyStr = opts.passkey {
    guard let providerType = PasskeyProviderType(rawValue: passkeyStr.lowercased()) else {
        print("Invalid passkey provider '\(passkeyStr)'. Use 'file', 'yubikey', or 'fido2'.")
        exit(1)
    }
    let prfProvider = try createPrfProvider(type: providerType, dataDir: resolvedDir)
    seed = try await resolvePasskeySeed(
        provider: prfProvider,
        breezApiKey: breezApiKey,
        walletName: opts.walletName,
        listWalletNames: opts.listWalletNames,
        storeWalletName: opts.storeWalletName
    )
} else {
    let mnemonic = try persistence.getOrCreateMnemonic()
    seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: nil)
}

// Build SDK
let builder = SdkBuilder(config: config, seed: seed)
if let connectionString = opts.postgresConnectionString {
    await builder.withPostgresStorage(config: defaultPostgresStorageConfig(connectionString: connectionString))
} else {
    await builder.withDefaultStorage(storageDir: resolvedDir)
}
if let accountNumber = opts.accountNumber {
    await builder.withKeySet(config: KeySetConfig(
        keySetType: .default,
        useAddressIndex: false,
        accountNumber: accountNumber
    ))
}

let sdk = try await builder.build()

// Event listener
_ = await sdk.addEventListener(listener: CliEventListener())

// Token issuer
let tokenIssuer = sdk.getTokenIssuer()

// Build command registry
let registry = buildCommandRegistry()

// Set up tab completion
allCompletionCommands = commandNames + issuerCommandNames + contactsCommandNames + ["help", "exit", "quit"]
rl_attempted_completion_function = attemptedCompletion

// Load history
let historyPath = persistence.historyFile()
read_history(historyPath)

// REPL
print("Breez SDK CLI Interactive Mode")
print("Type 'help' for available commands or 'exit' to quit")

let networkLabel = network == .mainnet ? "mainnet" : "regtest"
let promptStr = "breez-spark-cli [\(networkLabel)]> "

replLoop: while true {
    readlineActive = true
    guard let line = editlineRead(promptStr) else {
        readlineActive = false
        // EOF (Ctrl-D)
        print("\nCTRL-D")
        break
    }
    readlineActive = false

    let trimmed = line.trimmingCharacters(in: .whitespaces)
    if trimmed.isEmpty { continue }

    persistence.appendHistory(trimmed)

    if trimmed == "exit" || trimmed == "quit" {
        break
    }

    if trimmed == "help" {
        printHelp(registry)
        continue
    }

    let args = splitArgs(trimmed)
    let cmdName = args[0]
    let cmdArgs = Array(args.dropFirst())

    if cmdName == "issuer" {
        await dispatchIssuerCommand(cmdArgs, tokenIssuer: tokenIssuer)
    } else if cmdName == "contacts" {
        await dispatchContactsCommand(cmdArgs, sdk: sdk)
    } else if let cmd = registry[cmdName] {
        do {
            try await cmd.run(sdk, cmdArgs)
        } catch {
            print("Error: \(error)")
        }
    } else {
        print("Unknown command: \(cmdName). Type 'help' for available commands.")
    }
}

// Save history
write_history(historyPath)

// Cleanup
do {
    try await sdk.disconnect()
} catch {
    FileHandle.standardError.write(Data("Warning: disconnect error: \(error)\n".utf8))
}

print("Goodbye!")
exit(0)
