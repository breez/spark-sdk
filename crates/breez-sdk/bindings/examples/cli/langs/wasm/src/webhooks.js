'use strict'

const { Command } = require('commander')
const { printValue } = require('./serialization')

/**
 * Parse a CLI event type string into the SDK WebhookEventType object.
 *
 * @param {string} s - The event type string
 * @returns {object} The WebhookEventType object
 */
function parseEventType(s) {
  switch (s) {
    case 'lightning-receive':
      return { type: 'lightningReceiveFinished' }
    case 'lightning-send':
      return { type: 'lightningSendFinished' }
    case 'coop-exit':
      return { type: 'coopExitFinished' }
    case 'static-deposit':
      return { type: 'staticDepositFinished' }
    default:
      throw new Error(
        `Unknown event type: ${s}. Valid values: lightning-receive, lightning-send, coop-exit, static-deposit`
      )
  }
}

/**
 * Register all webhooks subcommands on the given commander program.
 *
 * @param {Command} program - The parent commander program (or subcommand)
 * @param {() => object} getSdk - Function that returns the SDK instance
 */
function registerWebhooksCommands(program, getSdk) {
  const webhooks = program
    .command('webhooks')
    .description('Webhook related commands')

  // --- register ---
  webhooks
    .command('register')
    .description('Register a new webhook')
    .argument('<url>', 'URL that will receive webhook notifications')
    .argument('<secret>', 'Secret for HMAC-SHA256 signature verification')
    .argument('<events...>', 'Event types to subscribe to (lightning-receive, lightning-send, coop-exit, static-deposit)')
    .action(async (url, secret, events) => {
      const sdk = getSdk()
      const eventTypes = events.map(parseEventType)
      const response = await sdk.registerWebhook({
        url,
        secret,
        eventTypes
      })
      printValue(response)
    })

  // --- unregister ---
  webhooks
    .command('unregister')
    .description('Unregister a webhook')
    .argument('<webhook_id>', 'ID of the webhook to unregister')
    .action(async (webhookId) => {
      const sdk = getSdk()
      await sdk.unregisterWebhook({ webhookId })
      console.log('Webhook unregistered successfully')
    })

  // --- list ---
  webhooks
    .command('list')
    .description('List all registered webhooks')
    .action(async () => {
      const sdk = getSdk()
      const webhooksList = await sdk.listWebhooks()
      printValue(webhooksList)
    })
}

module.exports = { registerWebhooksCommands }
