import BreezSdkSpark
import Foundation

func prepareExit(sdk: BreezSdk) async throws -> PrepareUnilateralExitResponse {
    // ANCHOR: prepare-unilateral-exit
    // Create a signer from your UTXO private key (32-byte secret key)
    let signer = try SingleKeySigner(secretKeyBytes: Data(hex: "your-secret-key-hex"))

    let response = try await sdk.prepareUnilateralExit(
        request: PrepareUnilateralExitRequest(
            feeRate: 2,
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
    for leaf in response.selectedLeaves {
        print("Leaf \(leaf.id): \(leaf.value) sats (exit cost: ~\(leaf.estimatedCost) sats)")
    }

    for leaf in response.transactions {
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
