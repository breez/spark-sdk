using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class Webhooks
    {
        async Task RegisterWebhook(BreezSdk sdk)
        {
            // ANCHOR: register-webhook
            var response = await sdk.RegisterWebhook(request: new RegisterWebhookRequest(
                url: "https://example.com/webhook",
                secret: "your-webhook-secret",
                eventTypes: new WebhookEventType[]
                {
                    new WebhookEventType.LightningReceiveFinished(),
                    new WebhookEventType.LightningSendFinished()
                }
            ));
            Console.WriteLine($"Webhook registered with ID: {response.webhookId}");
            // ANCHOR_END: register-webhook
        }

        async Task UnregisterWebhook(BreezSdk sdk)
        {
            // ANCHOR: unregister-webhook
            var webhookId = "webhook-id";
            await sdk.UnregisterWebhook(request: new UnregisterWebhookRequest(
                webhookId: webhookId
            ));
            Console.WriteLine("Webhook unregistered");
            // ANCHOR_END: unregister-webhook
        }

        async Task ListWebhooks(BreezSdk sdk)
        {
            // ANCHOR: list-webhooks
            var webhooks = await sdk.ListWebhooks();
            foreach (var webhook in webhooks)
            {
                Console.WriteLine($"Webhook: id={webhook.id}, url={webhook.url}, events={webhook.eventTypes}");
            }
            // ANCHOR_END: list-webhooks
        }
    }
}
