use anyhow::Result;
use breez_sdk_spark::*;

async fn prepare_exit(sdk: &BreezSdk) -> Result<PrepareUnilateralExitResponse> {
    // ANCHOR: prepare-unilateral-exit
    // Create a signer from your UTXO private key (32-byte secret key)
    let secret_key_bytes: Vec<u8> = hex::decode("your-secret-key-hex")?;
    let signer = std::sync::Arc::new(signer::SingleKeySigner::new(secret_key_bytes)?);

    let response = sdk
        .prepare_unilateral_exit(
            PrepareUnilateralExitRequest {
                fee_rate: 2,
                inputs: vec![UnilateralExitCpfpInput::P2wpkh {
                    txid: "your-utxo-txid".to_string(),
                    vout: 0,
                    value: 50_000,
                    pubkey: "your-compressed-pubkey-hex".to_string(),
                }],
                destination: "bc1q...your-destination-address".to_string(),
            },
            signer,
        )
        .await?;

    // The SDK automatically selects which leaves are profitable to exit.
    // Review the selected leaves and their estimated costs:
    for leaf in &response.selected_leaves {
        println!(
            "Leaf {}: {} sats (exit cost: ~{} sats)",
            leaf.id, leaf.value, leaf.estimated_cost
        );
    }

    // The response contains signed transactions ready to broadcast:
    // - response.transactions: parent/child transaction pairs per leaf
    // - response.sweep_tx_hex: signed sweep transaction for the final step
    // Change from CPFP fee-bumping always goes back to the first input's address.
    for leaf in &response.transactions {
        for pair in &leaf.tx_cpfp_pairs {
            if let Some(blocks) = pair.csv_timelock_blocks {
                println!("Timelock: wait {} blocks", blocks);
            }
            // pair.parent_tx_hex: pre-signed Spark transaction
            // pair.child_tx_hex: signed CPFP transaction — broadcast alongside parent
        }
    }
    // ANCHOR_END: prepare-unilateral-exit

    Ok(response)
}
