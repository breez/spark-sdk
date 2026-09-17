import Foundation
import BreezSdkSpark

// MARK: - Advanced command names (for REPL completion)

let advancedCommandNames: [String] = [
    "advanced unilateral-exit",
    "advanced check-unilateral-exit",
    "advanced export-unilateral-exit-state",
    "advanced import-unilateral-exit-state",
]

// MARK: - Dispatch

func dispatchAdvancedCommand(_ args: [String], sdk: BreezSdk) async {
    if args.isEmpty || args[0] == "help" {
        printAdvancedHelp()
        return
    }

    let subName = args[0]
    let subArgs = Array(args.dropFirst())

    do {
        switch subName {
        case "unilateral-exit":
            try await handleUnilateralExit(sdk, subArgs)
        case "check-unilateral-exit":
            try await handleCheckUnilateralExit(sdk, subArgs)
        case "export-unilateral-exit-state":
            try await handleExportUnilateralExitState(sdk, subArgs)
        case "import-unilateral-exit-state":
            try await handleImportUnilateralExitState(sdk, subArgs)
        default:
            print("Unknown advanced subcommand: \(subName). Use 'advanced help' for available commands.")
        }
    } catch {
        print("Error: \(error)")
    }
}

// MARK: - Help

private func printAdvancedHelp() {
    print("\nAdvanced subcommands (expert-only, misuse can strand or lose funds):")
    print("  advanced \("unilateral-exit".padding(toLength: 38, withPad: " ", startingAt: 0))Build and sign a unilateral exit")
    print("  advanced \("check-unilateral-exit".padding(toLength: 38, withPad: " ", startingAt: 0))Check a signed exit against the chain")
    print("  advanced \("export-unilateral-exit-state".padding(toLength: 38, withPad: " ", startingAt: 0))Export exit state to a file")
    print("  advanced \("import-unilateral-exit-state".padding(toLength: 38, withPad: " ", startingAt: 0))Import exit state from a file")
    print()
}

// MARK: - Handlers

private func handleUnilateralExit(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard let feeRateStr = fp.get("fee-rate"),
          let feeRate = UInt64(feeRateStr) else {
        print("Usage: advanced unilateral-exit --fee-rate <sat/vbyte> --destination <address> [--funding-kind p2wpkh|p2tr] [--leaf <id> ...] [--output-file <path>]")
        return
    }
    guard let destination = fp.get("destination") else {
        print("Usage: advanced unilateral-exit --fee-rate <sat/vbyte> --destination <address> [--funding-kind p2wpkh|p2tr] [--leaf <id> ...] [--output-file <path>]")
        return
    }

    let fundingKindStr = fp.get("funding-kind") ?? "p2tr"
    let fundingKind: CpfpFundingKind
    switch fundingKindStr.lowercased() {
    case "p2wpkh": fundingKind = .p2wpkh
    case "p2tr":   fundingKind = .p2tr
    default:
        print("Invalid funding kind '\(fundingKindStr)'. Use 'p2wpkh' or 'p2tr'.")
        return
    }

    let leafIds = collectRepeatedFlag(args, flag: "--leaf")
    let selection: ExitLeafSelection = leafIds.isEmpty ? .auto : .specific(leafIds: leafIds)
    let outputFile = fp.get("output-file")

    let prepared = try await sdk.prepareUnilateralExit(
        request: PrepareUnilateralExitRequest(
            feeRateSatPerVbyte: feeRate,
            fundingKind: fundingKind,
            destination: destination,
            selection: selection
        )
    )
    printValue(prepared)

    if prepared.leaves.isEmpty {
        print("No leaves to exit.")
        return
    }

    guard let utxoLine = readlinePrompt(
        "Funding UTXO(s) as txid:vout:value:pubkey (space-separated, blank to stop): "
    ) else { return }

    if utxoLine.trimmingCharacters(in: .whitespaces).isEmpty {
        print("No funding provided; showing the quote only.")
        return
    }

    let utxoParts = utxoLine.split(separator: " ").map(String.init)
    var fundingInputs: [CpfpInput] = []
    for part in utxoParts {
        guard let input = parseCpfpInput(part, fundingKind) else {
            print("Invalid funding UTXO '\(part)', expected txid:vout:value:pubkey")
            return
        }
        fundingInputs.append(input)
    }

    guard let keyLine = readlinePrompt("Hex secret key for the funding UTXO(s): ") else { return }
    guard let keyData = dataFromHex(keyLine.trimmingCharacters(in: .whitespaces)) else {
        print("Invalid hex key")
        return
    }
    let signer = try singleKeyCpfpSigner(secretKeyBytes: keyData)

    let response = try await sdk.unilateralExit(
        request: UnilateralExitRequest(
            prepared: prepared,
            fundingInputs: fundingInputs
        ),
        signer: signer
    )
    printExitTransactions(response)
    if let outputFile {
        try writeExit(outputFile, response)
    }
}

