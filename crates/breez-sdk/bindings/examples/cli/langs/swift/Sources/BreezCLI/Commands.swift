import Foundation
import BreezSdkSpark
import BigNumber
import CryptoKit

// MARK: - Flag parsing

struct FlagParser {
    var positional: [String] = []
    private var flags: [String: String] = [:]

    init(_ args: [String]) {
        var i = 0
        while i < args.count {
            let arg = args[i]
            if arg.hasPrefix("--") {
                let key = String(arg.dropFirst(2))
                if i + 1 < args.count && !args[i + 1].hasPrefix("-") {
                    flags[key] = args[i + 1]
                    i += 2
                } else {
                    flags[key] = ""
                    i += 1
                }
            } else if arg.hasPrefix("-") && arg.count == 2 {
                let key = String(arg.dropFirst(1))
                if i + 1 < args.count && !args[i + 1].hasPrefix("-") {
                    flags[key] = args[i + 1]
                    i += 2
                } else {
                    flags[key] = ""
                    i += 1
                }
            } else {
                positional.append(arg)
                i += 1
            }
        }
    }

    func get(_ keys: String...) -> String? {
        for key in keys {
            if let val = flags[key], !val.isEmpty { return val }
        }
        return nil
    }

    func has(_ keys: String...) -> Bool {
        for key in keys { if flags[key] != nil { return true } }
        return false
    }
}

// MARK: - Command entry

struct CommandEntry {
    let name: String
    let description: String
    let run: (BreezSdk, [String]) async throws -> Void
}

// MARK: - Command names (for REPL completion)

let commandNames: [String] = [
    "get-info",
    "get-payment",
    "sync",
    "list-payments",
    "receive",
    "pay",
    "lnurl-pay",
    "lnurl-withdraw",
    "lnurl-auth",
    "claim-htlc-payment",
    "claim-deposit",
    "parse",
    "refund-deposit",
    "list-unclaimed-deposits",
    "buy-bitcoin",
    "check-lightning-address-available",
    "get-lightning-address",
    "register-lightning-address",
    "accept-lightning-address-transfer",
    "delete-lightning-address",
    "list-fiat-currencies",
    "list-fiat-rates",
    "recommended-fees",
    "get-tokens-metadata",
    "fetch-conversion-limits",
    "get-user-settings",
    "set-user-settings",
    "get-spark-status",
]

// MARK: - Registry

