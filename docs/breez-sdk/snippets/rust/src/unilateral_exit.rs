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

    for tx in &response.transactions {
        if let Some(blocks) = tx.csv_timelock_blocks {
            println!("{}: wait {} blocks after its parents confirm", tx.txid, blocks);
        }
    }
    // ANCHOR_END: unilateral-exit

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
