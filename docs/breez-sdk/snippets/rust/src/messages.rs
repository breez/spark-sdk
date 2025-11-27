use anyhow::Result;
use breez_sdk_spark::*;
use tracing::info;

#[allow(dead_code)]
async fn sign_message(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: sign-message
    let message = "<message to sign>".to_string();
    // Set to true to get a compact signature rather than a DER
    let compact = true;

    let sign_message_request = SignMessageRequest { message, compact };
    let sign_message_response = sdk.sign_message(sign_message_request).await?;

    let signature = sign_message_response.signature;
    let pubkey = sign_message_response.pubkey;

    info!("Pubkey: {}", pubkey);
    info!("Signature: {}", signature);
    // ANCHOR_END: sign-message
    Ok(())
}

#[allow(dead_code)]
async fn check_message(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: check-message
    let check_message_request = CheckMessageRequest {
        message: "<message>".to_string(),
        pubkey: "<pubkey of signer>".to_string(),
        signature: "<message signature>".to_string(),
    };
    let check_message_response = sdk.check_message(check_message_request).await?;

    let is_valid = check_message_response.is_valid;

    info!("Signature valid: {}", is_valid);
    // ANCHOR_END: check-message
    Ok(())
}
