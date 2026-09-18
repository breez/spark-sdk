import Foundation
import BreezSdkSpark
import CEditLine

// MARK: - CLI flags

struct CliOptions {
    var dataDir: String = "./.data"
    var network: String = "regtest"
    var sparkConfig: String?
    var chainApiUrl: String?
    var chainApiType: String?
    var accountNumber: UInt32?
    var postgresConnectionString: String?
    var mysqlConnectionString: String?
    var stableBalanceTokens: [String] = []
    var stableBalanceDefaultActiveLabel: String?
    var stableBalanceThreshold: UInt64?
    var passkey: String?
    var label: String?
    var listLabels: Bool = false
    var storeLabel: Bool = false
    var rpid: String?
    var serverMode: Bool = false
    var lnurlDomain: String?
    var proxy: String?
    var proxyUser: String?
    var proxyPassword: String?
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
        case "--spark-config":
            i += 1
            if i < args.count { opts.sparkConfig = args[i] }
        case "--chain-api-url":
            i += 1
            if i < args.count { opts.chainApiUrl = args[i] }
        case "--chain-api-type":
            i += 1
            if i < args.count { opts.chainApiType = args[i] }
        case "--account-number":
            i += 1
            if i < args.count { opts.accountNumber = UInt32(args[i]) }
        case "--postgres-connection-string":
            i += 1
            if i < args.count { opts.postgresConnectionString = args[i] }
        case "--mysql-connection-string":
            i += 1
            if i < args.count { opts.mysqlConnectionString = args[i] }
        case "--stable-balance-token":
            i += 1
            if i < args.count { opts.stableBalanceTokens.append(args[i]) }
        case "--stable-balance-default-active-label":
            i += 1
            if i < args.count { opts.stableBalanceDefaultActiveLabel = args[i] }
        case "--stable-balance-threshold":
            i += 1
            if i < args.count { opts.stableBalanceThreshold = UInt64(args[i]) }
        case "--passkey":
            i += 1
            if i < args.count { opts.passkey = args[i] }
        case "--label":
            i += 1
            if i < args.count { opts.label = args[i] }
        case "--list-labels":
            opts.listLabels = true
        case "--store-label":
            opts.storeLabel = true
        case "--rpid":
            i += 1
            if i < args.count { opts.rpid = args[i] }
        case "--server-mode":
            opts.serverMode = true
        case "--lnurl-domain":
            i += 1
            if i < args.count { opts.lnurlDomain = args[i] }
        case "--proxy":
            i += 1
            if i < args.count { opts.proxy = args[i] }
        case "--proxy-user":
            i += 1
            if i < args.count { opts.proxyUser = args[i] }
        case "--proxy-password":
            i += 1
            if i < args.count { opts.proxyPassword = args[i] }
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

/// Parses `HOST:PORT` into a `ProxyConfig`. Splits from the right so an
/// IPv6 literal keeps its colons.
func parseProxy(address: String, username: String?, password: String?) -> ProxyConfig {
    guard let lastColon = address.lastIndex(of: ":") else {
        print("Invalid proxy '\(address)', expected HOST:PORT")
        exit(1)
    }
    let host = String(address[address.startIndex..<lastColon])
    let portStr = String(address[address.index(after: lastColon)...])
    guard let port = UInt16(portStr) else {
        print("Invalid proxy port '\(portStr)'")
        exit(1)
    }
    return ProxyConfig(host: host, port: port, username: username, password: password)
}

func parseChainApiType(_ value: String) -> ChainApiType? {
    switch value {
    case "esplora":
        return .esplora
    case "mempool-space":
        return .mempoolSpace
    default:
        return nil
    }
}

func loadSparkConfig(path: String) throws -> SparkConfig {
    let data = try Data(contentsOf: URL(fileURLWithPath: path))
    guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
        throw NSError(domain: "CLI", code: 1, userInfo: [
            NSLocalizedDescriptionKey: "Failed to parse Spark config \(path)",
        ])
    }
    guard let coordinatorIdentifier = json["coordinator_identifier"] as? String,
          let threshold = json["threshold"] as? Int,
          let opsArray = json["signing_operators"] as? [[String: Any]],
          let sspDict = json["ssp_config"] as? [String: Any],
          let sspBaseUrl = sspDict["base_url"] as? String,
          let sspIdentityPubkey = sspDict["identity_public_key"] as? String
    else {
        throw NSError(domain: "CLI", code: 1, userInfo: [
            NSLocalizedDescriptionKey: "Missing required fields in Spark config \(path)",
        ])
    }
    let operators = try opsArray.map { op -> SparkSigningOperator in
        guard let id = op["id"] as? Int,
              let identifier = op["identifier"] as? String,
              let address = op["address"] as? String,
              let identityPubkey = op["identity_public_key"] as? String
        else {
            throw NSError(domain: "CLI", code: 1, userInfo: [
                NSLocalizedDescriptionKey: "Invalid signing operator in Spark config \(path)",
            ])
        }
        return SparkSigningOperator(
            id: UInt32(id),
            identifier: identifier,
            address: address,
            identityPublicKey: identityPubkey,
            caCertPem: op["ca_cert_pem"] as? String
        )
    }
    let sspConfig = SparkSspConfig(
        baseUrl: sspBaseUrl,
        identityPublicKey: sspIdentityPubkey,
        schemaEndpoint: sspDict["schema_endpoint"] as? String
    )
    return SparkConfig(
        coordinatorIdentifier: coordinatorIdentifier,
        threshold: UInt32(threshold),
        signingOperators: operators,
        sspConfig: sspConfig,
        expectedWithdrawBondSats: (json["expected_withdraw_bond_sats"] as? Int).map { UInt64($0) } ?? 0,
        expectedWithdrawRelativeBlockLocktime: (json["expected_withdraw_relative_block_locktime"] as? Int).map { UInt64($0) } ?? 0,
        maxTokenTransactionInputs: (json["max_token_transaction_inputs"] as? Int).map { UInt32($0) }
    )
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
            // Command is running: just print the event normally.
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

/// Reads a line using libedit (with history and editing support). On piped
/// (non-TTY) stdin libedit echoes input with redraw escapes that corrupt the
/// output stream, so plain line reads are used instead.
func editlineRead(_ prompt: String) -> String? {
    if isatty(fileno(stdin)) == 0 {
        return readLine(strippingNewline: true)
    }
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
case "signet":
    network = .signet
default:
    print("Invalid network. Use 'regtest', 'signet', or 'mainnet'")
    exit(1)
}

// Load spark config
let sparkConfig: SparkConfig? = try opts.sparkConfig.map { try loadSparkConfig(path: $0) }

// Parse chain API options
let chainApi: (String, ChainApiType)?
if let chainApiUrl = opts.chainApiUrl {
    let apiType: ChainApiType
    if let typeStr = opts.chainApiType {
        guard let parsed = parseChainApiType(typeStr) else {
            print("Invalid chain API type '\(typeStr)'. Expected 'esplora' or 'mempool-space'")
            exit(1)
        }
        apiType = parsed
    } else {
        apiType = .esplora
    }
    chainApi = (chainApiUrl, apiType)
} else {
    chainApi = nil
}

// Init logging
try initLogging(logDir: resolvedDir, appLogger: nil, logFilter: nil)

// Persistence
let persistence = CliPersistence(dataDir: resolvedDir)

// Config
var config: Config
if opts.serverMode {
    print("Server mode enabled. Run `sync` between operations.")
    config = defaultServerConfig(network: network)
} else {
    config = defaultConfig(network: network)
}
let breezApiKey: String? = {
    if let key = ProcessInfo.processInfo.environment["BREEZ_API_KEY"], !key.isEmpty {
        return key
    }
    return nil
}()
config.apiKey = breezApiKey
if let sparkConfig = sparkConfig {
    config.sparkConfig = sparkConfig
}
if network == .mainnet {
    config.crossChainConfig = CrossChainConfig()
}
let proxy: ProxyConfig? = opts.proxy.map {
    parseProxy(address: $0, username: opts.proxyUser, password: opts.proxyPassword)
}
config.proxy = proxy
if let lnurlDomain = opts.lnurlDomain {
    config.lnurlDomain = lnurlDomain
}
if !opts.stableBalanceTokens.isEmpty {
    let tokens: [StableBalanceToken] = opts.stableBalanceTokens.map { s in
        let parts = s.split(separator: ":", maxSplits: 1)
        guard parts.count == 2 else {
            fatalError("Invalid token format '\(s)', expected LABEL:token_identifier")
        }
        return StableBalanceToken(
            label: String(parts[0]),
            tokenIdentifier: String(parts[1])
        )
    }
    config.stableBalanceConfig = StableBalanceConfig(
        tokens: tokens,
        defaultActiveLabel: opts.stableBalanceDefaultActiveLabel,
        thresholdSats: opts.stableBalanceThreshold,
        maxSlippageBps: nil
    )
}

// Resolve seed (passkey or mnemonic)
let seed: Seed
if let passkeyStr = opts.passkey {
    guard let providerType = PasskeyProviderType(rawValue: passkeyStr.lowercased()) else {
        print("Invalid passkey provider '\(passkeyStr)'. Use 'platform', 'file', 'yubikey', or 'fido2'.")
        exit(1)
    }
    let prfProvider = try createPrfProvider(type: providerType, dataDir: resolvedDir, rpId: opts.rpid)
    seed = try await resolvePasskeySeed(
        provider: prfProvider,
        breezApiKey: breezApiKey,
        label: opts.label,
        listLabels: opts.listLabels,
        storeLabel: opts.storeLabel,
        proxy: proxy
    )
} else {
    let mnemonic = try persistence.getOrCreateMnemonic()
    seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: nil)
}

