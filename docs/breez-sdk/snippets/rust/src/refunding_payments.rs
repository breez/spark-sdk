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
                    info!("Claim failed: Fee exceeded. Max: {}, Actual: {}", max_fee, actual_fee);
                }
                DepositClaimError::MissingUtxo { .. } => {
                    info!("Claim failed: UTXO not found");
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
    let max_fee = Some(Fee {
        fee_type: FeeType::Absolute { fee_sat: 500 }, // 500 sats
    });
    
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
    
    // Set the fee for the refund transaction
    let fee = Fee {
        fee_type: FeeType::Absolute { fee_sat: 500 }, // 500 sats
    };
    
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
