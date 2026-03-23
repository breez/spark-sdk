use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

pub(crate) async fn register_webhook(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: register-webhook
    let response = sdk
        .register_webhook(RegisterWebhookRequest {
            url: "https://example.com/webhook".to_string(),
            secret: "your-webhook-secret".to_string(),
            event_types: vec![
                WebhookEventType::LightningReceiveFinished,
                WebhookEventType::LightningSendFinished,
            ],
        })
        .await?;
    info!("Webhook registered with ID: {}", response.webhook_id);
    // ANCHOR_END: register-webhook
    Ok(())
}

pub(crate) async fn unregister_webhook(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: unregister-webhook
    let webhook_id = "webhook-id".to_string();
    sdk.unregister_webhook(UnregisterWebhookRequest { webhook_id })
        .await?;
    info!("Webhook unregistered");
    // ANCHOR_END: unregister-webhook
    Ok(())
}

pub(crate) async fn list_webhooks(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: list-webhooks
    let webhooks = sdk.list_webhooks().await?;
    for webhook in webhooks {
        info!(
            "Webhook: id={}, url={}, events={:?}",
            webhook.id, webhook.url, webhook.event_types
        );
    }
    // ANCHOR_END: list-webhooks
    Ok(())
}
