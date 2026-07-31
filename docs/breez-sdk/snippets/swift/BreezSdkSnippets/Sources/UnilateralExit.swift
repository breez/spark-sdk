import BreezSdkSpark
import Foundation

func quoteExit(sdk: BreezSdk) async throws -> PrepareUnilateralExitResponse {
    // ANCHOR: prepare-unilateral-exit
    let quote = try await sdk.prepareUnilateralExit(
        request: PrepareUnilateralExitRequest(
            feeRateSatPerVbyte: 2,
            fundingKind: .p2wpkh,
            destination: "bc1q...your-destination-address",
            selection: .auto
        )
    )

    print("Recovering \(quote.recoverableValueSat) sats for \(quote.totalFeeSat) sats in fees")
    print("Fund a single UTXO of at least \(quote.singleUtxoFundingSat) sats")
    // ANCHOR_END: prepare-unilateral-exit
    return quote
}

func buildExit(sdk: BreezSdk, quote: PrepareUnilateralExitResponse) async throws {
    // ANCHOR: unilateral-exit
    let secretKeyBytes = Data(hexString: "your-secret-key-hex")!
    let signer = try singleKeyCpfpSigner(secretKeyBytes: secretKeyBytes)

    let response = try await sdk.unilateralExit(
        request: UnilateralExitRequest(
            prepared: quote,
            fundingInputs: [
                .p2wpkh(
                    txid: "your-utxo-txid",
                    vout: 0,
                    value: 50_000,
                    pubkey: "your-compressed-pubkey-hex"
                )
            ]
        ),
        signer: signer
    )

    for tx in response.transactions {
        if let blocks = tx.csvTimelockBlocks {
            print("\(tx.txid): wait \(blocks) blocks after its parents confirm")
        }
    }
    // ANCHOR_END: unilateral-exit
}

func exportExitState(sdk: BreezSdk) async throws -> String {
    // ANCHOR: export-unilateral-exit-state
    let exported = try await sdk.exportUnilateralExitState()

    // Keep the state somewhere the wallet's own storage cannot take with it.
    print("Exit state is \(exported.exitState.count) bytes")
    // ANCHOR_END: export-unilateral-exit-state

    return exported.exitState
}

func importExitState(sdk: BreezSdk, exitState: String) async throws {
    // ANCHOR: import-unilateral-exit-state
    let imported = try await sdk.importUnilateralExitState(
        request: ImportUnilateralExitStateRequest(exitState: exitState)
    )

    print("Imported \(imported.importedLeaves) leaves, skipped \(imported.skippedLeaves)")
    // ANCHOR_END: import-unilateral-exit-state
}

// ANCHOR: custom-cpfp-signer
class CustomCpfpSigner: CpfpSigner {
    func signPsbt(psbtBytes: Data) async throws -> Data {
        return try await signPsbtWithYourKeys(psbtBytes: psbtBytes)
    }

    private func signPsbtWithYourKeys(psbtBytes: Data) async throws -> Data {
        return psbtBytes
    }
}
// ANCHOR_END: custom-cpfp-signer
