import BreezSdkSpark

func registerWebhook(sdk: BreezSdk) async throws {
    // ANCHOR: register-webhook
    let response = try await sdk.registerWebhook(
        request: RegisterWebhookRequest(
            url: "https://example.com/webhook",
            secret: "your-webhook-secret",
            eventTypes: [.lightningReceiveFinished, .lightningSendFinished]
        ))
    print("Webhook registered with ID: \(response.webhookId)")
    // ANCHOR_END: register-webhook
}

func unregisterWebhook(sdk: BreezSdk) async throws {
    // ANCHOR: unregister-webhook
    let webhookId = "webhook-id"
    try await sdk.unregisterWebhook(
        request: UnregisterWebhookRequest(webhookId: webhookId))
    print("Webhook unregistered")
    // ANCHOR_END: unregister-webhook
}

func listWebhooks(sdk: BreezSdk) async throws {
    // ANCHOR: list-webhooks
    let webhooks = try await sdk.listWebhooks()
    for webhook in webhooks {
        print("Webhook: id=\(webhook.id), url=\(webhook.url), events=\(webhook.eventTypes)")
    }
    // ANCHOR_END: list-webhooks
}
