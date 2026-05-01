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
                fee_rate_sat_per_vbyte: 2,
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
    for leaf in &response.leaves {
        println!(
            "Leaf {}: {} sats (exit cost: ~{} sats)",
            leaf.leaf_id, leaf.value, leaf.estimated_cost
        );
        for tx in &leaf.transactions {
            if let Some(blocks) = tx.csv_timelock_blocks {
                println!("Timelock: wait {} blocks", blocks);
            }
            // tx.tx_hex: pre-signed Spark transaction
            // tx.cpfp_tx_hex: signed CPFP transaction — broadcast alongside parent
        }
    }

    // Check if any node confirmations couldn't be verified
    if !response.unverified_node_ids.is_empty() {
        println!(
            "Warning: could not verify confirmation status for {} nodes",
            response.unverified_node_ids.len()
        );
    }

    // response.sweep_tx_hex: signed sweep transaction for the final step.
    // Broadcast after refund transactions confirm and CSV timelocks expire.
    // ANCHOR_END: prepare-unilateral-exit

    Ok(response)
}
