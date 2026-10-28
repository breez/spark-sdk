import Foundation
import BreezSdkSpark
import BigNumber

// MARK: - Token allowance command names (for REPL completion)

let tokenAllowancesCommandNames: [String] = [
    "token-allowances create",
    "token-allowances revoke",
    "token-allowances list",
]

// MARK: - Dispatch

func dispatchTokenAllowancesCommand(_ args: [String], sdk: BreezSdk) async {
    if args.isEmpty || args[0] == "help" {
        printTokenAllowancesHelp()
        return
    }

    let subName = args[0]
    let subArgs = Array(args.dropFirst())

    do {
        switch subName {
        case "create":
            try await handleTokenAllowanceCreate(sdk, subArgs)
        case "revoke":
            try await handleTokenAllowanceRevoke(sdk, subArgs)
        case "list":
            try await handleTokenAllowanceList(sdk, subArgs)
        default:
            print("Unknown token-allowances subcommand: \(subName). Use 'token-allowances help' for available commands.")
        }
    } catch {
        print("Error: \(error)")
    }
}

// MARK: - Help

private func printTokenAllowancesHelp() {
    print("\nToken allowance subcommands:")
    print("  token-allowances \("create".padding(toLength: 22, withPad: " ", startingAt: 0))Grant a spender an allowance over this wallet's tokens")
    print("  token-allowances \("revoke".padding(toLength: 22, withPad: " ", startingAt: 0))Revoke a token allowance this wallet granted")
    print("  token-allowances \("list".padding(toLength: 22, withPad: " ", startingAt: 0))List token allowances (as owner or spender)")
    print()
}

// MARK: - Limit parsing

private func parseTokenAllowanceLimit(_ raw: String) -> TokenAllowanceLimit? {
    if raw.lowercased() == "unlimited" {
        return .unlimited
    }
    guard let amount = BInt(raw) else { return nil }
    return .amount(amount: amount)
}

// MARK: - Role parsing

private func parseTokenAllowanceRole(_ raw: String) -> TokenAllowanceRole? {
    switch raw.lowercased() {
    case "owner": return .owner
    case "spender": return .spender
    default: return nil
    }
}

// MARK: - Handlers

private func handleTokenAllowanceCreate(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard fp.positional.count >= 3,
          let expiryTime = UInt64(fp.positional[2]) else {
        print("Usage: token-allowances create <spender_address> <token_identifier> <expiry_time> --max-per-payment <amount|unlimited> --max-total <amount|unlimited> [-r <recipient> ...]")
        return
    }

    let spenderAddress = fp.positional[0]
    let tokenIdentifier = fp.positional[1]

    guard let maxPerPaymentStr = fp.get("max-per-payment"),
          let maxPerPayment = parseTokenAllowanceLimit(maxPerPaymentStr) else {
        print("--max-per-payment is required (amount or 'unlimited')")
        return
    }
    guard let maxTotalStr = fp.get("max-total"),
          let maxTotal = parseTokenAllowanceLimit(maxTotalStr) else {
        print("--max-total is required (amount or 'unlimited')")
        return
    }

    let allowedRecipients = fp.getAll("r", "recipient")

    let response = try await sdk.createTokenAllowance(
        request: CreateTokenAllowanceRequest(
            spenderAddress: spenderAddress,
            tokenIdentifier: tokenIdentifier,
            maxPerPayment: maxPerPayment,
            maxTotal: maxTotal,
            expiryTime: expiryTime,
            allowedRecipients: allowedRecipients
        ))
    printValue(response.allowance)
}

private func handleTokenAllowanceRevoke(_ sdk: BreezSdk, _ args: [String]) async throws {
    guard !args.isEmpty else {
        print("Usage: token-allowances revoke <allowance_id>")
        return
    }

    try await sdk.revokeTokenAllowance(
        request: RevokeTokenAllowanceRequest(allowanceId: args[0]))
    print("Token allowance revoked")
}

private func handleTokenAllowanceList(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    let roleStr = fp.positional.first ?? "owner"
    guard let role = parseTokenAllowanceRole(roleStr) else {
        print("Invalid role '\(roleStr)'. Use 'owner' or 'spender'.")
        return
    }

    let counterpartyAddress = fp.get("counterparty-address")
    let tokenIdentifier = fp.get("token-identifier")
    let includeInactive: Bool? = fp.get("include-inactive").map { $0 == "true" }
    let offset = fp.get("offset").flatMap { UInt32($0) }
    let limit = fp.get("limit").flatMap { UInt32($0) }

    let response = try await sdk.listTokenAllowances(
        request: ListTokenAllowancesRequest(
            role: role,
            counterpartyAddress: counterpartyAddress,
            tokenIdentifier: tokenIdentifier,
            includeInactive: includeInactive,
            offset: offset,
            limit: limit
        ))
    printValue(response.allowances)
}

// MARK: - Pull payment retry loop

private let pullRetryIntervalNs: UInt64 = 5_000_000_000
private let pullRetryWindowSecs: UInt64 = 600

func pullUntilSettled(
    sdk: BreezSdk,
    prepareResponse: PreparePullPaymentResponse
) async throws -> PullPaymentResponse {
    let deadline = prepareResponse.expiryTime + pullRetryWindowSecs
    while true {
        do {
            let response = try await sdk.pullPayment(
                request: PullPaymentRequest(prepareResponse: prepareResponse))
            let now = UInt64(Date().timeIntervalSince1970)
            if response.status != .pending || now >= deadline {
                return response
            }
            print("The pull \(response.txHash) is pending, checking again in 5s")
        } catch {
            let now = UInt64(Date().timeIntervalSince1970)
            if !mayRetryPullError(error) || now >= deadline {
                throw error
            }
            print("Sending the pull failed (\(error)), trying again in 5s")
        }
        try await Task.sleep(nanoseconds: pullRetryIntervalNs)
    }
}

private func mayRetryPullError(_ error: Error) -> Bool {
    guard let sdkError = error as? SdkError else { return false }
    switch sdkError {
    case .NetworkError: return true
    case .SparkError: return true
    default: return false
    }
}