func buildCommandRegistry() -> [String: CommandEntry] {
    [
        "get-info":                          CommandEntry(name: "get-info", description: "Get balance information", run: handleGetInfo),
        "get-payment":                       CommandEntry(name: "get-payment", description: "Get the payment with the given ID", run: handleGetPayment),
        "sync":                              CommandEntry(name: "sync", description: "Sync wallet state", run: handleSync),
        "list-payments":                     CommandEntry(name: "list-payments", description: "List payments", run: handleListPayments),
        "receive":                           CommandEntry(name: "receive", description: "Receive a payment", run: handleReceive),
        "pay":                               CommandEntry(name: "pay", description: "Pay the given payment request", run: handlePay),
        "lnurl-pay":                         CommandEntry(name: "lnurl-pay", description: "Pay using LNURL", run: handleLnurlPay),
        "lnurl-withdraw":                    CommandEntry(name: "lnurl-withdraw", description: "Withdraw using LNURL", run: handleLnurlWithdraw),
        "lnurl-auth":                        CommandEntry(name: "lnurl-auth", description: "Authenticate using LNURL", run: handleLnurlAuth),
        "claim-htlc-payment":                CommandEntry(name: "claim-htlc-payment", description: "Claim an HTLC payment", run: handleClaimHtlcPayment),
        "claim-deposit":                     CommandEntry(name: "claim-deposit", description: "Claim an on-chain deposit", run: handleClaimDeposit),
        "parse":                             CommandEntry(name: "parse", description: "Parse an input (invoice, address, LNURL)", run: handleParse),
        "refund-deposit":                    CommandEntry(name: "refund-deposit", description: "Refund an on-chain deposit", run: handleRefundDeposit),
        "list-unclaimed-deposits":           CommandEntry(name: "list-unclaimed-deposits", description: "List unclaimed on-chain deposits", run: handleListUnclaimedDeposits),
        "buy-bitcoin":                       CommandEntry(name: "buy-bitcoin", description: "Buy Bitcoin via MoonPay", run: handleBuyBitcoin),
        "check-lightning-address-available": CommandEntry(name: "check-lightning-address-available", description: "Check if a lightning address username is available", run: handleCheckLightningAddress),
        "get-lightning-address":             CommandEntry(name: "get-lightning-address", description: "Get registered lightning address", run: handleGetLightningAddress),
        "register-lightning-address":        CommandEntry(name: "register-lightning-address", description: "Register a lightning address", run: handleRegisterLightningAddress),
        "accept-lightning-address-transfer": CommandEntry(name: "accept-lightning-address-transfer", description: "Produce a transfer authorization for the current username, granting it to a transferee pubkey", run: handleAcceptLightningAddressTransfer),
        "delete-lightning-address":          CommandEntry(name: "delete-lightning-address", description: "Delete lightning address", run: handleDeleteLightningAddress),
        "list-fiat-currencies":              CommandEntry(name: "list-fiat-currencies", description: "List fiat currencies", run: handleListFiatCurrencies),
        "list-fiat-rates":                   CommandEntry(name: "list-fiat-rates", description: "List available fiat rates", run: handleListFiatRates),
        "recommended-fees":                  CommandEntry(name: "recommended-fees", description: "Get recommended BTC fees", run: handleRecommendedFees),
        "get-tokens-metadata":               CommandEntry(name: "get-tokens-metadata", description: "Get metadata for token(s)", run: handleGetTokensMetadata),
        "fetch-conversion-limits":           CommandEntry(name: "fetch-conversion-limits", description: "Fetch conversion limits for a token", run: handleFetchConversionLimits),
        "get-user-settings":                 CommandEntry(name: "get-user-settings", description: "Get user settings", run: handleGetUserSettings),
        "set-user-settings":                 CommandEntry(name: "set-user-settings", description: "Update user settings", run: handleSetUserSettings),
        "get-spark-status":                  CommandEntry(name: "get-spark-status", description: "Get Spark network service status", run: handleGetSparkStatus),
    ]
}

// MARK: - Help

func printHelp(_ registry: [String: CommandEntry]) {
    print("\nAvailable commands:")
    let names = registry.keys.sorted()
    for name in names {
        let cmd = registry[name]!
        print("  \(name.padding(toLength: 40, withPad: " ", startingAt: 0))\(cmd.description)")
    }
    print("\n  \("issuer <subcommand>".padding(toLength: 40, withPad: " ", startingAt: 0))Token issuer commands (use 'issuer help' for details)")
    print("  \("contacts <subcommand>".padding(toLength: 40, withPad: " ", startingAt: 0))Contacts commands (use 'contacts help' for details)")
    print("  \("exit / quit".padding(toLength: 40, withPad: " ", startingAt: 0))Exit the CLI")
    print("  \("help".padding(toLength: 40, withPad: " ", startingAt: 0))Show this help message")
    print()
}

// MARK: - Interactive payment options

