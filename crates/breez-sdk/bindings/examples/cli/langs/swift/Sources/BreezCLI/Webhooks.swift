import Foundation
import BreezSdkSpark

// MARK: - Webhooks command names (for REPL completion)

let webhooksCommandNames: [String] = [
    "webhooks register",
    "webhooks unregister",
    "webhooks list",
]

// MARK: - Dispatch

func dispatchWebhooksCommand(_ args: [String], sdk: BreezSdk) async {
    if args.isEmpty || args[0] == "help" {
        printWebhooksHelp()
        return
    }

    let subName = args[0]
    let subArgs = Array(args.dropFirst())

    do {
        switch subName {
        case "register":
            try await handleWebhookRegister(sdk, subArgs)
        case "unregister":
            try await handleWebhookUnregister(sdk, subArgs)
        case "list":
            try await handleWebhookList(sdk, subArgs)
        default:
            print("Unknown webhooks subcommand: \(subName). Use 'webhooks help' for available commands.")
        }
    } catch {
        print("Error: \(error)")
    }
}

// MARK: - Help

private func printWebhooksHelp() {
    print("\nWebhooks subcommands:")
    print("  webhooks \("register".padding(toLength: 30, withPad: " ", startingAt: 0))Register a new webhook")
    print("  webhooks \("unregister".padding(toLength: 30, withPad: " ", startingAt: 0))Unregister a webhook")
    print("  webhooks \("list".padding(toLength: 30, withPad: " ", startingAt: 0))List all registered webhooks")
    print()
}

// MARK: - Parsing helper

private func parseWebhookEventType(_ raw: String) -> WebhookEventType? {
    switch raw.lowercased().replacingOccurrences(of: "-", with: "").replacingOccurrences(of: "_", with: "") {
    case "lightningreceive": return .lightningReceiveFinished
    case "lightningsend": return .lightningSendFinished
    case "coopexit": return .coopExitFinished
    case "staticdeposit": return .staticDepositFinished
    default: return nil
    }
}

// MARK: - Handlers

// --- register ---

private func handleWebhookRegister(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard fp.positional.count >= 3 else {
        print("Usage: webhooks register <url> <secret> <event_type> [<event_type> ...]")
        print("Event types: lightning-receive, lightning-send, coop-exit, static-deposit")
        return
    }

    let url = fp.positional[0]
    let secret = fp.positional[1]
    let eventTypes = fp.positional.dropFirst(2).compactMap { parseWebhookEventType(String($0)) }

    if eventTypes.isEmpty {
        print("No valid event types provided")
        print("Event types: lightning-receive, lightning-send, coop-exit, static-deposit")
        return
    }

    let result = try await sdk.registerWebhook(request: RegisterWebhookRequest(
        url: url,
        secret: secret,
        eventTypes: eventTypes
    ))
    printValue(result)
}

// --- unregister ---

private func handleWebhookUnregister(_ sdk: BreezSdk, _ args: [String]) async throws {
    guard !args.isEmpty else {
        print("Usage: webhooks unregister <webhook_id>")
        return
    }

    try await sdk.unregisterWebhook(request: UnregisterWebhookRequest(
        webhookId: args[0]
    ))
    print("Webhook unregistered successfully")
}

// --- list ---

private func handleWebhookList(_ sdk: BreezSdk, _ args: [String]) async throws {
    let result = try await sdk.listWebhooks()
    printValue(result)
}
