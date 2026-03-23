import { type BreezSdk, WebhookEventType } from '@breeztech/breez-sdk-spark-react-native'

const exampleRegisterWebhook = async (sdk: BreezSdk) => {
  // ANCHOR: register-webhook
  const response = await sdk.registerWebhook({
    url: 'https://example.com/webhook',
    secret: 'your-webhook-secret',
    eventTypes: [new WebhookEventType.LightningReceiveFinished(), new WebhookEventType.LightningSendFinished()]
  })
  console.log(`Webhook registered with ID: ${response.webhookId}`)
  // ANCHOR_END: register-webhook
}

const exampleUnregisterWebhook = async (sdk: BreezSdk) => {
  // ANCHOR: unregister-webhook
  const webhookId = 'webhook-id'
  await sdk.unregisterWebhook({ webhookId })
  console.log('Webhook unregistered')
  // ANCHOR_END: unregister-webhook
}

const exampleListWebhooks = async (sdk: BreezSdk) => {
  // ANCHOR: list-webhooks
  const webhooks = await sdk.listWebhooks()
  for (const webhook of webhooks) {
    console.log(`Webhook: id=${webhook.id}, url=${webhook.url}, events=${String(webhook.eventTypes)}`)
  }
  // ANCHOR_END: list-webhooks
}
