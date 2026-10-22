/**
 * x402 subcommands.
 *
 * Mirrors the Rust CLI `x402` subcommands:
 *   fetch
 */

import {
  X402PaymentAmount,
  X402PaymentStatus,
  X402Request,
} from '@breeztech/breez-sdk-spark-react-native'
import type { BreezSdkInterface } from '@breeztech/breez-sdk-spark-react-native'
import { formatValue } from './serialization'

/** All x402 subcommand names for help and completion. */
export const X402_COMMAND_NAMES = [
  'fetch',
]

const PAYMENT_REQUIRED_HEADER = 'payment-required'
const PAYMENT_SIGNATURE_HEADER = 'payment-signature'
const PAYMENT_RESPONSE_HEADER = 'payment-response'
// Times to pay the same challenge again while its payment is pending. Fetching
// again instead would bring a new challenge with a new invoice to pay.
const PENDING_RETRIES = 3
// Times to send the paid request again while the server answers 402 without a
// new challenge, meaning the payment may still settle.
const SETTLE_RETRIES = 5
const SETTLE_RETRY_DELAY_MS = 2000

const FETCH_USAGE =
  'Usage: x402 fetch <url> --max-amount-sats <n> [-X|--method GET|POST] [-d|--body <body>] [-H|--header "Name: value"]...'

interface HttpResponse {
  status: number
  header: (name: string) => string | null
  body: string
}

/**
 * Dispatch an x402 subcommand.
 *
 * @param args - The arguments after "x402" (e.g., ["fetch", "https://...", "--max-amount-sats", "100"])
 * @param sdk - The BreezSdkInterface instance
 * @returns A string result to display
 */
export async function dispatchX402Command(
  args: string[],
  sdk: BreezSdkInterface
): Promise<string> {
  if (args.length === 0 || args[0] === 'help') {
    return printX402Help()
  }

  const subcommand = args[0]
  const subArgs = args.slice(1)

  switch (subcommand) {
    case 'fetch':
      return handleFetch(sdk, subArgs)
    default:
      return `Unknown x402 subcommand: ${subcommand}. Use 'x402 help' for available commands.`
  }
}

function printX402Help(): string {
  const lines = [
    '',
    'x402 subcommands:',
    '  x402 fetch <url> --max-amount-sats <n> [-X|--method GET|POST] [-d|--body <body>] [-H|--header "Name: value"]...',
    '                                         Fetch a URL, paying its x402 Lightning challenge if the server asks for one',
    '',
  ]
  return lines.join('\n')
}

// --- fetch ---