func readPaymentOptions(_ method: SendPaymentMethod) -> SendPaymentOptions? {
    switch method {
    case let .bitcoinAddress(_, feeQuote):
        let fastFee = feeQuote.speedFast.userFeeSat + feeQuote.speedFast.l1BroadcastFeeSat
        let mediumFee = feeQuote.speedMedium.userFeeSat + feeQuote.speedMedium.l1BroadcastFeeSat
        let slowFee = feeQuote.speedSlow.userFeeSat + feeQuote.speedSlow.l1BroadcastFeeSat
        print("Please choose payment fee:")
        print("1. Fast: \(fastFee) sats")
        print("2. Medium: \(mediumFee) sats")
        print("3. Slow: \(slowFee) sats")
        let line = readlineWithDefault("Choose (1/2/3): ", defaultValue: "1")
        let speed: OnchainConfirmationSpeed
        switch line.trimmingCharacters(in: .whitespaces) {
        case "2": speed = .medium
        case "3": speed = .slow
        default:  speed = .fast
        }
        return .bitcoinAddress(confirmationSpeed: speed)

    case let .bolt11Invoice(_, sparkTransferFeeSats, lightningFeeSats):
        if let sparkFee = sparkTransferFeeSats {
            print("Choose payment option:")
            print("1. Spark transfer fee: \(sparkFee) sats")
            print("2. Lightning fee: \(lightningFeeSats) sats")
            let line = readlineWithDefault("Choose (1/2): ", defaultValue: "1")
            if line.trimmingCharacters(in: .whitespaces) == "1" {
                return .bolt11Invoice(preferSpark: true, completionTimeoutSecs: 0)
            }
        }
        return .bolt11Invoice(preferSpark: false, completionTimeoutSecs: 0)

    case let .sparkAddress(_, _, tokenIdentifier):
        if tokenIdentifier != nil {
            return nil
        }
        let line = readlineWithDefault("Do you want to create an HTLC transfer? (y/n): ", defaultValue: "n")
        if line.trimmingCharacters(in: .whitespaces).lowercased() != "y" {
            return nil
        }
        guard let hashLine = readlinePrompt("Payment hash (hex, or empty to generate): ") else {
            return nil
        }
        let paymentHash: String
        if hashLine.trimmingCharacters(in: .whitespaces).isEmpty {
            var preimageBytes = [UInt8](repeating: 0, count: 32)
            _ = SecRandomCopyBytes(kSecRandomDefault, 32, &preimageBytes)
            let preimage = preimageBytes.map { String(format: "%02x", $0) }.joined()
            let hash = SHA256.hash(data: Data(preimageBytes))
            paymentHash = hash.map { String(format: "%02x", $0) }.joined()
            print("Generated preimage: \(preimage)")
            print("Associated payment hash: \(paymentHash)")
        } else {
            paymentHash = hashLine.trimmingCharacters(in: .whitespaces)
        }
        guard let expiryLine = readlinePrompt("HTLC expiry duration in seconds: "),
              let expiryDuration = UInt64(expiryLine.trimmingCharacters(in: .whitespaces)) else {
            print("Invalid expiry duration")
            return nil
        }
        return .sparkAddress(htlcOptions: SparkHtlcOptions(
            paymentHash: paymentHash,
            expiryDurationSecs: expiryDuration
        ))

    case .sparkInvoice:
        return nil
    }
}

// MARK: - Hex helper

private func generatePreimageAndHash() -> (preimage: String, paymentHash: String) {
    var preimageBytes = [UInt8](repeating: 0, count: 32)
    _ = SecRandomCopyBytes(kSecRandomDefault, 32, &preimageBytes)
    let preimage = preimageBytes.map { String(format: "%02x", $0) }.joined()
    let hash = SHA256.hash(data: Data(preimageBytes))
    let paymentHash = hash.map { String(format: "%02x", $0) }.joined()
    return (preimage, paymentHash)
}

// MARK: - Command handlers

// --- get-info ---

func handleGetInfo(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    let ensureSynced: Bool? = fp.has("e", "ensure-synced") ? true : nil
    let result = try await sdk.getInfo(request: GetInfoRequest(ensureSynced: ensureSynced))
    printValue(result)
}

// --- get-payment ---

func handleGetPayment(_ sdk: BreezSdk, _ args: [String]) async throws {
    guard !args.isEmpty else {
        print("Usage: get-payment <payment_id>")
        return
    }
    let result = try await sdk.getPayment(request: GetPaymentRequest(paymentId: args[0]))
    printValue(result)
}

// --- sync ---

func handleSync(_ sdk: BreezSdk, _ args: [String]) async throws {
    let result = try await sdk.syncWallet(request: SyncWalletRequest())
    printValue(result)
}

// --- list-payments ---

