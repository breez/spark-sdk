import BreezSdkSpark
import Foundation

func prepareExit(sdk: BreezSdk) async throws -> PrepareUnilateralExitResponse {
    // ANCHOR: prepare-unilateral-exit
    // Create a signer from your UTXO private key (32-byte secret key)
    let secretKeyBytes = Data(count: 32) // Replace with your actual secret key bytes
    let signer = try SingleKeySigner(secretKeyBytes: secretKeyBytes)

    let response = try await sdk.prepareUnilateralExit(
        request: PrepareUnilateralExitRequest(
            feeRateSatPerVbyte: 2,
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

    // The SDK automatically selects which leaves are profitable to exit.
    for leaf in response.leaves {
        print("Leaf \(leaf.leafId): \(leaf.value) sats (exit cost: ~\(leaf.estimatedCost) sats)")
        for tx in leaf.transactions {
            if let blocks = tx.csvTimelockBlocks {
                print("Timelock: wait \(blocks) blocks")
            }
            // tx.txHex: pre-signed Spark transaction
            // tx.cpfpTxHex: signed CPFP transaction — broadcast alongside parent
        }
    }

    if !response.unverifiedNodeIds.isEmpty {
        print("Warning: could not verify confirmation status for \(response.unverifiedNodeIds.count) nodes")
    }
    // ANCHOR_END: prepare-unilateral-exit
    return response
}
