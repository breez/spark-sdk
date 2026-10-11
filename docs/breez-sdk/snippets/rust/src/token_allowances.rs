use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

async fn create_token_allowance(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: create-token-allowance
    let response = sdk
        .create_token_allowance(CreateTokenAllowanceRequest {
            spender_address: "<spender spark address>".to_string(),
            token_identifier: "<token identifier>".to_string(),
            max_per_payment: TokenAllowanceLimit::Amount { amount: 5_000_000 },
            max_total: TokenAllowanceLimit::Amount {
                amount: 100_000_000,
            },
            expiry_time: 1_798_761_600,
            allowed_recipients: vec![],
        })
        .await?;
    info!("Allowance id: {}", response.allowance.id);
    // ANCHOR_END: create-token-allowance
    Ok(())
}

async fn list_token_allowances(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: list-token-allowances
    let response = sdk
        .list_token_allowances(ListTokenAllowancesRequest {
            role: TokenAllowanceRole::Owner,
            counterparty_address: None,
            token_identifier: None,
            include_inactive: Some(false),
            offset: None,
            limit: None,
        })
        .await?;
    for allowance in response.allowances {
        info!("{}: spent {}", allowance.id, allowance.spent_amount);
    }
    // ANCHOR_END: list-token-allowances
    Ok(())
}

async fn revoke_token_allowance(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: revoke-token-allowance
    sdk.revoke_token_allowance(RevokeTokenAllowanceRequest {
        allowance_id: "<allowance id>".to_string(),
    })
    .await?;
    // ANCHOR_END: revoke-token-allowance
    Ok(())
}

async fn pull_payment(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: pull-payment
    let prepare_response = sdk
        .prepare_pull_payment(PreparePullPaymentRequest {
            payer_address: "<payer spark address>".to_string(),
            token_identifier: "<token identifier>".to_string(),
            receivers: vec![PullReceiver {
                amount: 5_000_000,
                receiver_address: None,
            }],
        })
        .await?;
    info!("Pulling {}", prepare_response.amount);

    let response = sdk
        .pull_payment(PullPaymentRequest { prepare_response })
        .await?;
    info!("Pull transaction: {}", response.tx_hash);
    // ANCHOR_END: pull-payment
    Ok(())
}
