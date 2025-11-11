use anyhow::Result;
use breez_sdk_spark::{
    BreezIssuerSdk, BreezSdk, BurnIssuerTokenRequest, CreateIssuerTokenRequest, FreezeIssuerTokenRequest,
    MintIssuerTokenRequest, Payment, TokenMetadata, UnfreezeIssuerTokenRequest,
};
use log::info;

fn get_issuer_sdk(sdk: BreezSdk) -> BreezIssuerSdk {
    // ANCHOR: get-issuer-sdk
    let issuer_sdk = sdk.get_issuer_sdk();
    // ANCHOR_END: get-issuer-sdk
    issuer_sdk
}

async fn create_token(issuer_sdk: &BreezIssuerSdk) -> Result<TokenMetadata> {
    // ANCHOR: create-token
    let request = CreateIssuerTokenRequest {
        name: "My Token".to_string(),
        ticker: "MTK".to_string(),
        decimals: 6,
        is_freezable: true,
        total_supply: 1_000_000,
    };
    let token_metadata = issuer_sdk.create_issuer_token(request).await?;
    info!("Token identifier: {}", token_metadata.identifier);
    // ANCHOR_END: create-token
    Ok(token_metadata)
}

async fn mint_token(issuer_sdk: &BreezIssuerSdk) -> Result<Payment> {
    // ANCHOR: mint-token
    let request = MintIssuerTokenRequest {
        amount: 1_000,
    };

    let payment = issuer_sdk
        .mint_issuer_token(request)
        .await?;
    // ANCHOR_END: mint-token
    Ok(payment)
}

async fn burn_token(issuer_sdk: &BreezIssuerSdk) -> Result<Payment> {
    // ANCHOR: burn-token
    let request = BurnIssuerTokenRequest {
        amount: 1_000,
    };

    let payment = issuer_sdk
        .burn_issuer_token(request)
        .await?;
    // ANCHOR_END: burn-token
    Ok(payment)
}

async fn get_token_metadata(issuer_sdk: &BreezIssuerSdk) -> Result<TokenMetadata> {
    // ANCHOR: get-token-metadata
    let token_balance = issuer_sdk.get_issuer_token_balance().await?;
    info!("Token balance: {}", token_balance.balance);

    let token_metadata = issuer_sdk.get_issuer_token_metadata().await?;
    info!("Token ticker: {}", token_metadata.ticker);
    // ANCHOR_END: get-token-metadata
    Ok(token_metadata)
}

async fn freeze_token(issuer_sdk: &BreezIssuerSdk) -> Result<()> {
    // ANCHOR: freeze-token
    let spark_address = "<spark address>".to_string();
    // Freeze the tokens held at the specified Spark address
    let freeze_request = FreezeIssuerTokenRequest {
        address: spark_address.clone(),
    };
    let freeze_response = issuer_sdk.freeze_issuer_token(freeze_request).await?;

    // Unfreeze the tokens held at the specified Spark address
    let unfreeze_request = UnfreezeIssuerTokenRequest {
        address: spark_address,
    };
    let unfreeze_response = issuer_sdk.unfreeze_issuer_token(unfreeze_request).await?;
    // ANCHOR_END: freeze-token
    Ok(())
}