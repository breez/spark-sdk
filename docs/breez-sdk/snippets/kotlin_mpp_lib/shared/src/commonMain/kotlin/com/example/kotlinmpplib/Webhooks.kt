package com.example.kotlinmpplib

import breez_sdk_spark.*

class Webhooks {
    suspend fun registerWebhook(sdk: BreezSdk) {
        // ANCHOR: register-webhook
        val response = sdk.registerWebhook(RegisterWebhookRequest(
            url = "https://example.com/webhook",
            secret = "your-webhook-secret",
            eventTypes = listOf(
                WebhookEventType.LightningReceiveFinished,
                WebhookEventType.LightningSendFinished
            )
        ))
        // Log.v("Breez", "Webhook registered with ID: ${response.webhookId}")
        // ANCHOR_END: register-webhook
    }

    suspend fun unregisterWebhook(sdk: BreezSdk) {
        // ANCHOR: unregister-webhook
        val webhookId = "webhook-id"
        sdk.unregisterWebhook(UnregisterWebhookRequest(webhookId = webhookId))
        // Log.v("Breez", "Webhook unregistered")
        // ANCHOR_END: unregister-webhook
    }

    suspend fun listWebhooks(sdk: BreezSdk) {
        // ANCHOR: list-webhooks
        val webhooks = sdk.listWebhooks()
        for (webhook in webhooks) {
            // Log.v("Breez", "Webhook: id=${webhook.id}, url=${webhook.url}, events=${webhook.eventTypes}")
        }
        // ANCHOR_END: list-webhooks
    }
}
