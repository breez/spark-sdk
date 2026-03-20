import logging
from breez_sdk_spark import (
    BreezSdk,
    RegisterWebhookRequest,
    UnregisterWebhookRequest,
    WebhookEventType,
)


async def register_webhook(sdk: BreezSdk):
    # ANCHOR: register-webhook
    response = await sdk.register_webhook(
        request=RegisterWebhookRequest(
            url="https://example.com/webhook",
            secret="your-webhook-secret",
            event_types=[
                WebhookEventType.LIGHTNING_RECEIVE_FINISHED,
                WebhookEventType.LIGHTNING_SEND_FINISHED,
            ],
        )
    )
    logging.debug(f"Webhook registered with ID: {response.webhook_id}")
    # ANCHOR_END: register-webhook


async def unregister_webhook(sdk: BreezSdk):
    # ANCHOR: unregister-webhook
    webhook_id = "webhook-id"
    await sdk.unregister_webhook(
        request=UnregisterWebhookRequest(webhook_id=webhook_id)
    )
    logging.debug("Webhook unregistered")
    # ANCHOR_END: unregister-webhook


async def list_webhooks(sdk: BreezSdk):
    # ANCHOR: list-webhooks
    webhooks = await sdk.list_webhooks()
    for webhook in webhooks:
        logging.debug(
            f"Webhook: id={webhook.id}, url={webhook.url}, "
            f"events={webhook.event_types}"
        )
    # ANCHOR_END: list-webhooks