func handleListPayments(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    let limit = fp.get("l", "limit").flatMap { UInt32($0) } ?? 10
    let offset = fp.get("o", "offset").flatMap { UInt32($0) } ?? 0

    let typeFilter: [PaymentType]? = fp.get("t", "type-filter").map { raw in
        raw.split(separator: ",").compactMap { parsePaymentType(String($0)) }
    }
    let statusFilter: [PaymentStatus]? = fp.get("s", "status-filter").map { raw in
        raw.split(separator: ",").compactMap { parsePaymentStatus(String($0)) }
    }
    let assetFilter: AssetFilter? = fp.get("a", "asset-filter").flatMap { parseAssetFilter($0) }
    let fromTimestamp = fp.get("from-timestamp").flatMap { UInt64($0) }
    let toTimestamp = fp.get("to-timestamp").flatMap { UInt64($0) }
    let sortAscending: Bool? = fp.get("sort-ascending").map { $0 == "true" }

    var paymentDetailsFilter: [PaymentDetailsFilter] = []
    if let htlcStatusRaw = fp.get("spark-htlc-status-filter") {
        let statuses = htlcStatusRaw.split(separator: ",").compactMap { parseSparkHtlcStatus(String($0)) }
        paymentDetailsFilter.append(.spark(htlcStatus: statuses, conversionRefundNeeded: nil))
    }
    if let txHash = fp.get("tx-hash") {
        paymentDetailsFilter.append(.token(conversionRefundNeeded: nil, txHash: txHash, txType: nil))
    }
    if let txTypeRaw = fp.get("tx-type"), let txType = parseTokenTransactionType(txTypeRaw) {
        paymentDetailsFilter.append(.token(conversionRefundNeeded: nil, txHash: nil, txType: txType))
    }

    let result = try await sdk.listPayments(request: ListPaymentsRequest(
        typeFilter: typeFilter,
        statusFilter: statusFilter,
        assetFilter: assetFilter,
        paymentDetailsFilter: paymentDetailsFilter.isEmpty ? nil : paymentDetailsFilter,
        fromTimestamp: fromTimestamp,
        toTimestamp: toTimestamp,
        offset: offset,
        limit: limit,
        sortAscending: sortAscending
    ))
    printValue(result)
}

// MARK: - Filter parsing helpers

private func parsePaymentType(_ raw: String) -> PaymentType? {
    switch raw.lowercased() {
    case "send": return .send
    case "receive": return .receive
    default: return nil
    }
}

private func parsePaymentStatus(_ raw: String) -> PaymentStatus? {
    switch raw.lowercased() {
    case "pending": return .pending
    case "completed": return .completed
    case "failed": return .failed
    default: return nil
    }
}

private func parseAssetFilter(_ raw: String) -> AssetFilter? {
    if raw.lowercased() == "bitcoin" {
        return .bitcoin
    }
    return .token(tokenIdentifier: raw)
}

private func parseSparkHtlcStatus(_ raw: String) -> SparkHtlcStatus? {
    switch raw.lowercased().replacingOccurrences(of: "_", with: "") {
    case "waitingforpreimage": return .waitingForPreimage
    case "preimageshared": return .preimageShared
    case "returned": return .returned
    default: return nil
    }
}

private func parseTokenTransactionType(_ raw: String) -> TokenTransactionType? {
    switch raw.lowercased() {
    case "transfer": return .transfer
    case "mint": return .mint
    case "burn": return .burn
    default: return nil
    }
}

// --- receive ---

func handleReceive(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard let method = fp.get("m", "method") else {
        print("Usage: receive -m <method> [options]")
        print("Methods: sparkaddress, sparkinvoice, bitcoin, bolt11")
        return
    }

    let description = fp.get("d", "description")
    let amountStr = fp.get("a", "amount")
    let tokenIdentifier = fp.get("t", "token-identifier")
    let expirySecs = fp.get("e", "expiry-secs").flatMap { UInt32($0) }
    let senderPublicKey = fp.get("s", "sender-public-key")
    let hodl = fp.has("hodl")
    let newAddress = fp.has("new-address")

    let paymentMethod: ReceivePaymentMethod

    switch method.lowercased() {
    case "sparkaddress":
        paymentMethod = .sparkAddress

    case "sparkinvoice":
        let amount: BInt? = amountStr.flatMap { BInt($0) }
        let expiryTime: UInt64? = expirySecs.map { secs in
            UInt64(Date().timeIntervalSince1970) + UInt64(secs)
        }
        paymentMethod = .sparkInvoice(
            amount: amount,
            tokenIdentifier: tokenIdentifier,
            expiryTime: expiryTime,
            description: description,
            senderPublicKey: senderPublicKey
        )

    case "bitcoin":
        paymentMethod = .bitcoinAddress(newAddress: newAddress)

    case "bolt11":
        var paymentHash: String? = nil
        if hodl {
            let (preimage, hash) = generatePreimageAndHash()
            print("HODL invoice preimage: \(preimage)")
            print("Payment hash: \(hash)")
            print("Save the preimage! Use `claim-htlc-payment` with it to settle.")
            paymentHash = hash
        }
        let amountSats: UInt64? = amountStr.flatMap { UInt64($0) }
        paymentMethod = .bolt11Invoice(
            description: description ?? "",
            amountSats: amountSats,
            expirySecs: expirySecs,
            paymentHash: paymentHash
        )

    default:
        print("Invalid payment method: \(method)")
        print("Available methods: sparkaddress, sparkinvoice, bitcoin, bolt11")
        return
    }

    let result = try await sdk.receivePayment(request: ReceivePaymentRequest(
        paymentMethod: paymentMethod
    ))

    if result.fee > 0 {
        print("Prepared payment requires fee of \(result.fee) sats/token base units\n")
    }
    printValue(result)
}

