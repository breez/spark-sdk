use anyhow::Result;
use breez_sdk_spark::*;

async fn quote_exit(sdk: &BreezSdk) -> Result<PrepareUnilateralExitResponse> {
    // ANCHOR: prepare-unilateral-exit
    let quote = sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: 2,
            funding_kind: CpfpFundingKind::P2wpkh,
            destination: "bc1q...your-destination-address".to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;

    println!(
        "Recovering {} sats for {} sats in fees",
        quote.recoverable_value_sat, quote.total_fee_sat
    );
    println!("Fund a single UTXO of at least {} sats", quote.single_utxo_funding_sat);
    // ANCHOR_END: prepare-unilateral-exit

    Ok(quote)
}

async fn build_exit(sdk: &BreezSdk, quote: PrepareUnilateralExitResponse) -> Result<()> {
    // ANCHOR: unilateral-exit
    let secret_key_bytes: Vec<u8> = hex::decode("your-secret-key-hex")?;
    let signer = signer::single_key_cpfp_signer(secret_key_bytes)?;

    let response = sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote,
                funding_inputs: vec![CpfpInput::P2wpkh {
                    txid: "your-utxo-txid".to_string(),
                    vout: 0,
                    value: 50_000,
                    pubkey: "your-compressed-pubkey-hex".to_string(),
                }],
            },
            signer,
        )
        .await?;

    // Store the whole response: it is the only record of the exit.
    for tx in &response.transactions {
        if let Some(blocks) = tx.csv_timelock_blocks {
            println!("{}: wait {} blocks after its parents confirm", tx.txid, blocks);
        }
    }
    // ANCHOR_END: unilateral-exit

    Ok(())
}

async fn check_exit(sdk: &BreezSdk, stored: UnilateralExitResponse) -> Result<()> {
    // ANCHOR: check-unilateral-exit
    let checked = sdk
        .check_unilateral_exit(CheckUnilateralExitRequest { exit: stored })
        .await?;

    // Store this one in place of the one you had.
    let exit = checked.exit;

    match checked.verdict {
        UnilateralExitVerdict::Valid => {
            for tx in &exit.transactions {
                let confirmed = matches!(tx.status, ConfirmationStatus::Confirmed { .. });
                if tx.dependencies_met && !confirmed {
                    // Also wait out csv_timelock_blocks before broadcasting.
                    println!("ready to broadcast: {}", tx.txid);
                }
            }
        }
        UnilateralExitVerdict::Done => {
            println!("The exit finished: {} sats recovered", exit.recoverable_value_sat);
        }
        UnilateralExitVerdict::Redo { reason } => {
            // Quote and build again, naming the same leaves. Pass exit.funding_inputs
            // back and the SDK follows them to whatever they have become.
            println!("Build the exit again: {reason:?}");
        }
    }
    // ANCHOR_END: check-unilateral-exit

    Ok(())
}

async fn export_exit_state(sdk: &BreezSdk) -> Result<String> {
    // ANCHOR: export-unilateral-exit-state
    let exported = sdk.export_unilateral_exit_state().await?;

    // Keep the state somewhere the wallet's own storage cannot take with it.
    println!("Exit state is {} bytes", exported.exit_state.len());
    // ANCHOR_END: export-unilateral-exit-state

    Ok(exported.exit_state)
}

async fn import_exit_state(sdk: &BreezSdk, exit_state: String) -> Result<()> {
    // ANCHOR: import-unilateral-exit-state
    let imported = sdk
        .import_unilateral_exit_state(ImportUnilateralExitStateRequest { exit_state })
        .await?;

    println!(
        "Imported {} leaves, skipped {}",
        imported.imported_leaves, imported.skipped_foreign_leaves
    );
    // ANCHOR_END: import-unilateral-exit-state

    Ok(())
}

// ANCHOR: custom-cpfp-signer
struct MyCpfpSigner;

#[async_trait::async_trait]
impl signer::CpfpSigner for MyCpfpSigner {
    async fn sign_psbt(&self, psbt_bytes: Vec<u8>) -> Result<Vec<u8>, SignerError> {
        let signed_psbt_bytes = sign_psbt_with_your_keys(psbt_bytes)?;
        Ok(signed_psbt_bytes)
    }
}

fn sign_psbt_with_your_keys(psbt_bytes: Vec<u8>) -> Result<Vec<u8>, SignerError> {
    Ok(psbt_bytes)
}
// ANCHOR_END: custom-cpfp-signer
