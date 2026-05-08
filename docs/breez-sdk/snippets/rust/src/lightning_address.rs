use breez_sdk_spark::{
    default_config, AcceptLightningAddressTransferRequest, BreezSdk, CheckLightningAddressRequest,
    Config, GetPaymentRequest, LightningAddressTransfer, Network, PaymentDetails,
    RegisterLightningAddressRequest,
};

pub fn configure_lightning_address() -> Config {
    // ANCHOR: config-lightning-address
    let mut config = default_config(Network::Mainnet);
    config.api_key = Some("your-api-key".to_string());
    config.lnurl_domain = Some("yourdomain.com".to_string());
    // ANCHOR_END: config-lightning-address
    config
}

pub async fn check_lightning_address_availability(sdk: &BreezSdk) -> anyhow::Result<bool> {
    // Define the username
    let username = "a username".to_string();

    // ANCHOR: check-lightning-address
    let request = CheckLightningAddressRequest { username };

    let is_available = sdk.check_lightning_address_available(request).await?;
    // ANCHOR_END: check-lightning-address
    Ok(is_available)
}

pub async fn register_lightning_address(sdk: &BreezSdk) -> anyhow::Result<(String, String, String)> {
    // Define the parameters
    let username = "a username".to_string();
    let description = Some("Lightning address description".to_string());

    // ANCHOR: register-lightning-address
    let request = RegisterLightningAddressRequest {
        username,
        description,
        transfer: None,
    };

    let address_info = sdk.register_lightning_address(request).await?;
    let lightning_address = address_info.lightning_address;
    let lnurl_url = address_info.lnurl.url;
    let lnurl_bech32 = address_info.lnurl.bech32;
    // ANCHOR_END: register-lightning-address
    Ok((lightning_address, lnurl_url, lnurl_bech32))
}

// Step 1 — run on the *current owner's* wallet. Produces the authorization
// the new owner needs to take over the username in a single atomic call.
pub async fn sign_lightning_address_transfer(
    current_owner_sdk: &BreezSdk,
    transferee_pubkey: &str,
) -> anyhow::Result<LightningAddressTransfer> {
    // ANCHOR: sign-lightning-address-transfer
    let transfer = current_owner_sdk
        .accept_lightning_address_transfer(AcceptLightningAddressTransferRequest {
            transferee_pubkey: transferee_pubkey.to_string(),
        })
        .await?;
    // ANCHOR_END: sign-lightning-address-transfer
    Ok(transfer)
}

// Step 2 — run on the *new owner's* wallet with the authorization received
// from the current owner (e.g. via QR code or deep link).
pub async fn register_lightning_address_via_transfer(
    new_owner_sdk: &BreezSdk,
    transfer: LightningAddressTransfer,
) -> anyhow::Result<(String, String, String)> {
    let username = "a username".to_string();
    let description = Some("Lightning address description".to_string());

    // ANCHOR: register-lightning-address-transfer
    let request = RegisterLightningAddressRequest {
        username,
        description,
        transfer: Some(transfer),
    };

    let address_info = new_owner_sdk.register_lightning_address(request).await?;
    let lightning_address = address_info.lightning_address;
    let lnurl_url = address_info.lnurl.url;
    let lnurl_bech32 = address_info.lnurl.bech32;
    // ANCHOR_END: register-lightning-address-transfer
    Ok((lightning_address, lnurl_url, lnurl_bech32))
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
        let lnurl_url = &info.lnurl.url;
        let lnurl_bech32 = &info.lnurl.bech32;
    }
    // ANCHOR_END: get-lightning-address
    Ok(())
}

pub async fn access_sender_comment(sdk: &BreezSdk) -> anyhow::Result<()> {
    let payment_id = "<payment id>".to_string();
    let response = sdk.get_payment(GetPaymentRequest { payment_id }).await?;
    let payment = response.payment;

    // ANCHOR: access-sender-comment
    // Check if this is a lightning payment with LNURL receive metadata
    if let Some(PaymentDetails::Lightning {
        lnurl_receive_metadata: Some(metadata),
        ..
    }) = payment.details
    {
        // Access the sender comment if present
        if let Some(comment) = metadata.sender_comment {
            println!("Sender comment: {}", comment);
        }
    }
    // ANCHOR_END: access-sender-comment
    Ok(())
}

pub async fn access_nostr_zap(sdk: &BreezSdk) -> anyhow::Result<()> {
    let payment_id = "<payment id>".to_string();
    let response = sdk.get_payment(GetPaymentRequest { payment_id }).await?;
    let payment = response.payment;

    // ANCHOR: access-nostr-zap
    // Check if this is a lightning payment with LNURL receive metadata
    if let Some(PaymentDetails::Lightning {
        lnurl_receive_metadata: Some(metadata),
        ..
    }) = payment.details
    {
        // Access the Nostr zap request if present
        if let Some(zap_request) = metadata.nostr_zap_request {
            // The zap_request is a JSON string containing the Nostr event (kind 9734)
            println!("Nostr zap request: {}", zap_request);
        }

        // Access the Nostr zap receipt if present
        if let Some(zap_receipt) = metadata.nostr_zap_receipt {
            // The zap_receipt is a JSON string containing the Nostr event (kind 9735)
            println!("Nostr zap receipt: {}", zap_receipt);
        }
    }
    // ANCHOR_END: access-nostr-zap
    Ok(())
}