// --- pay ---

func handlePay(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard let paymentRequest = fp.get("r", "payment-request") else {
        print("Usage: pay -r <payment_request> [-a <amount>] [-t <token_identifier>] [--from-bitcoin] [--from-token <id>] [-s <slippage_bps>] [--fees-included]")
        return
    }

    let amount: BInt? = fp.get("a", "amount").flatMap { BInt($0) }
    let tokenIdentifier = fp.get("t", "token-identifier")
    let idempotencyKey = fp.get("i", "idempotency-key")
    let fromBitcoin = fp.has("from-bitcoin")
    let fromTokenId = fp.get("from-token")
    let maxSlippageBps = fp.get("s", "convert-max-slippage-bps").flatMap { UInt32($0) }
    let feesIncluded = fp.has("fees-included")

    let conversionOptions: ConversionOptions?
    if fromBitcoin {
        conversionOptions = ConversionOptions(
            conversionType: .fromBitcoin,
            maxSlippageBps: maxSlippageBps,
            completionTimeoutSecs: nil
        )
    } else if let fromTokenId {
        conversionOptions = ConversionOptions(
            conversionType: .toBitcoin(fromTokenIdentifier: fromTokenId),
            maxSlippageBps: maxSlippageBps,
            completionTimeoutSecs: nil
        )
    } else {
        conversionOptions = nil
    }

    let feePolicy: FeePolicy? = feesIncluded ? .feesIncluded : nil

    let prepareResponse = try await sdk.prepareSendPayment(request: PrepareSendPaymentRequest(
        paymentRequest: paymentRequest,
        amount: amount,
        tokenIdentifier: tokenIdentifier,
        conversionOptions: conversionOptions,
        feePolicy: feePolicy
    ))

    if let estimate = prepareResponse.conversionEstimate {
        let units: String
        if case .fromBitcoin = estimate.options.conversionType {
            units = "sats"
        } else {
            units = "token base units"
        }
        print("Estimated conversion of \(estimate.amountIn) \(units) → \(estimate.amountOut) \(units) with a \(estimate.fee) \(units) fee")
        let line = readlineWithDefault("Do you want to continue (y/n): ", defaultValue: "y")
        if line.trimmingCharacters(in: .whitespaces).lowercased() != "y" {
            print("Payment cancelled")
            return
        }
    }

    let paymentOptions = readPaymentOptions(prepareResponse.paymentMethod)

    let result = try await sdk.sendPayment(request: SendPaymentRequest(
        prepareResponse: prepareResponse,
        options: paymentOptions,
        idempotencyKey: idempotencyKey
    ))
    printValue(result)
}

// --- lnurl-pay ---

