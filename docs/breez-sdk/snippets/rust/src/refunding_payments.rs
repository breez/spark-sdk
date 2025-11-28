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
                DepositClaimError::MaxDepositClaimFeeExceeded { max_fee, required_fee, .. } => {
                    info!("Max claim fee exceeded. Max: {:?} sats, Required: {} sats", max_fee, required_fee);
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

async fn handle_fee_exceeded(sdk: &BreezSdk, deposit: &DepositInfo) -> Result<()> {
    // ANCHOR: handle-fee-exceeded
    if let Some(DepositClaimError::MaxDepositClaimFeeExceeded { required_fee, .. }) = &deposit.claim_error {
        // Show UI to user with the required fee and get approval
        let user_approved = true; // Replace with actual user approval logic

        if user_approved {
            let request = ClaimDepositRequest {
                txid: deposit.txid.clone(),
                vout: deposit.vout,
                max_fee: Some(Fee::Fixed { amount: *required_fee }),
            };
            sdk.claim_deposit(request).await?;
        }
    }
    // ANCHOR_END: handle-fee-exceeded
    Ok(())
}

async fn claim_deposit(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: claim-deposit
    let txid = "your_deposit_txid".to_string();
    let vout = 0;
    
    // Set a higher max fee to retry claiming
    let max_fee = Some(Fee::Fixed { amount: 5_000 });
    
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

async fn recommended_fees_example() -> Result<()> {
    // ANCHOR: recommended-fees
    let response = recommended_fees(network: Network::Mainnet).await?;
    info!("Fastest fee: {} sats/vByte", response.fastest_fee);
    info!("Half-hour fee: {} sats/vByte", response.half_hour_fee);
    info!("Hour fee: {} sats/vByte", response.hour_fee);
    info!("Economy fee: {} sats/vByte", response.economy_fee);
    info!("Minimum fee: {} sats/vByte", response.minimum_fee);
    // ANCHOR_END: recommended-fees
    Ok(())
}