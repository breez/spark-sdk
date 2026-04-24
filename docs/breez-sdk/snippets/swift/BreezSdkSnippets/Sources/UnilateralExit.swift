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

    // Create a signer from your UTXO private key (32-byte secret key)
    let signer = try SingleKeySigner(secretKeyBytes: Data(hex: "your-secret-key-hex"))

    let response = try await sdk.prepareUnilateralExit(
        request: PrepareUnilateralExitRequest(
            feeRate: 2,
            leafIds: leafIds,
            inputs: [
                .p2wpkh(
                    txid: "your-utxo-txid",
                    vout: 0,
                    value: 50_000,
                    pubkey: "your-compressed-pubkey-hex"
                )
            ],
            destination: "bc1q...your-destination-address"
        ),
        signer: signer
    )

    // The response contains signed transactions ready to broadcast:
    // - response.leaves: parent/child transaction pairs
    // - response.sweepTxHex: signed sweep transaction for the final step
    // Change from CPFP fee-bumping always goes back to the first input's address.
    for leaf in response.leaves {
        for pair in leaf.txCpfpPairs {
            if let blocks = pair.csvTimelockBlocks {
                print("Timelock: wait \(blocks) blocks")
            }
            // pair.parentTxHex: pre-signed Spark transaction
            // pair.childTxHex: signed CPFP transaction — broadcast alongside parent
        }
    }
    // ANCHOR_END: prepare-unilateral-exit
    return response
}
