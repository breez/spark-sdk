use anyhow::Result;
use breez_sdk_spark::*;

async fn list_leaves_for_exit(sdk: &BreezSdk) -> Result<Vec<Leaf>> {
    // ANCHOR: list-leaves
    let response = sdk
        .list_leaves(ListLeavesRequest {
            min_value_sats: Some(10_000),
        })
        .await?;

    for leaf in &response.leaves {
        println!("Leaf {}: {} sats", leaf.id, leaf.value);
    }
    // ANCHOR_END: list-leaves

    Ok(response.leaves)
}

async fn prepare_exit(sdk: &BreezSdk) -> Result<PrepareUnilateralExitResponse> {
    // ANCHOR: prepare-unilateral-exit
    let leaf_ids = vec!["leaf-id-1".to_string(), "leaf-id-2".to_string()];

    // Create a signer from your UTXO private key (32-byte secret key)
    let secret_key_bytes: Vec<u8> = hex::decode("your-secret-key-hex")?;
    let signer = std::sync::Arc::new(
        signer::SingleKeySigner::new(secret_key_bytes)?,
    );

    let response = sdk
        .prepare_unilateral_exit(
            PrepareUnilateralExitRequest {
                fee_rate: 2,
                leaf_ids,
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

    // The response contains signed transactions ready to broadcast:
    // - response.leaves: parent/child transaction pairs
    // - response.sweep_tx_hex: signed sweep transaction for the final step
    // Change from CPFP fee-bumping always goes back to the first input's address.
    for leaf in &response.leaves {
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