private func handleExportUnilateralExitState(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard let outputFile = fp.get("output-file") else {
        print("Usage: advanced export-unilateral-exit-state --output-file <path>")
        return
    }

    let exported = try await sdk.exportUnilateralExitState()
    let url = URL(fileURLWithPath: outputFile)
    try exported.exitState.write(to: url, atomically: true, encoding: .utf8)
    print("Wrote \(exported.exitState.count) bytes to \(outputFile)")
}

private func handleImportUnilateralExitState(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard let inputFile = fp.get("input-file") else {
        print("Usage: advanced import-unilateral-exit-state --input-file <path>")
        return
    }

    let url = URL(fileURLWithPath: inputFile)
    let exitState = try String(contentsOf: url, encoding: .utf8)
    let imported = try await sdk.importUnilateralExitState(
        request: ImportUnilateralExitStateRequest(exitState: exitState)
    )
    print(
        "Imported \(imported.importedLeaves) leaf(s), " +
        "skipped \(imported.skippedForeignLeaves) leaf(s) from a different wallet " +
        "and \(imported.skippedConflictingLeaves) that disagree with what this wallet holds, " +
        "left out the exit data of \(imported.skippedChains) leaf(s)"
    )
}

private func handleCheckUnilateralExit(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard let inputFile = fp.get("input-file") else {
        print("Usage: advanced check-unilateral-exit --input-file <path> [--output-file <path>]")
        return
    }
    let outputFile = fp.get("output-file")

    let exit = try readExit(inputFile)
    let checked = try await sdk.checkUnilateralExit(
        request: CheckUnilateralExitRequest(exit: exit)
    )

    switch checked.verdict {
    case .valid:
        print("Verdict: Valid")
    case .done:
        print("Verdict: Done")
    case .redo:
        print("Verdict: Redo")
        print("  (this exit cannot be finished, quote and build it again)")
    }
    printExitTransactions(checked.exit)
    try writeExit(outputFile ?? inputFile, checked.exit)
}

// MARK: - Exit file I/O

private func readExit(_ path: String) throws -> UnilateralExitResponse {
    let url = URL(fileURLWithPath: path)
    let data = try Data(contentsOf: url)
    guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
        throw exitFileError("expected a JSON object")
    }
    return try deserializeExitResponse(json)
}

private func writeExit(_ path: String, _ exit: UnilateralExitResponse) throws {
    let json = serialize(exit)
    let url = URL(fileURLWithPath: path)
    try json.write(to: url, atomically: true, encoding: .utf8)
    print("Wrote the exit to \(path)")
}

// MARK: - Exit deserialization