// Build SDK
let builder = SdkBuilder(config: config, seed: seed)
if let (url, apiType) = chainApi {
    await builder.withRestChainService(url: url, apiType: apiType, credentials: nil)
}
if let connectionString = opts.postgresConnectionString {
    await builder.withStorageBackend(storage: try postgresStorage(
        config: defaultPostgresStorageConfig(connectionString: connectionString)
    ))
} else if let connectionString = opts.mysqlConnectionString {
    await builder.withStorageBackend(storage: try mysqlStorage(
        config: defaultMysqlStorageConfig(connectionString: connectionString)
    ))
} else {
    await builder.withDefaultStorage(storageDir: resolvedDir)
}
if let accountNumber = opts.accountNumber {
    await builder.withAccountNumber(accountNumber: accountNumber)
}

let sdk = try await builder.build()

// Event listener
_ = await sdk.addEventListener(listener: CliEventListener())

// Token issuer
let tokenIssuer = sdk.getTokenIssuer()

// Build command registry
let registry = buildCommandRegistry()

// Set up tab completion
allCompletionCommands = commandNames + advancedCommandNames + issuerCommandNames + contactsCommandNames + webhooksCommandNames + stableBalanceCommandNames + ["help", "exit", "quit"]
rl_attempted_completion_function = attemptedCompletion

// Load history
let historyPath = persistence.historyFile()
read_history(historyPath)

// REPL
print("Breez SDK CLI Interactive Mode")
print("Type 'help' for available commands or 'exit' to quit")

let networkLabel: String
switch network {
case .mainnet: networkLabel = "mainnet"
case .signet: networkLabel = "signet"
case .regtest: networkLabel = "regtest"
}
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

    if cmdName == "advanced" {
        await dispatchAdvancedCommand(cmdArgs, sdk: sdk)
    } else if cmdName == "issuer" {
        await dispatchIssuerCommand(cmdArgs, tokenIssuer: tokenIssuer)
    } else if cmdName == "contacts" {
        await dispatchContactsCommand(cmdArgs, sdk: sdk)
    } else if cmdName == "webhooks" {
        await dispatchWebhooksCommand(cmdArgs, sdk: sdk)
    } else if cmdName == "stable-balance" {
        await dispatchStableBalanceCommand(cmdArgs, sdk: sdk)
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
