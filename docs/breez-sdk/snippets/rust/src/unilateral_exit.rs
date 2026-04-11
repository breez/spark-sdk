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

    let response = sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate: 2,
            leaf_ids,
            utxos: vec![UnilateralExitCpfpUtxo {
                txid: "your-utxo-txid".to_string(),
                vout: 0,
                value: 50_000,
                pubkey: "your-compressed-pubkey-hex".to_string(),
                utxo_type: UnilateralExitCpfpUtxoType::P2wpkh,
            }],
            destination: "bc1q...your-destination-address".to_string(),
        })
        .await?;

    // The response contains:
    // - response.leaves: transaction/PSBT pairs to sign and broadcast
    // - response.sweep_tx_hex: signed sweep transaction for the final step
    for leaf in &response.leaves {
        for pair in &leaf.tx_cpfp_psbts {
            if let Some(blocks) = pair.csv_timelock_blocks {
                println!("Timelock: wait {} blocks", blocks);
            }
            // pair.parent_tx_hex: pre-signed Spark transaction
            // pair.child_psbt_hex: unsigned CPFP PSBT — sign with your UTXO key
        }
    }
    // ANCHOR_END: prepare-unilateral-exit

    Ok(response)
}
