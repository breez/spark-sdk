import BreezSdkSpark
import Foundation

func listLeavesForExit(sdk: BreezSdk) async throws -> [Leaf] {
    // ANCHOR: list-leaves
    let response = try await sdk.listLeaves(
        request: ListLeavesRequest(minValueSats: 10_000)
    )

    for leaf in response.leaves {
        print("Leaf \(leaf.id): \(leaf.value) sats")
    }
    // ANCHOR_END: list-leaves
    return response.leaves
}

func prepareExit(sdk: BreezSdk) async throws -> PrepareUnilateralExitResponse {
    // ANCHOR: prepare-unilateral-exit
    let leafIds = ["leaf-id-1", "leaf-id-2"]

    let response = try await sdk.prepareUnilateralExit(
        request: PrepareUnilateralExitRequest(
            feeRate: 2,
            leafIds: leafIds,
            utxos: [
                UnilateralExitCpfpUtxo(
                    txid: "your-utxo-txid",
                    vout: 0,
                    value: 50_000,
                    pubkey: "your-compressed-pubkey-hex",
                    utxoType: .p2wpkh
                )
            ],
            destination: "bc1q...your-destination-address"
        )
    )

    // The response contains:
    // - response.leaves: transaction/PSBT pairs to sign and broadcast
    // - response.sweepTxHex: signed sweep transaction for the final step
    for leaf in response.leaves {
        for pair in leaf.txCpfpPsbts {
            if let blocks = pair.csvTimelockBlocks {
                print("Timelock: wait \(blocks) blocks")
            }
            // pair.parentTxHex: pre-signed Spark transaction
            // pair.childPsbtHex: unsigned CPFP PSBT — sign with your UTXO key
        }
    }
    // ANCHOR_END: prepare-unilateral-exit
    return response
}
