package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func RegisterWebhook(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.RegisterWebhookResponse, error) {
	// ANCHOR: register-webhook
	response, err := sdk.RegisterWebhook(breez_sdk_spark.RegisterWebhookRequest{
		Url:    "https://example.com/webhook",
		Secret: "your-webhook-secret",
		EventTypes: []breez_sdk_spark.WebhookEventType{
			breez_sdk_spark.WebhookEventTypeLightningReceiveFinished{},
			breez_sdk_spark.WebhookEventTypeLightningSendFinished{},
		},
	})
	if err != nil {
		return nil, err
	}

	log.Printf("Webhook registered with ID: %v", response.WebhookId)
	// ANCHOR_END: register-webhook
	return &response, nil
}

func UnregisterWebhook(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: unregister-webhook
	webhookId := "webhook-id"
	err := sdk.UnregisterWebhook(breez_sdk_spark.UnregisterWebhookRequest{
		WebhookId: webhookId,
	})
	if err != nil {
		return err
	}

	log.Printf("Webhook unregistered")
	// ANCHOR_END: unregister-webhook
	return nil
}

func ListWebhooks(sdk *breez_sdk_spark.BreezSdk) ([]breez_sdk_spark.Webhook, error) {
	// ANCHOR: list-webhooks
	webhooks, err := sdk.ListWebhooks()
	if err != nil {
		return nil, err
	}

	for _, webhook := range webhooks {
		log.Printf("Webhook: id=%v, url=%v, events=%v", webhook.Id, webhook.Url, webhook.EventTypes)
	}
	// ANCHOR_END: list-webhooks
	return webhooks, nil
}
