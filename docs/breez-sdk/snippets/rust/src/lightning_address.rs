use anyhow::anyhow;
use breez_sdk_spark::{BreezSdk, CheckLightningAddressRequest, Config, Network, RegisterLightningAddressRequest, default_config};

pub fn configure_lightning_address() -> Config {
    // ANCHOR: config-lightning-address
    let mut config = default_config(Network::Mainnet);
    config.api_key = Some("your-api-key".to_string());
    config.lnurl_domain = Some("yourdomain.com".to_string());
    // ANCHOR_END: config-lightning-address
    config
}

pub async fn check_lightning_address_availability(sdk: &BreezSdk, username: String) -> anyhow::Result<bool> {
    // Define the username
    let username = username;
    
    // ANCHOR: check-lightning-address
    let request = CheckLightningAddressRequest {
        username,
    };
    
    let is_available = sdk.check_lightning_address_available(request).await?;
    // ANCHOR_END: check-lightning-address
    Ok(is_available)
}


pub async fn register_lightning_address(
    sdk: &BreezSdk,
    username: String,
    description: Option<String>,
) -> anyhow::Result<(String, String)> {
    // Define the parameters
    let username = username;
    let description = description;
    
    // ANCHOR: register-lightning-address
    let request = RegisterLightningAddressRequest {
        username,
        description,
    };
    
    let address_info = sdk.register_lightning_address(request).await?;
    let lightning_address = address_info.lightning_address;
    let lnurl = address_info.lnurl;
    // ANCHOR_END: register-lightning-address
    Ok((lightning_address, lnurl))
}

pub async fn delete_lightning_address(sdk: &BreezSdk) -> anyhow::Result<()> {
    // ANCHOR: delete-lightning-address
    sdk.delete_lightning_address().await?;
    // ANCHOR_END: delete-lightning-address
    Ok(())
}

pub async fn get_lightning_address(sdk: &BreezSdk) -> anyhow::Result<()> {
    // ANCHOR: get-lightning-address
    let address_info_opt = sdk.get_lightning_address().await?;
    
    if let Some(info) = address_info_opt {
        let lightning_address = &info.lightning_address;
        let username = &info.username;
        let description = &info.description;
        let lnurl = &info.lnurl;
    }
    // ANCHOR_END: get-lightning-address
    Ok(())
}
