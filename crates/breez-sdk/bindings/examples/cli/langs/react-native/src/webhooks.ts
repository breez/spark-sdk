/**
 * Webhook subcommands.
 *
 * Mirrors the Rust CLI `webhooks` subcommands:
 *   register, unregister, list
 */

import { WebhookEventType } from '@breeztech/breez-sdk-spark-react-native'
import type { BreezSdkInterface } from '@breeztech/breez-sdk-spark-react-native'
import { formatValue } from './serialization'

/** All webhook subcommand names for help and completion. */
export const WEBHOOKS_COMMAND_NAMES = [
  'register',
  'unregister',
  'list',
]

/**
 * Parse a named flag from an argument array.
 * Returns the value after the flag, or undefined if not found.
 */
function parseFlag(args: string[], ...flags: string[]): string | undefined {
  for (const flag of flags) {
    const idx = args.indexOf(flag)
    if (idx !== -1 && idx + 1 < args.length) {
      return args[idx + 1]
    }
  }
  return undefined
}

/**
 * Dispatch a webhook subcommand.
 *
 * @param args - The arguments after "webhooks" (e.g., ["register", "https://...", "secret", "lightning-receive"])
 * @param sdk - The BreezSdkInterface instance
 * @returns A string result to display
 */
export async function dispatchWebhooksCommand(
  args: string[],
  sdk: BreezSdkInterface
): Promise<string> {
  if (args.length === 0 || args[0] === 'help') {
    return printWebhooksHelp()
  }

  const subcommand = args[0]
  const subArgs = args.slice(1)

  switch (subcommand) {
    case 'register':
      return handleRegister(sdk, subArgs)
    case 'unregister':
      return handleUnregister(sdk, subArgs)
    case 'list':
      return handleList(sdk)
    default:
      return `Unknown webhooks subcommand: ${subcommand}. Use 'webhooks help' for available commands.`
  }
}

function printWebhooksHelp(): string {
  const lines = [
    '',
    'Webhooks subcommands:',
    '  webhooks register <url> <secret> <event_type> [<event_type> ...]',
    '                                         Register a new webhook',
    '  webhooks unregister <webhook_id>       Unregister a webhook',
    '  webhooks list                          List all registered webhooks',
    '',
    'Event types: lightning-receive, lightning-send, coop-exit, static-deposit',
    '',
  ]
  return lines.join('\n')
}

function parseEventType(s: string): WebhookEventType | undefined {
  switch (s) {
    case 'lightning-receive':
      return new WebhookEventType.LightningReceiveFinished()
    case 'lightning-send':
      return new WebhookEventType.LightningSendFinished()
    case 'coop-exit':
      return new WebhookEventType.CoopExitFinished()
    case 'static-deposit':
      return new WebhookEventType.StaticDepositFinished()
    default:
      return undefined
  }
}

// --- register ---

async function handleRegister(sdk: BreezSdkInterface, args: string[]): Promise<string> {
  // Support both positional and flag-based arguments
  const urlFlag = parseFlag(args, '--url')
  const secretFlag = parseFlag(args, '--secret')

  let url: string | undefined
  let secret: string | undefined
  let eventStrs: string[]

  if (urlFlag && secretFlag) {
    url = urlFlag
    secret = secretFlag
    const eventsFlag = parseFlag(args, '--events')
    eventStrs = eventsFlag ? eventsFlag.split(',').map(s => s.trim()) : []
  } else {
    // Positional: register <url> <secret> <event> [<event> ...]
    if (args.length < 3) {
      return 'Usage: webhooks register <url> <secret> <event_type> [<event_type> ...]\n' +
        'Event types: lightning-receive, lightning-send, coop-exit, static-deposit'
    }
    url = args[0]
    secret = args[1]
    eventStrs = args.slice(2)
  }

  if (!url || !secret || eventStrs.length === 0) {
    return 'Usage: webhooks register <url> <secret> <event_type> [<event_type> ...]'
  }

  const eventTypes: WebhookEventType[] = []
  for (const s of eventStrs) {
    const eventType = parseEventType(s)
    if (!eventType) {
      return `Unknown event type: ${s}. Valid values: lightning-receive, lightning-send, coop-exit, static-deposit`
    }
    eventTypes.push(eventType)
  }

  const result = await sdk.registerWebhook({
    url,
    secret,
    eventTypes,
  })
  return formatValue(result)
}

// --- unregister ---

async function handleUnregister(sdk: BreezSdkInterface, args: string[]): Promise<string> {
  if (args.length < 1) {
    return 'Usage: webhooks unregister <webhook_id>'
  }

  await sdk.unregisterWebhook({ webhookId: args[0] })
  return 'Webhook unregistered successfully'
}

// --- list ---

async function handleList(sdk: BreezSdkInterface): Promise<string> {
  const result = await sdk.listWebhooks()
  return formatValue(result)
}
