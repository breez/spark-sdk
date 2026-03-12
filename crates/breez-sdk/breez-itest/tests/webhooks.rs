use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rstest::*;
use tracing::info;

/// Test the full register / list / delete webhook lifecycle.
///
/// Starts the SDK *without* `support_lnurl_verify` so no automatic LNURL
/// webhook is created. Then manually registers a webhook, verifies it appears
/// in `list_webhooks`, deletes it, and verifies it's gone.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_01_webhook_register_list_delete(
    #[future] alice_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_01_webhook_register_list_delete ===");

    let alice = alice_sdk.await?;

    // No webhooks should exist initially
    let response = alice.sdk.list_webhooks().await?;
    assert!(
        response.webhooks.is_empty(),
        "Expected no webhooks initially, got {} webhook(s)",
        response.webhooks.len(),
    );
    info!("Confirmed no webhooks exist initially");

    // Register a webhook
    let webhook_url = "https://example.com/webhook";
    let event_types = vec![
        WebhookEventType::LightningReceiveFinished,
        WebhookEventType::LightningSendFinished,
    ];

    let register_response = alice
        .sdk
        .register_webhook(RegisterWebhookRequest {
            url: webhook_url.to_string(),
            event_types: event_types.clone(),
        })
        .await?;

    let webhook_id = register_response.webhook_id;
    info!("Registered webhook: {webhook_id}");

    // List webhooks — the newly registered one should be present
    let response = alice.sdk.list_webhooks().await?;
    assert_eq!(
        response.webhooks.len(),
        1,
        "Expected exactly one webhook, got {}",
        response.webhooks.len(),
    );

    let webhook = &response.webhooks[0];
    assert_eq!(webhook.id, webhook_id);
    assert_eq!(webhook.url, webhook_url);
    assert!(
        webhook
            .event_types
            .contains(&WebhookEventType::LightningReceiveFinished),
        "Webhook should contain LightningReceiveFinished",
    );
    assert!(
        webhook
            .event_types
            .contains(&WebhookEventType::LightningSendFinished),
        "Webhook should contain LightningSendFinished",
    );
    info!("Verified webhook is listed with correct fields");

    // Delete the webhook
    let delete_response = alice
        .sdk
        .delete_webhook(DeleteWebhookRequest {
            webhook_id: webhook_id.clone(),
        })
        .await?;

    assert!(delete_response.success, "Expected delete to succeed");
    info!("Deleted webhook: {webhook_id}");

    // List again — should be empty
    let response = alice.sdk.list_webhooks().await?;
    assert!(
        response.webhooks.is_empty(),
        "Expected no webhooks after deletion, got {}",
        response.webhooks.len(),
    );
    info!("Confirmed webhook is gone after deletion");

    info!("=== Test test_01_webhook_register_list_delete PASSED ===");
    Ok(())
}