func handleLnurlPay(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard let lnurl = fp.positional.first else {
        print("Usage: lnurl-pay <lnurl> [-c <comment>] [-v <validate>] [-i <idempotency_key>] [--from-token <id>] [-s <slippage_bps>] [--fees-included]")
        return
    }

    let comment = fp.get("c", "comment")
    let validateStr = fp.get("v", "validate")
    let validateSuccessUrl: Bool? = validateStr.map { $0 == "true" }
    let idempotencyKey = fp.get("i", "idempotency-key")
    let fromTokenId = fp.get("from-token")
    let maxSlippageBps = fp.get("s", "convert-max-slippage-bps").flatMap { UInt32($0) }
    let feesIncluded = fp.has("fees-included")

    let conversionOptions: ConversionOptions? = fromTokenId.map { tokenId in
        ConversionOptions(
            conversionType: .toBitcoin(fromTokenIdentifier: tokenId),
            maxSlippageBps: maxSlippageBps,
            completionTimeoutSecs: nil
        )
    }
    let feePolicy: FeePolicy? = feesIncluded ? .feesIncluded : nil

    let input = try await sdk.parse(input: lnurl)

    let payRequest: LnurlPayRequestDetails
    switch input {
    case let .lnurlPay(v1):
        payRequest = v1
    case let .lightningAddress(v1):
        payRequest = v1.payRequest
    default:
        print("Input is not an LNURL-pay or lightning address")
        return
    }

    let minSendable = (payRequest.minSendable + 999) / 1000
    let maxSendable = payRequest.maxSendable / 1000
    let prompt = "Amount to pay (min \(minSendable) sat, max \(maxSendable) sat): "
    guard let amountLine = readlinePrompt(prompt),
          let amountSats = UInt64(amountLine.trimmingCharacters(in: .whitespaces)) else {
        print("Invalid amount")
        return
    }

    let prepareResponse = try await sdk.prepareLnurlPay(request: PrepareLnurlPayRequest(
        amount: BInt(amountSats),
        payRequest: payRequest,
        comment: comment,
        validateSuccessActionUrl: validateSuccessUrl,
        tokenIdentifier: nil,
        conversionOptions: conversionOptions,
        feePolicy: feePolicy
    ))

    if let estimate = prepareResponse.conversionEstimate {
        print("Estimated conversion of \(estimate.amountIn) token base units → \(estimate.amountOut) sats with a \(estimate.fee) token base units fee")
        let line = readlineWithDefault("Do you want to continue (y/n): ", defaultValue: "y")
        if line.trimmingCharacters(in: .whitespaces).lowercased() != "y" {
            print("Payment cancelled")
            return
        }
    }

    printValue(prepareResponse)

    let confirm = readlineWithDefault("Do you want to continue? (y/n): ", defaultValue: "y")
    if confirm.trimmingCharacters(in: .whitespaces).lowercased() != "y" {
        return
    }

    let result = try await sdk.lnurlPay(request: LnurlPayRequest(
        prepareResponse: prepareResponse,
        idempotencyKey: idempotencyKey
    ))
    printValue(result)
}

// --- lnurl-withdraw ---

func handleLnurlWithdraw(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard let lnurl = fp.positional.first else {
        print("Usage: lnurl-withdraw <lnurl> [--timeout N]")
        return
    }

    let timeoutSecs = fp.get("t", "timeout").flatMap { UInt32($0) }

    let input = try await sdk.parse(input: lnurl)

    guard case let .lnurlWithdraw(v1) = input else {
        print("Input is not an LNURL-withdraw")
        return
    }
    let withdrawRequest = v1

    let minWithdrawable = (withdrawRequest.minWithdrawable + 999) / 1000
    let maxWithdrawable = withdrawRequest.maxWithdrawable / 1000
    let prompt = "Amount to withdraw (min \(minWithdrawable) sat, max \(maxWithdrawable) sat): "
    guard let amountLine = readlinePrompt(prompt),
          let amountSats = UInt64(amountLine.trimmingCharacters(in: .whitespaces)) else {
        print("Invalid amount")
        return
    }

    let result = try await sdk.lnurlWithdraw(request: LnurlWithdrawRequest(
        amountSats: amountSats,
        withdrawRequest: withdrawRequest,
        completionTimeoutSecs: timeoutSecs
    ))
    printValue(result)
}

// --- lnurl-auth ---

func handleLnurlAuth(_ sdk: BreezSdk, _ args: [String]) async throws {
    guard !args.isEmpty else {
        print("Usage: lnurl-auth <lnurl>")
        return
    }

    let input = try await sdk.parse(input: args[0])

    guard case let .lnurlAuth(v1) = input else {
        print("Input is not an LNURL-auth")
        return
    }
    let authRequest = v1

    let action = authRequest.action ?? "auth"
    let prompt = "Authenticate with \(authRequest.domain) (action: \(action))? (y/n): "
    let line = readlineWithDefault(prompt, defaultValue: "y")
    if line.trimmingCharacters(in: .whitespaces).lowercased() != "y" {
        return
    }

    let result = try await sdk.lnurlAuth(requestData: authRequest)
    printValue(result)
}

// --- claim-htlc-payment ---

func handleClaimHtlcPayment(_ sdk: BreezSdk, _ args: [String]) async throws {
    guard !args.isEmpty else {
        print("Usage: claim-htlc-payment <preimage>")
        return
    }

    let result = try await sdk.claimHtlcPayment(request: ClaimHtlcPaymentRequest(
        preimage: args[0]
    ))
    printValue(result.payment)
}