async function handleFetch(sdk: BreezSdkInterface, args: string[]): Promise<string> {
  const positional: string[] = []
  let maxAmountStr: string | undefined
  let method = 'GET'
  let body: string | undefined
  const headerLines: string[] = []

  for (let i = 0; i < args.length; i++) {
    const arg = args[i]
    const isValueFlag = ['--max-amount-sats', '-X', '--method', '-d', '--body', '-H', '--header'].includes(arg)
    if (!isValueFlag) {
      positional.push(arg)
      continue
    }
    const value = args[++i]
    if (value === undefined) {
      return `Error: ${arg} needs a value\n${FETCH_USAGE}`
    }
    switch (arg) {
      case '--max-amount-sats':
        maxAmountStr = value
        break
      case '-X':
      case '--method':
        method = value
        break
      case '-d':
      case '--body':
        body = value
        break
      default:
        headerLines.push(value)
    }
  }

  const url = positional[0]
  if (url === undefined || maxAmountStr === undefined) {
    return FETCH_USAGE
  }
  if (!/^\d+$/.test(maxAmountStr)) {
    return `Error: invalid --max-amount-sats ${maxAmountStr}`
  }
  const maxAmountSats = BigInt(maxAmountStr)

  // The server hashes the headers it binds, so send every header explicitly:
  // anything fetch added on its own would be missing from the hash computed here.
  let headers: Map<string, string>
  try {
    headers = parseHeaders(headerLines)
  } catch (error: unknown) {
    return `Error: ${errorMessage(error)}`
  }
  if (!headers.has('accept')) {
    headers.set('accept', '*/*')
  }

  // A bytes body keeps fetch from adding a content-type.
  const bodyBytes = body !== undefined ? utf8Bytes(body) : undefined

  const lines: string[] = []
  try {
    let response = await send(method, url, headers, bodyBytes)
    if (response.status !== 402) {
      printResponse(lines, response)
      return lines.join('\n')
    }
    const paymentRequired = response.header(PAYMENT_REQUIRED_HEADER)
    if (paymentRequired === null) {
      throw new Error('402 response has no PAYMENT-REQUIRED header')
    }

    const x402 = sdk.getX402()
    const request = {
      paymentRequired,
      request: new X402Request.Http({
        method,
        url,
        headers: Array.from(headers, ([name, value]) => ({ name, value })),
        body: bodyBytes,
      }),
      maxAmount: new X402PaymentAmount.Lightning({ amountSats: maxAmountSats }),
      conversionOptions: undefined,
      completionTimeoutSecs: undefined,
    }
    let paid = await x402.pay(request)
    for (let i = 0; i < PENDING_RETRIES; i++) {
      if (paid.status === X402PaymentStatus.Paid) {
        break
      }
      lines.push('Payment is pending, waiting for it to complete...')
      paid = await x402.pay(request)
    }
    lines.push(formatValue(paid))
    const signature: string | undefined = paid.paymentSignature
    if (signature === undefined) {
      lines.push('Payment is still pending. Check its status with list-payments.')
      return lines.join('\n')
    }

    headers.set(PAYMENT_SIGNATURE_HEADER, signature)
    response = await send(method, url, headers, bodyBytes)
    for (let i = 0; i < SETTLE_RETRIES; i++) {
      if (response.status !== 402 || response.header(PAYMENT_REQUIRED_HEADER) !== null) {
        break
      }
      lines.push('Payment is not settled yet, sending it again...')
      await new Promise<void>(resolve => setTimeout(() => resolve(), SETTLE_RETRY_DELAY_MS))
      response = await send(method, url, headers, bodyBytes)
    }
    printResponse(lines, response)
  } catch (error: unknown) {
    lines.push(`Error: ${errorMessage(error)}`)
  }
  return lines.join('\n')
}

function parseHeaders(lines: string[]): Map<string, string> {
  const headers = new Map<string, string>()
  for (const line of lines) {
    const idx = line.indexOf(':')
    if (idx === -1) {
      throw new Error(`header ${JSON.stringify(line)} is not "Name: value"`)
    }
    headers.set(line.slice(0, idx).trim().toLowerCase(), line.slice(idx + 1).trim())
  }
  return headers
}

async function send(
  method: string,
  url: string,
  headers: Map<string, string>,
  body: ArrayBuffer | undefined
): Promise<HttpResponse> {
  let response: Response
  switch (method) {
    case 'GET':
      response = await fetch(url, { method, headers: Object.fromEntries(headers) })
      break
    case 'POST':
      response = await fetch(url, { method, headers: Object.fromEntries(headers), body })
      break
    default:
      throw new Error(`unsupported method ${method}: use GET or POST`)
  }
  // fetch follows redirects itself, so one reaching here could not be followed.
  if ([301, 302, 303, 307, 308].includes(response.status)) {
    const location = response.headers.get('location') ?? '<no location>'
    throw new Error(
      `endpoint returned a ${response.status} redirect to ${location}, which this client does not follow`
    )
  }
  return {
    status: response.status,
    header: name => response.headers.get(name),
    body: await response.text(),
  }
}

function printResponse(lines: string[], response: HttpResponse): void {
  lines.push(`HTTP ${response.status}`)
  const settlement = response.header(PAYMENT_RESPONSE_HEADER)
  const decoded = settlement !== null ? decodeBase64(settlement) : undefined
  if (decoded !== undefined) {
    lines.push(`PAYMENT-RESPONSE: ${decoded}`)
  }
  lines.push(response.body)
}

function decodeBase64(value: string): string | undefined {
  let binary: string
  try {
    binary = atob(value)
  } catch {
    return undefined
  }
  // Hermes has no TextDecoder, so decode the UTF-8 bytes through escape.
  try {
    return decodeURIComponent(escape(binary))
  } catch {
    return binary
  }
}

function utf8Bytes(text: string): ArrayBuffer {
  const bytes = new TextEncoder().encode(text)
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
