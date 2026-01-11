use anyhow::Result;
use breez_sdk_spark::{
    default_config, BreezSdk, BurnIssuerTokenRequest, CreateIssuerTokenRequest,
    FreezeIssuerTokenRequest, KeySetConfig, KeySetType, MintIssuerTokenRequest, Network, Payment, SdkBuilder,
    Seed, TokenIssuer, TokenMetadata, UnfreezeIssuerTokenRequest,
};
use log::info;

fn get_token_issuer(sdk: BreezSdk) {
    // ANCHOR: get-token-issuer
    let token_issuer = sdk.get_token_issuer();
    // ANCHOR_END: get-token-issuer
}

async fn create_token(token_issuer: &TokenIssuer) -> Result<TokenMetadata> {
    // ANCHOR: create-token
    let request = CreateIssuerTokenRequest {
        name: "My Token".to_string(),
        ticker: "MTK".to_string(),
        decimals: 6,
        is_freezable: false,
        max_supply: 1_000_000,
    };
    let token_metadata = token_issuer.create_issuer_token(request).await?;
    info!("Token identifier: {}", token_metadata.identifier);
    // ANCHOR_END: create-token
    Ok(token_metadata)
}

async fn create_token_with_custom_account_number() -> Result<BreezSdk> {
    // ANCHOR: custom-account-number
    let account_number = 21;

    let mnemonic = "<mnemonic words>".to_string();
    let seed = Seed::Mnemonic {
        mnemonic,
        passphrase: None,
    };
    let config = default_config(Network::Mainnet);
    let mut builder = SdkBuilder::new(config, seed);
    builder = builder.with_default_storage("./.data".to_string());

    // Set the account number for the SDK
    builder = builder.with_key_set(KeySetConfig {
        key_set_type: KeySetType::Default,
        use_address_index: false,
        account_number: Some(account_number),
    });

    let sdk = builder.build().await?;
    // ANCHOR_END: custom-account-number
    Ok(sdk)
}

async fn mint_token(token_issuer: &TokenIssuer) -> Result<Payment> {
    // ANCHOR: mint-token
    let request = MintIssuerTokenRequest { amount: 1_000 };

    let payment = token_issuer.mint_issuer_token(request).await?;
    // ANCHOR_END: mint-token
    Ok(payment)
}

async fn burn_token(token_issuer: &TokenIssuer) -> Result<Payment> {
    // ANCHOR: burn-token
    let request = BurnIssuerTokenRequest { amount: 1_000 };

    let payment = token_issuer.burn_issuer_token(request).await?;
    // ANCHOR_END: burn-token
    Ok(payment)
}

async fn get_token_metadata(token_issuer: &TokenIssuer) -> Result<TokenMetadata> {
    // ANCHOR: get-token-metadata
    let token_balance = token_issuer.get_issuer_token_balance().await?;
    info!("Token balance: {}", token_balance.balance);

    let token_metadata = token_issuer.get_issuer_token_metadata().await?;
    info!("Token ticker: {}", token_metadata.ticker);
    // ANCHOR_END: get-token-metadata
    Ok(token_metadata)
}

async fn freeze_token(token_issuer: &TokenIssuer) -> Result<()> {
    // ANCHOR: freeze-token
    let spark_address = "<spark address>".to_string();
    // Freeze the tokens held at the specified Spark address
    let freeze_request = FreezeIssuerTokenRequest {
        address: spark_address.clone(),
    };
    let freeze_response = token_issuer.freeze_issuer_token(freeze_request).await?;

    // Unfreeze the tokens held at the specified Spark address
    let unfreeze_request = UnfreezeIssuerTokenRequest {
        address: spark_address,
    };
    let unfreeze_response = token_issuer.unfreeze_issuer_token(unfreeze_request).await?;
    // ANCHOR_END: freeze-token
    Ok(())
}