// --- claim-deposit ---

func handleClaimDeposit(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard fp.positional.count >= 2,
          let vout = UInt32(fp.positional[1]) else {
        print("Usage: claim-deposit <txid> <vout> [--fee-sat N | --sat-per-vbyte N | --recommended-fee-leeway N]")
        return
    }

    let txid = fp.positional[0]
    let feeSat = fp.get("fee-sat").flatMap { UInt64($0) }
    let satPerVbyte = fp.get("sat-per-vbyte").flatMap { UInt64($0) }
    let recommendedFeeLeeway = fp.get("recommended-fee-leeway").flatMap { UInt64($0) }

    let maxFee: MaxFee?
    if let leeway = recommendedFeeLeeway {
        if feeSat != nil || satPerVbyte != nil {
            print("Cannot specify fee-sat or sat-per-vbyte when using recommended fee")
            return
        }
        maxFee = .networkRecommended(leewaySatPerVbyte: leeway)
    } else if let feeSat, satPerVbyte != nil {
        print("Cannot specify both fee-sat and sat-per-vbyte")
        return
    } else if let feeSat {
        maxFee = .fixed(amount: feeSat)
    } else if let satPerVbyte {
        maxFee = .rate(satPerVbyte: satPerVbyte)
    } else {
        maxFee = nil
    }

    let result = try await sdk.claimDeposit(request: ClaimDepositRequest(
        txid: txid,
        vout: vout,
        maxFee: maxFee
    ))
    printValue(result)
}

// --- parse ---

func handleParse(_ sdk: BreezSdk, _ args: [String]) async throws {
    guard !args.isEmpty else {
        print("Usage: parse <input>")
        return
    }

    let result = try await sdk.parse(input: args[0])
    printValue(result)
}

// --- refund-deposit ---

func handleRefundDeposit(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard fp.positional.count >= 3,
          let vout = UInt32(fp.positional[1]) else {
        print("Usage: refund-deposit <txid> <vout> <destination_address> [--fee-sat N | --sat-per-vbyte N]")
        return
    }

    let txid = fp.positional[0]
    let destAddr = fp.positional[2]
    let feeSat = fp.get("fee-sat").flatMap { UInt64($0) }
    let satPerVbyte = fp.get("sat-per-vbyte").flatMap { UInt64($0) }

    let fee: Fee
    if let feeSat, satPerVbyte != nil {
        print("Cannot specify both fee-sat and sat-per-vbyte")
        return
    } else if let feeSat {
        fee = .fixed(amount: feeSat)
    } else if let satPerVbyte {
        fee = .rate(satPerVbyte: satPerVbyte)
    } else {
        print("Must specify either --fee-sat or --sat-per-vbyte")
        return
    }

    let result = try await sdk.refundDeposit(request: RefundDepositRequest(
        txid: txid,
        vout: vout,
        destinationAddress: destAddr,
        fee: fee
    ))
    printValue(result)
}

// --- list-unclaimed-deposits ---

func handleListUnclaimedDeposits(_ sdk: BreezSdk, _ args: [String]) async throws {
    let result = try await sdk.listUnclaimedDeposits(request: ListUnclaimedDepositsRequest())
    printValue(result)
}

// --- buy-bitcoin ---

func handleBuyBitcoin(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    let lockedAmount = fp.get("locked-amount-sat").flatMap { UInt64($0) }
    let redirectUrl = fp.get("redirect-url")

    let result = try await sdk.buyBitcoin(request: .moonpay(
        lockedAmountSat: lockedAmount,
        redirectUrl: redirectUrl
    ))
    print("Open this URL in a browser to complete the purchase:")
    print(result.url)
}

// --- check-lightning-address-available ---

func handleCheckLightningAddress(_ sdk: BreezSdk, _ args: [String]) async throws {
    guard !args.isEmpty else {
        print("Usage: check-lightning-address-available <username>")
        return
    }

    let available = try await sdk.checkLightningAddressAvailable(req: CheckLightningAddressRequest(
        username: args[0]
    ))
    if available {
        print("Username '\(args[0])' is available")
    } else {
        print("Username '\(args[0])' is NOT available")
    }
}

// --- get-lightning-address ---