private func deserializeExitResponse(_ d: [String: Any]) throws -> UnilateralExitResponse {
    guard let recoverableValueSat = jsonUInt64(d["recoverable_value_sat"]),
          let totalFeeSat = jsonUInt64(d["total_fee_sat"]),
          let cpfpFeeSat = jsonUInt64(d["cpfp_fee_sat"]),
          let fanoutFeeSat = jsonUInt64(d["fanout_fee_sat"]),
          let sweepFeeSat = jsonUInt64(d["sweep_fee_sat"]),
          let leavesArr = d["leaves"] as? [[String: Any]],
          let txsArr = d["transactions"] as? [[String: Any]],
          let fundingArr = d["funding_inputs"] as? [[String: Any]]
    else {
        throw exitFileError("missing or invalid top-level fields")
    }
    return UnilateralExitResponse(
        recoverableValueSat: recoverableValueSat,
        totalFeeSat: totalFeeSat,
        cpfpFeeSat: cpfpFeeSat,
        fanoutFeeSat: fanoutFeeSat,
        sweepFeeSat: sweepFeeSat,
        leaves: try leavesArr.map { try deserializeExitLeaf($0) },
        transactions: try txsArr.map { try deserializeExitTx($0) },
        fundingInputs: try fundingArr.map { try deserializeCpfpInputFromJson($0) }
    )
}

private func deserializeExitLeaf(_ d: [String: Any]) throws -> UnilateralExitLeaf {
    guard let leafId = d["leaf_id"] as? String,
          let value = jsonUInt64(d["value"])
    else {
        throw exitFileError("invalid leaf")
    }
    return UnilateralExitLeaf(leafId: leafId, value: value)
}

private func deserializeExitTx(_ d: [String: Any]) throws -> UnilateralExitTransaction {
    guard let kindStr = d["kind"] as? String,
          let txid = d["txid"] as? String,
          let txHex = d["tx_hex"] as? String
    else {
        throw exitFileError("invalid transaction")
    }

    let kind: UnilateralExitTxKind
    switch kindStr {
    case "fanOut":  kind = .fanOut
    case "node":    kind = .node
    case "refund":  kind = .refund
    case "sweep":   kind = .sweep
    default: throw exitFileError("unknown tx kind '\(kindStr)'")
    }

    let dependsOn = (d["depends_on"] as? [String]) ?? []

    let status: ExitTransactionStatus
    if let statusDict = d["status"] as? [String: Any],
       let statusType = statusDict["type"] as? String {
        switch statusType {
        case "confirmed":
            status = .confirmed(blockHeight: jsonUInt32(statusDict["block_height"]))
        case "waitingForTimelock":
            status = .waitingForTimelock(spendableAtHeight: jsonUInt32(statusDict["spendable_at_height"]))
        default:
            throw exitFileError("unknown status type '\(statusType)'")
        }
    } else if let statusStr = d["status"] as? String {
        switch statusStr {
        case "ready":                   status = .ready
        case "waitingForDependencies":  status = .waitingForDependencies
        case "unverified":              status = .unverified
        default: throw exitFileError("unknown status '\(statusStr)'")
        }
    } else {
        throw exitFileError("missing status")
    }

    return UnilateralExitTransaction(
        kind: kind,
        nodeId: d["node_id"] as? String,
        txid: txid,
        txHex: txHex,
        cpfpTxHex: d["cpfp_tx_hex"] as? String,
        csvTimelockBlocks: jsonUInt32(d["csv_timelock_blocks"]),
        dependsOn: dependsOn,
        status: status
    )
}

private func deserializeCpfpInputFromJson(_ d: [String: Any]) throws -> CpfpInput {
    guard let type = d["type"] as? String,
          let txid = d["txid"] as? String,
          let vout = jsonUInt32(d["vout"]),
          let value = jsonUInt64(d["value"])
    else {
        throw exitFileError("invalid funding input")
    }
    switch type {
    case "p2wpkh":
        guard let pubkey = d["pubkey"] as? String else {
            throw exitFileError("p2wpkh missing pubkey")
        }
        return .p2wpkh(txid: txid, vout: vout, value: value, pubkey: pubkey)
    case "p2tr":
        guard let pubkey = d["pubkey"] as? String else {
            throw exitFileError("p2tr missing pubkey")
        }
        return .p2tr(txid: txid, vout: vout, value: value, pubkey: pubkey)
    case "custom":
        guard let scriptPubkeyHex = d["script_pubkey_hex"] as? String,
              let signedInputWeight = jsonUInt64(d["signed_input_weight"])
        else {
            throw exitFileError("custom input missing fields")
        }
        return .custom(
            txid: txid, vout: vout, value: value,
            scriptPubkeyHex: scriptPubkeyHex,
            signedInputWeight: signedInputWeight
        )
    default:
        throw exitFileError("unknown funding input type '\(type)'")
    }
}

