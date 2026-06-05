import Foundation
import BreezSdkSpark

// MARK: - Stable balance command names (for REPL completion)

let stableBalanceCommandNames: [String] = [
    "stable-balance get",
    "stable-balance set",
    "stable-balance unset",
]

// MARK: - Dispatch

func dispatchStableBalanceCommand(_ args: [String], sdk: BreezSdk) async {
    if args.isEmpty || args[0] == "help" {
        printStableBalanceHelp()
        return
    }

    let subName = args[0]
    let subArgs = Array(args.dropFirst())

    do {
        switch subName {
        case "get":
            try await handleStableBalanceGet(sdk, subArgs)
        case "set":
            try await handleStableBalanceSet(sdk, subArgs)
        case "unset":
            try await handleStableBalanceUnset(sdk, subArgs)
        default:
            print("Unknown stable-balance subcommand: \(subName). Use 'stable-balance help' for available commands.")
        }
    } catch {
        print("Error: \(error)")
    }
}

// MARK: - Help

private func printStableBalanceHelp() {
    print("\nStable balance subcommands:")
    print("  stable-balance \("get".padding(toLength: 22, withPad: " ", startingAt: 0))Get the stable balance active label")
    print("  stable-balance \("set".padding(toLength: 22, withPad: " ", startingAt: 0))Set the stable balance active label")
    print("  stable-balance \("unset".padding(toLength: 22, withPad: " ", startingAt: 0))Unset stable balance")
    print()
}

// MARK: - Handlers

// --- get ---

private func handleStableBalanceGet(_ sdk: BreezSdk, _ args: [String]) async throws {
    let settings = try await sdk.getUserSettings()
    printValue(settings.stableBalanceActiveLabel)
}

// --- set ---

private func handleStableBalanceSet(_ sdk: BreezSdk, _ args: [String]) async throws {
    guard !args.isEmpty else {
        print("Usage: stable-balance set <label>")
        return
    }

    try await sdk.updateUserSettings(request: UpdateUserSettingsRequest(
        sparkPrivateModeEnabled: nil,
        stableBalanceActiveLabel: .set(label: args[0])
    ))
    let settings = try await sdk.getUserSettings()
    printValue(settings)
}

// --- unset ---

private func handleStableBalanceUnset(_ sdk: BreezSdk, _ args: [String]) async throws {
    try await sdk.updateUserSettings(request: UpdateUserSettingsRequest(
        sparkPrivateModeEnabled: nil,
        stableBalanceActiveLabel: .unset
    ))
    let settings = try await sdk.getUserSettings()
    printValue(settings)
}