func handleGetLightningAddress(_ sdk: BreezSdk, _ args: [String]) async throws {
    let result = try await sdk.getLightningAddress()
    if let address = result {
        printValue(address)
    } else {
        print("No lightning address registered")
    }
}

// --- register-lightning-address ---

func handleRegisterLightningAddress(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard let username = fp.positional.first else {
        print("Usage: register-lightning-address <username> [-d <description>] [--transfer-pubkey <pk> --transfer-signature <sig>]")
        return
    }

    let description = fp.get("d", "description")
    let transferPubkey = fp.get("transfer-pubkey")
    let transferSignature = fp.get("transfer-signature")

    if (transferPubkey == nil) != (transferSignature == nil) {
        print("Error: --transfer-pubkey and --transfer-signature must be provided together")
        return
    }
    let transfer: LightningAddressTransfer? = transferPubkey.map {
        LightningAddressTransfer(pubkey: $0, signature: transferSignature!)
    }

    let result = try await sdk.registerLightningAddress(request: RegisterLightningAddressRequest(
        username: username,
        description: description,
        transfer: transfer
    ))
    printValue(result)
}

// --- accept-lightning-address-transfer ---

func handleAcceptLightningAddressTransfer(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard fp.positional.count >= 1 else {
        print("Usage: accept-lightning-address-transfer <transferee_pubkey>")
        return
    }
    let result = try await sdk.acceptLightningAddressTransfer(
        request: AcceptLightningAddressTransferRequest(
            transfereePubkey: fp.positional[0]
        )
    )
    printValue(result)
}

// --- delete-lightning-address ---

func handleDeleteLightningAddress(_ sdk: BreezSdk, _ args: [String]) async throws {
    try await sdk.deleteLightningAddress()
    print("Lightning address deleted")
}

// --- list-fiat-currencies ---

func handleListFiatCurrencies(_ sdk: BreezSdk, _ args: [String]) async throws {
    let result = try await sdk.listFiatCurrencies()
    printValue(result)
}

// --- list-fiat-rates ---

func handleListFiatRates(_ sdk: BreezSdk, _ args: [String]) async throws {
    let result = try await sdk.listFiatRates()
    printValue(result)
}

// --- recommended-fees ---

func handleRecommendedFees(_ sdk: BreezSdk, _ args: [String]) async throws {
    let result = try await sdk.recommendedFees()
    printValue(result)
}

// --- get-tokens-metadata ---

func handleGetTokensMetadata(_ sdk: BreezSdk, _ args: [String]) async throws {
    guard !args.isEmpty else {
        print("Usage: get-tokens-metadata <token_id> [<token_id2> ...]")
        return
    }

    let result = try await sdk.getTokensMetadata(request: GetTokensMetadataRequest(
        tokenIdentifiers: args
    ))
    printValue(result)
}

// --- fetch-conversion-limits ---

func handleFetchConversionLimits(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard let tokenId = fp.get("token") ?? fp.positional.first else {
        print("Usage: fetch-conversion-limits --token <token_id> [--from-bitcoin]")
        return
    }

    let fromBitcoin = fp.has("f", "from-bitcoin")

    let conversionType: ConversionType
    let tokenIdentifier: String?
    if fromBitcoin {
        conversionType = .fromBitcoin
        tokenIdentifier = tokenId
    } else {
        conversionType = .toBitcoin(fromTokenIdentifier: tokenId)
        tokenIdentifier = nil
    }

    let result = try await sdk.fetchConversionLimits(request: FetchConversionLimitsRequest(
        conversionType: conversionType,
        tokenIdentifier: tokenIdentifier
    ))
    printValue(result)
}

// --- get-user-settings ---

func handleGetUserSettings(_ sdk: BreezSdk, _ args: [String]) async throws {
    let result = try await sdk.getUserSettings()
    printValue(result)
}

// --- set-user-settings ---

func handleSetUserSettings(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    let privateModeStr = fp.get("p", "private", "spark-private-mode")
    let privateMode: Bool? = privateModeStr.map { $0 == "true" }

    try await sdk.updateUserSettings(request: UpdateUserSettingsRequest(
        sparkPrivateModeEnabled: privateMode
    ))
    print("User settings updated")
}

// --- get-spark-status ---

func handleGetSparkStatus(_ sdk: BreezSdk, _ args: [String]) async throws {
    let result = try await getSparkStatus()
    printValue(result)
}