private func jsonUInt64(_ value: Any?) -> UInt64? {
    if let n = value as? UInt64 { return n }
    if let n = value as? Int { return UInt64(n) }
    if let n = value as? NSNumber { return n.uint64Value }
    return nil
}

private func jsonUInt32(_ value: Any?) -> UInt32? {
    if let n = value as? UInt32 { return n }
    if let n = value as? Int { return UInt32(n) }
    if let n = value as? NSNumber { return n.uint32Value }
    return nil
}

private func exitFileError(_ msg: String) -> NSError {
    NSError(domain: "BreezCLI", code: 1, userInfo: [
        NSLocalizedDescriptionKey: "Invalid exit file: \(msg)",
    ])
}

// MARK: - Helpers

private func collectRepeatedFlag(_ args: [String], flag: String) -> [String] {
    var values: [String] = []
    var i = 0
    while i < args.count {
        if args[i] == flag {
            i += 1
            if i < args.count { values.append(args[i]) }
        }
        i += 1
    }
    return values
}

private func parseCpfpInput(_ s: String, _ kind: CpfpFundingKind) -> CpfpInput? {
    let parts = s.split(separator: ":").map(String.init)
    guard parts.count == 4,
          let vout = UInt32(parts[1]),
          let value = UInt64(parts[2]) else {
        return nil
    }
    let txid = parts[0]
    let pubkey = parts[3]
    switch kind {
    case .p2wpkh:
        return .p2wpkh(txid: txid, vout: vout, value: value, pubkey: pubkey)
    case .p2tr:
        return .p2tr(txid: txid, vout: vout, value: value, pubkey: pubkey)
    case .custom:
        return nil
    }
}

private func printExitTransactions(_ response: UnilateralExitResponse) {
    print(
        "Recoverable \(response.recoverableValueSat) sats, " +
        "total fee \(response.totalFeeSat) sats " +
        "(cpfp \(response.cpfpFeeSat), fanout \(response.fanoutFeeSat), sweep \(response.sweepFeeSat)), " +
        "\(response.transactions.count) transaction(s):"
    )
    for (i, tx) in response.transactions.enumerated() {
        let after = tx.dependsOn.isEmpty ? "" : ", after \(tx.dependsOn.joined(separator: ","))"
        let csv = tx.csvTimelockBlocks.map { ", csv \($0) blocks" } ?? ""
        print("  [\(i)] \(tx.kind) status=\(tx.status) txid=\(tx.txid)\(after)\(csv)")
        switch tx.status {
        case let .confirmed(blockHeight):
            if let height = blockHeight {
                print("      (confirmed in block \(height), nothing to broadcast)")
            } else {
                print("      (already confirmed, nothing to broadcast)")
            }
            continue
        case .waitingForDependencies:
            print("      (waiting on the transactions it depends on)")
        case let .waitingForTimelock(spendableAtHeight):
            if let height = spendableAtHeight {
                print("      (waiting for its timelock, until block \(height))")
            } else {
                print("      (waiting for its timelock)")
            }
        case .ready, .unverified:
            break
        }
        let package: String
        if let cpfp = tx.cpfpTxHex {
            package = "\(tx.txHex),\(cpfp)"
        } else {
            package = tx.txHex
        }
        print("      Package: \(package)")
    }
}

private func dataFromHex(_ hex: String) -> Data? {
    let trimmed = hex.trimmingCharacters(in: .whitespaces)
    guard trimmed.count % 2 == 0 else { return nil }
    var data = Data(capacity: trimmed.count / 2)
    var index = trimmed.startIndex
    while index < trimmed.endIndex {
        let nextIndex = trimmed.index(index, offsetBy: 2)
        guard let byte = UInt8(trimmed[index..<nextIndex], radix: 16) else { return nil }
        data.append(byte)
        index = nextIndex
    }
    return data
}
