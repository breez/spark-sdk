import Foundation
import BreezSdkSpark

// MARK: - Advanced command names (for REPL completion)

let advancedCommandNames: [String] = [
    "advanced unilateral-exit",
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
    print("  advanced \("unilateral-exit".padding(toLength: 30, withPad: " ", startingAt: 0))Build and sign a unilateral exit")
    print()
}

// MARK: - Handlers

private func handleUnilateralExit(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard let feeRateStr = fp.get("fee-rate"),
          let feeRate = UInt64(feeRateStr) else {
        print("Usage: advanced unilateral-exit --fee-rate <sat/vbyte> --destination <address> [--funding-kind p2wpkh|p2tr] [--leaf <id> ...]")
        return
    }
    guard let destination = fp.get("destination") else {
        print("Usage: advanced unilateral-exit --fee-rate <sat/vbyte> --destination <address> [--funding-kind p2wpkh|p2tr] [--leaf <id> ...]")
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
        "total fee \(response.totalFeeSat) sats, " +
        "\(response.transactions.count) transaction(s):"
    )
    for (i, tx) in response.transactions.enumerated() {
        let after = tx.dependsOn.isEmpty ? "" : ", after \(tx.dependsOn.joined(separator: ","))"
        let csv = tx.csvTimelockBlocks.map { ", csv \($0) blocks" } ?? ""
        print("  [\(i)] \(tx.kind) status=\(tx.status) txid=\(tx.txid)\(after)\(csv)")
        if case .confirmed = tx.status {
            print("      (already confirmed, nothing to broadcast)")
            continue
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
