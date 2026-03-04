import Foundation
import BreezSdkSpark

// MARK: - Contacts command names (for REPL completion)

let contactsCommandNames: [String] = [
    "contacts add",
    "contacts update",
    "contacts delete",
    "contacts list",
]

// MARK: - Dispatch

func dispatchContactsCommand(_ args: [String], sdk: BreezSdk) async {
    if args.isEmpty || args[0] == "help" {
        printContactsHelp()
        return
    }

    let subName = args[0]
    let subArgs = Array(args.dropFirst())

    do {
        switch subName {
        case "add":
            try await handleContactAdd(sdk, subArgs)
        case "update":
            try await handleContactUpdate(sdk, subArgs)
        case "delete":
            try await handleContactDelete(sdk, subArgs)
        case "list":
            try await handleContactList(sdk, subArgs)
        default:
            print("Unknown contacts subcommand: \(subName). Use 'contacts help' for available commands.")
        }
    } catch {
        print("Error: \(error)")
    }
}

// MARK: - Help

private func printContactsHelp() {
    print("\nContacts subcommands:")
    print("  contacts \("add".padding(toLength: 30, withPad: " ", startingAt: 0))Add a new contact")
    print("  contacts \("update".padding(toLength: 30, withPad: " ", startingAt: 0))Update an existing contact")
    print("  contacts \("delete".padding(toLength: 30, withPad: " ", startingAt: 0))Delete a contact")
    print("  contacts \("list".padding(toLength: 30, withPad: " ", startingAt: 0))List contacts")
    print()
}

// MARK: - Handlers

// --- add ---

private func handleContactAdd(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard fp.positional.count >= 2 else {
        print("Usage: contacts add <name> <payment_identifier>")
        return
    }

    let result = try await sdk.addContact(
        request: AddContactRequest(
            name: fp.positional[0],
            paymentIdentifier: fp.positional[1]
        ))
    printValue(result)
}

// --- update ---

private func handleContactUpdate(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard fp.positional.count >= 3 else {
        print("Usage: contacts update <id> <name> <payment_identifier>")
        return
    }

    let result = try await sdk.updateContact(
        request: UpdateContactRequest(
            id: fp.positional[0],
            name: fp.positional[1],
            paymentIdentifier: fp.positional[2]
        ))
    printValue(result)
}

// --- delete ---

private func handleContactDelete(_ sdk: BreezSdk, _ args: [String]) async throws {
    guard !args.isEmpty else {
        print("Usage: contacts delete <id>")
        return
    }

    try await sdk.deleteContact(id: args[0])
    print("Contact deleted successfully")
}

// --- list ---

private func handleContactList(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    let offset = fp.get("offset").flatMap { UInt32($0) }
        ?? fp.positional.first.flatMap { UInt32($0) }
    let limit = fp.get("limit").flatMap { UInt32($0) }
        ?? (fp.positional.count > 1 ? UInt32(fp.positional[1]) : nil)

    let result = try await sdk.listContacts(
        request: ListContactsRequest(
            offset: offset,
            limit: limit
        ))
    printValue(result)
}
