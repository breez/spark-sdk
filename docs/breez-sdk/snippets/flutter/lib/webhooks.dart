import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<RegisterWebhookResponse> registerWebhook(BreezSdk sdk) async {
  // ANCHOR: register-webhook
  RegisterWebhookRequest request = RegisterWebhookRequest(
    url: "https://example.com/webhook",
    secret: "your-webhook-secret",
    eventTypes: [
      WebhookEventType.lightningReceiveFinished,
      WebhookEventType.lightningSendFinished,
    ],
  );
  RegisterWebhookResponse response = await sdk.registerWebhook(request: request);
  print("Webhook registered with ID: ${response.webhookId}");
  // ANCHOR_END: register-webhook
  return response;
}

Future<void> unregisterWebhook(BreezSdk sdk) async {
  // ANCHOR: unregister-webhook
  String webhookId = "webhook-id";
  await sdk.unregisterWebhook(
    request: UnregisterWebhookRequest(webhookId: webhookId),
  );
  print("Webhook unregistered");
  // ANCHOR_END: unregister-webhook
}

Future<List<Webhook>> listWebhooks(BreezSdk sdk) async {
  // ANCHOR: list-webhooks
  List<Webhook> webhooks = await sdk.listWebhooks();
  for (Webhook webhook in webhooks) {
    print("Webhook: id=${webhook.id}, url=${webhook.url}, events=${webhook.eventTypes}");
  }
  // ANCHOR_END: list-webhooks
  return webhooks;
}
