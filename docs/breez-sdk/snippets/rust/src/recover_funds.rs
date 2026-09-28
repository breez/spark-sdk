use anyhow::Result;
use breez_sdk_spark::*;

async fn fetch_recoverable_funds(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: recoverable-funds
    let info = sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;

    if info.recoverable_funds_sats > 0 {
        println!(
            "{} sats can be recovered on-chain",
            info.recoverable_funds_sats
        );
    }
    // ANCHOR_END: recoverable-funds

    Ok(())
}

async fn quote_recovery(sdk: &BreezSdk) -> Result<PrepareRecoverFundsResponse> {
    // ANCHOR: prepare-recover-funds
    let quote = sdk
        .prepare_recover_funds(PrepareRecoverFundsRequest {
            fee_rate_sat_per_vbyte: 2,
            funding_kind: Some(CpfpFundingKind::P2wpkh),
            destination: "bc1q...your-destination-address".to_string(),
            selection: ExitLeafSelection::RecoverableOnly,
        })
        .await?;

    if quote.leaves.is_empty() {
        println!("Nothing to recover");
        return Ok(quote);
    }
    for leaf in &quote.leaves {
        println!(
            "{}: {} sats, {:?}",
            leaf.leaf_id, leaf.value_sats, leaf.method
        );
    }
    println!(
        "Recovering {} sats for {} sats in fees",
        quote.recoverable_value_sats, quote.total_fee_sats
    );
    if let Some(funding) = &quote.funding {
        println!(
            "Fund one UTXO of at least {} sats",
            funding.single_utxo_sats
        );
    }
    // ANCHOR_END: prepare-recover-funds

    Ok(quote)
}

async fn recover_cooperatively(sdk: &BreezSdk, quote: PrepareRecoverFundsResponse) -> Result<()> {
    // ANCHOR: recover-cooperatively
    // A quote that asks for funding holds a unilateral exit: quote the
    // cooperative leaves alone to recover them without it.
    let quote = if quote.funding.is_some() {
        let leaf_ids: Vec<String> = quote
            .leaves
            .iter()
            .filter(|leaf| leaf.method == RecoveryMethod::Cooperative)
            .map(|leaf| leaf.leaf_id.clone())
            .collect();
        if leaf_ids.is_empty() {
            return Ok(());
        }
        sdk.prepare_recover_funds(PrepareRecoverFundsRequest {
            fee_rate_sat_per_vbyte: quote.fee_rate_sat_per_vbyte,
            funding_kind: None,
            destination: quote.destination,
            selection: ExitLeafSelection::Specific { leaf_ids },
        })
        .await?
    } else {
        quote
    };
    let response = sdk
        .recover_funds(
            RecoverFundsRequest {
                prepared: quote,
                funding_inputs: vec![],
            },
            None,
        )
        .await?;

    // Keep the whole response: check_recover_funds follows the recovery from it.
    for tx in &response.transactions {
        println!("Broadcast {}: {}", tx.txid, tx.tx_hex);
    }
    for failure in &response.failed {
        println!(
            "Leaf {} was not recovered: {}",
            failure.leaf_id, failure.error
        );
    }
    // ANCHOR_END: recover-cooperatively

    Ok(())
}

async fn recover_with_funding(
    sdk: &BreezSdk,
    quote: PrepareRecoverFundsResponse,
) -> Result<RecoverFundsResponse> {
    // ANCHOR: recover-funds
    let secret_key_bytes: Vec<u8> = hex::decode("your-secret-key-hex")?;
    let signer = signer::single_key_cpfp_signer(secret_key_bytes)?;

    let response = sdk
        .recover_funds(
            RecoverFundsRequest {
                prepared: quote,
                funding_inputs: vec![CpfpInput::P2wpkh {
                    txid: "your-utxo-txid".to_string(),
                    vout: 0,
                    value_sats: 50_000,
                    pubkey: "your-compressed-pubkey-hex".to_string(),
                }],
            },
            Some(signer),
        )
        .await?;

    // Keep the whole response: check_recover_funds follows the recovery from it.
    for tx in &response.transactions {
        if let Some(blocks) = tx.csv_timelock_blocks {
            println!(
                "{}: wait {} blocks after its parents confirm",
                tx.txid, blocks
            );
        }
    }
    // ANCHOR_END: recover-funds

    Ok(response)
}

async fn check_recovery(sdk: &BreezSdk, stored: RecoverFundsResponse) -> Result<()> {
    // ANCHOR: check-recover-funds
    let checked = sdk
        .check_recover_funds(CheckRecoverFundsRequest { recovery: stored })
        .await?;

    // Store this one in place of the one you had.
    let recovery = checked.recovery;

    match checked.verdict {
        RecoveryVerdict::Valid => {
            for tx in &recovery.transactions {
                if matches!(tx.status, ExitTransactionStatus::Ready) {
                    println!("ready to broadcast: {}", tx.txid);
                }
            }
        }
        RecoveryVerdict::Done => {
            println!("Every transaction confirmed: the recovery is done");
        }
        RecoveryVerdict::Redo { reason } => {
            // Quote and build again, naming the same leaves. Pass
            // recovery.funding_inputs back and the SDK follows them to whatever
            // they have become.
            println!("Build the recovery again: {reason:?}");
        }
    }
    // ANCHOR_END: check-recover-funds

    Ok(())
}

async fn back_up_exit_state(sdk: &BreezSdk) -> Result<String> {
    // ANCHOR: export-exit-state
    let exported = sdk.export_unilateral_exit_state().await?;

    // Keep the state somewhere the wallet's own storage cannot take with it.
    println!("Exit state is {} bytes", exported.exit_state.len());
    // ANCHOR_END: export-exit-state

    Ok(exported.exit_state)
}

async fn restore_exit_state(sdk: &BreezSdk, exit_state: String) -> Result<()> {
    // ANCHOR: import-exit-state
    let imported = sdk
        .import_unilateral_exit_state(ImportUnilateralExitStateRequest { exit_state })
        .await?;

    println!(
        "Imported {} leaves, skipped {}",
        imported.imported_leaves, imported.skipped_foreign_leaves
    );
    // ANCHOR_END: import-exit-state

    Ok(())
}

async fn collect_exit_data(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: sync-exit-data
    // With automatic collection off, an explicit sync is what collects the data
    // a unilateral exit needs, and it waits for the collection to finish. Needs
    // the Spark operators reachable, so run it on a schedule rather than at the
    // moment an exit is needed.
    sdk.sync_wallet(SyncWalletRequest {}).await?;
    // ANCHOR_END: sync-exit-data

    Ok(())
}

// ANCHOR: custom-cpfp-signer
struct MyFundingSigner;

#[async_trait::async_trait]
impl signer::CpfpSigner for MyFundingSigner {
    async fn sign_psbt(&self, psbt_bytes: Vec<u8>) -> Result<Vec<u8>, SignerError> {
        let signed_psbt_bytes = sign_with_funding_keys(psbt_bytes)?;
        Ok(signed_psbt_bytes)
    }
}

fn sign_with_funding_keys(psbt_bytes: Vec<u8>) -> Result<Vec<u8>, SignerError> {
    Ok(psbt_bytes)
}
// ANCHOR_END: custom-cpfp-signer
