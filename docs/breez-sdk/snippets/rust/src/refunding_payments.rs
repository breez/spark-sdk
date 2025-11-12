use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

async fn list_unclaimed_deposits(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: list-unclaimed-deposits
    let request = ListUnclaimedDepositsRequest {};
    let response = sdk.list_unclaimed_deposits(request).await?;
    
    for deposit in response.deposits {
        info!("Unclaimed deposit: {}:{}", deposit.txid, deposit.vout);
        info!("Amount: {} sats", deposit.amount_sats);
        
        if let Some(claim_error) = &deposit.claim_error {
            match claim_error {
                DepositClaimError::DepositClaimFeeExceeded { max_fee, actual_fee, .. } => {
                    info!("Max claim fee exceeded. Max: {:?}, Actual: {} sats", max_fee, actual_fee);
                }
                DepositClaimError::MissingUtxo { .. } => {
                    info!("UTXO not found when claiming deposit");
                }
                DepositClaimError::Generic { message } => {
                    info!("Claim failed: {}", message);
                }
            }
        }
    }
    // ANCHOR_END: list-unclaimed-deposits
    Ok(())
}

async fn claim_deposit(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: claim-deposit
    let txid = "your_deposit_txid".to_string();
    let vout = 0;
    
    // Set a higher max fee to retry claiming
    let max_fee = Some(Fee::Fixed { amount: 500 });
    
    let request = ClaimDepositRequest {
        txid,
        vout,
        max_fee,
    };
    
    let response = sdk.claim_deposit(request).await?;
    info!("Deposit claimed successfully. Payment: {:?}", response.payment);
    // ANCHOR_END: claim-deposit
    Ok(())
}

async fn refund_deposit(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: refund-deposit
    let txid = "your_deposit_txid".to_string();
    let vout = 0;
    let destination_address = "bc1qexample...".to_string(); // Your Bitcoin address
    
    // Set the fee for the refund transaction using a rate
    let fee = Fee::Rate { sat_per_vbyte: 5 };
    // or using a fixed amount
    //let fee = Fee::Fixed { amount: 500 };
    
    let request = RefundDepositRequest {
        txid,
        vout,
        destination_address,
        fee,
    };
    
    let response = sdk.refund_deposit(request).await?;
    info!("Refund transaction created:");
    info!("Transaction ID: {}", response.tx_id);
    info!("Transaction hex: {}", response.tx_hex);
    // ANCHOR_END: refund-deposit
    Ok(())
}

async fn recommended_fees(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: recommended-fees
    let response = sdk.recommended_fees().await?;
    info!("Fastest fee: {} sats", response.fastest_fee);
    info!("Half-hour fee: {} sats", response.half_hour_fee);
    info!("Hour fee: {} sats", response.hour_fee);
    info!("Economy fee: {} sats", response.economy_fee);
    info!("Minimum fee: {} sats", response.minimum_fee);
    // ANCHOR_END: recommended-fees
    Ok(())
}