import BreezSdkSpark
import Foundation

func fetchRecoverableFunds(sdk: BreezSdk) async throws {
    // ANCHOR: recoverable-funds
    let info = try await sdk.getInfo(request: GetInfoRequest(ensureSynced: false))

    if info.recoverableFundsSats > 0 {
        print("\(info.recoverableFundsSats) sats can be recovered on-chain")
    }
    // ANCHOR_END: recoverable-funds
}

func prepareRecovery(sdk: BreezSdk) async throws -> PrepareRecoverFundsResponse {
    // ANCHOR: prepare-recover-funds
    let quote = try await sdk.prepareRecoverFunds(
        request: PrepareRecoverFundsRequest(
            feeRateSatPerVbyte: 2,
            fundingKind: .p2wpkh,
            destination: "bc1q...your-destination-address",
            selection: .recoverableOnly
        )
    )

    if quote.leaves.isEmpty {
        print("Nothing to recover")
        return quote
    }
    for leaf in quote.leaves {
        print("\(leaf.leafId): \(leaf.valueSats) sats, \(leaf.method)")
    }
    print("Recovering \(quote.recoverableValueSats) sats for \(quote.totalFeeSats) sats in fees")
    if let funding = quote.funding {
        print("Fund one UTXO of at least \(funding.singleUtxoSats) sats")
    }
    // ANCHOR_END: prepare-recover-funds

    return quote
}

func recoverCooperatively(sdk: BreezSdk, quote: PrepareRecoverFundsResponse) async throws {
    // ANCHOR: recover-cooperatively
    // A quote with funding holds a unilateral exit: prepare the
    // cooperative leaves alone to recover them without it.
    var quote = quote
    if quote.funding != nil {
        let leafIds = quote.leaves.filter { $0.method == .cooperative }.map { $0.leafId }
        if leafIds.isEmpty {
            return
        }
        quote = try await sdk.prepareRecoverFunds(
            request: PrepareRecoverFundsRequest(
                feeRateSatPerVbyte: quote.feeRateSatPerVbyte,
                destination: quote.destination,
                selection: .specific(leafIds: leafIds)
            )
        )
    }
    let response = try await sdk.recoverFunds(
        request: RecoverFundsRequest(prepared: quote),
        signer: nil
    )

    // Keep the whole response: checkRecoverFunds follows the recovery from it.
    for tx in response.transactions {
        print("Broadcast \(tx.txid): \(tx.txHex)")
    }
    for failure in response.failed {
        print("Leaf \(failure.leafId) was not recovered: \(failure.error)")
    }
    // ANCHOR_END: recover-cooperatively
}

func recoverWithFunding(
    sdk: BreezSdk, quote: PrepareRecoverFundsResponse
) async throws -> RecoverFundsResponse {
    // ANCHOR: recover-funds
    let secretKeyBytes = Data(hexString: "your-secret-key-hex")!
    let signer = try singleKeyCpfpSigner(secretKeyBytes: secretKeyBytes)

    let response = try await sdk.recoverFunds(
        request: RecoverFundsRequest(
            prepared: quote,
            fundingInputs: [
                .p2wpkh(
                    txid: "your-utxo-txid",
                    vout: 0,
                    valueSats: 50_000,
                    pubkey: "your-compressed-pubkey-hex"
                )
            ]
        ),
        signer: signer
    )

    // Keep the whole response: checkRecoverFunds follows the recovery from it.
    for tx in response.transactions {
        if let blocks = tx.csvTimelockBlocks {
            print("\(tx.txid): wait \(blocks) blocks after its parents confirm")
        }
    }
    // ANCHOR_END: recover-funds

    return response
}

func checkRecovery(sdk: BreezSdk, stored: RecoverFundsResponse) async throws {
    // ANCHOR: check-recover-funds
    let checked = try await sdk.checkRecoverFunds(
        request: CheckRecoverFundsRequest(recovery: stored)
    )

    // Store this one in place of the one you had.
    let recovery = checked.recovery

    switch checked.verdict {
    case .valid:
        for tx in recovery.transactions {
            if case .ready = tx.status {
                print("ready to broadcast: \(tx.txid)")
            }
        }
    case .done:
        print("Every transaction confirmed: the recovery is done")
    case .redo(let reason):
        // Prepare and build again, naming the same leaves. Pass recovery.fundingInputs
        // back and the SDK follows them to whatever they have become.
        print("Build the recovery again: \(reason)")
    }
    // ANCHOR_END: check-recover-funds
}

func backUpExitState(sdk: BreezSdk) async throws -> String {
    // ANCHOR: export-exit-state
    let exported = try await sdk.exportUnilateralExitState()

    // Keep the state somewhere the wallet's own storage cannot take with it.
    print("Exit state is \(exported.exitState.utf8.count) bytes")
    // ANCHOR_END: export-exit-state

    return exported.exitState
}

func restoreExitState(sdk: BreezSdk, exitState: String) async throws {
    // ANCHOR: import-exit-state
    let imported = try await sdk.importUnilateralExitState(
        request: ImportUnilateralExitStateRequest(exitState: exitState)
    )

    print("Imported \(imported.importedLeaves) leaves, skipped \(imported.skippedForeignLeaves)")
    // ANCHOR_END: import-exit-state
}

func collectExitData(sdk: BreezSdk) async throws {
    // ANCHOR: sync-exit-data
    // With automatic collection off, an explicit sync is what collects the data
    // a unilateral exit needs, and it waits for the collection to finish. Needs
    // the Spark operators reachable, so run it on a schedule rather than at the
    // moment an exit is needed.
    let _ = try await sdk.syncWallet(request: SyncWalletRequest())
    // ANCHOR_END: sync-exit-data
}

// ANCHOR: custom-cpfp-signer
class MyFundingSigner: CpfpSigner {
    func signPsbt(psbtBytes: Data) async throws -> Data {
        let signedPsbtBytes = try signWithFundingKeys(psbtBytes: psbtBytes)
        return signedPsbtBytes
    }
}

func signWithFundingKeys(psbtBytes: Data) throws -> Data {
    return psbtBytes
}
// ANCHOR_END: custom-cpfp-signer
