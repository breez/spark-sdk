import Foundation
import BreezSdkSpark
import BigNumber

// MARK: - Issuer command names (for REPL completion)

let issuerCommandNames: [String] = [
    "issuer token-balance",
    "issuer token-metadata",
    "issuer create-token",
    "issuer mint-token",
    "issuer burn-token",
    "issuer freeze-token",
    "issuer unfreeze-token",
]

// MARK: - Dispatch

func dispatchIssuerCommand(_ args: [String], tokenIssuer: TokenIssuer) async {
    if args.isEmpty || args[0] == "help" {
        printIssuerHelp()
        return
    }

    let subName = args[0]
    let subArgs = Array(args.dropFirst())

    do {
        switch subName {
        case "token-balance":
            try await handleTokenBalance(tokenIssuer, subArgs)
        case "token-metadata":
            try await handleTokenMetadata(tokenIssuer, subArgs)
        case "create-token":
            try await handleCreateToken(tokenIssuer, subArgs)
        case "mint-token":
            try await handleMintToken(tokenIssuer, subArgs)
        case "burn-token":
            try await handleBurnToken(tokenIssuer, subArgs)
        case "freeze-token":
            try await handleFreezeToken(tokenIssuer, subArgs)
        case "unfreeze-token":
            try await handleUnfreezeToken(tokenIssuer, subArgs)
        default:
            print("Unknown issuer subcommand: \(subName). Use 'issuer help' for available commands.")
        }
    } catch {
        print("Error: \(error)")
    }
}

// MARK: - Help

private func printIssuerHelp() {
    print("\nIssuer subcommands:")
    print("  issuer \("token-balance".padding(toLength: 30, withPad: " ", startingAt: 0))Get issuer token balance")
    print("  issuer \("token-metadata".padding(toLength: 30, withPad: " ", startingAt: 0))Get issuer token metadata")
    print("  issuer \("create-token".padding(toLength: 30, withPad: " ", startingAt: 0))Create a new issuer token")
    print("  issuer \("mint-token".padding(toLength: 30, withPad: " ", startingAt: 0))Mint supply of the issuer token")
    print("  issuer \("burn-token".padding(toLength: 30, withPad: " ", startingAt: 0))Burn supply of the issuer token")
    print("  issuer \("freeze-token".padding(toLength: 30, withPad: " ", startingAt: 0))Freeze tokens at an address")
    print("  issuer \("unfreeze-token".padding(toLength: 30, withPad: " ", startingAt: 0))Unfreeze tokens at an address")
    print()
}

// MARK: - Handlers

// --- token-balance ---

private func handleTokenBalance(_ issuer: TokenIssuer, _ args: [String]) async throws {
    let result = try await issuer.getIssuerTokenBalance()
    printValue(result)
}

// --- token-metadata ---

private func handleTokenMetadata(_ issuer: TokenIssuer, _ args: [String]) async throws {
    let result = try await issuer.getIssuerTokenMetadata()
    printValue(result)
}

// --- create-token ---

private func handleCreateToken(_ issuer: TokenIssuer, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard let name = fp.get("name"),
          let ticker = fp.get("ticker") else {
        print("Usage: issuer create-token --name <name> --ticker <ticker> [--decimals N] [--freezable] [--max-supply N]")
        return
    }

    let decimals = fp.get("decimals").flatMap { UInt32($0) } ?? 6
    let freezable = fp.has("freezable", "f")
    let maxSupply: BInt = fp.get("max-supply").flatMap { BInt($0) } ?? BInt(0)

    let result = try await issuer.createIssuerToken(request: CreateIssuerTokenRequest(
        name: name,
        ticker: ticker,
        decimals: decimals,
        isFreezable: freezable,
        maxSupply: maxSupply
    ))
    printValue(result)
}

// --- mint-token ---

private func handleMintToken(_ issuer: TokenIssuer, _ args: [String]) async throws {
    guard !args.isEmpty, let amount = BInt(args[0]) else {
        print("Usage: issuer mint-token <amount>")
        return
    }

    let result = try await issuer.mintIssuerToken(request: MintIssuerTokenRequest(
        amount: amount
    ))
    printValue(result)
}

// --- burn-token ---

private func handleBurnToken(_ issuer: TokenIssuer, _ args: [String]) async throws {
    guard !args.isEmpty, let amount = BInt(args[0]) else {
        print("Usage: issuer burn-token <amount>")
        return
    }

    let result = try await issuer.burnIssuerToken(request: BurnIssuerTokenRequest(
        amount: amount
    ))
    printValue(result)
}

// --- freeze-token ---

private func handleFreezeToken(_ issuer: TokenIssuer, _ args: [String]) async throws {
    guard !args.isEmpty else {
        print("Usage: issuer freeze-token <address>")
        return
    }

    let result = try await issuer.freezeIssuerToken(request: FreezeIssuerTokenRequest(
        address: args[0]
    ))
    printValue(result)
}

// --- unfreeze-token ---

private func handleUnfreezeToken(_ issuer: TokenIssuer, _ args: [String]) async throws {
    guard !args.isEmpty else {
        print("Usage: issuer unfreeze-token <address>")
        return
    }

    let result = try await issuer.unfreezeIssuerToken(request: UnfreezeIssuerTokenRequest(
        address: args[0]
    ))
    printValue(result)
}
