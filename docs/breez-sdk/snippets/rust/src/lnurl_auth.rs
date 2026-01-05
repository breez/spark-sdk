use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

async fn parse_lnurl_auth(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: parse-lnurl-auth
    // LNURL-auth URL from a service
    // Can be in the form:
    // - lnurl1... (bech32 encoded)
    // - https://service.com/lnurl-auth?tag=login&k1=...
    let lnurl_auth_url = "lnurl1...";

    if let Ok(InputType::LnurlAuth(request_data)) = sdk.parse(lnurl_auth_url).await {
        info!("Domain: {}", request_data.domain);
        info!("Action: {:?}", request_data.action);

        // Show domain to user and ask for confirmation
        // This is important for security
    }
    // ANCHOR_END: parse-lnurl-auth
    Ok(())
}

async fn authenticate(sdk: &BreezSdk, request_data: LnurlAuthRequestDetails) -> Result<()> {
    // ANCHOR: lnurl-auth
    // Perform LNURL authentication
    let result = sdk.lnurl_auth(request_data).await?;

    match result {
        LnurlCallbackStatus::Ok => {
            info!("Authentication successful");
        }
        LnurlCallbackStatus::ErrorStatus { error_details } => {
            info!("Authentication failed: {}", error_details.reason);
        }
    }
    // ANCHOR_END: lnurl-auth
    Ok(())
}
