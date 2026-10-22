'use strict'

const { Command, InvalidArgumentError } = require('commander')
const { printValue } = require('./serialization')

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

/**
 * Register all x402 subcommands on the given commander program.
 *
 * @param {Command} program - The parent commander program (or subcommand)
 * @param {() => object} getSdk - Function that returns the SDK instance
 */
function registerX402Commands(program, getSdk) {
  const x402 = program
    .command('x402')
    .description('x402 related commands')

  // --- fetch ---
  x402
    .command('fetch')
    .description('Fetch a URL, paying its x402 Lightning challenge if the server asks for one')
    .argument('<url>', 'The URL to fetch')
    .requiredOption(
      '--max-amount-sats <sats>',
      'The most to spend in sats, routing fee included',
      parseSats
    )
    .option('-X, --method <method>', 'The HTTP method: GET or POST', 'GET')
    .option('-d, --body <body>', 'The request body')
    .option(
      '-H, --header <header>',
      'A request header as "Name: value". Can be repeated',
      (value, previous = []) => previous.concat([value])
    )
    .action(async (url, options) => {
      await handleFetch(getSdk(), url, options)
    })
}

async function handleFetch(sdk, url, options) {
  const { method, maxAmountSats } = options
  // Bytes rather than a string, so fetch adds no content-type of its own.
  const body = options.body !== undefined ? Buffer.from(options.body, 'utf8') : undefined
  // The server hashes the headers it binds, so send every header
  // explicitly: anything the HTTP client added on its own would be
  // missing from the hash computed here.
  const headers = parseHeaders(options.header ?? [])
  if (!headers.has('accept')) {
    headers.set('accept', '*/*')
  }

  let response = await send(method, url, headers, body)
  if (response.status !== 402) {
    printResponse(response)
    return
  }
  const paymentRequired = response.headers.get(PAYMENT_REQUIRED_HEADER)
  if (paymentRequired === null) {
    throw new Error('402 response has no PAYMENT-REQUIRED header')
  }

  const x402 = sdk.getX402()
  const request = {
    paymentRequired,
    request: {
      type: 'http',
      method,
      url,
      headers: [...headers].map(([name, value]) => ({ name, value })),
      body: body !== undefined ? Array.from(body) : undefined
    },
    maxAmount: { type: 'lightning', amountSats: maxAmountSats },
    conversionOptions: undefined,
    completionTimeoutSecs: undefined
  }
  let paid = await x402.pay(request)
  for (let i = 0; i < PENDING_RETRIES; i++) {
    if (paid.status === 'paid') {
      break
    }
    console.log('Payment is pending, waiting for it to complete...')
    paid = await x402.pay(request)
  }
  printValue(paid)
  if (paid.paymentSignature == null) {
    console.log('Payment is still pending. Check its status with list-payments.')
    return
  }

  headers.set(PAYMENT_SIGNATURE_HEADER, paid.paymentSignature)
  response = await send(method, url, headers, body)
  for (let i = 0; i < SETTLE_RETRIES; i++) {
    if (response.status !== 402 || response.headers.has(PAYMENT_REQUIRED_HEADER)) {
      break
    }
    console.log('Payment is not settled yet, sending it again...')
    await new Promise((resolve) => setTimeout(resolve, SETTLE_RETRY_DELAY_MS))
    response = await send(method, url, headers, body)
  }
  printResponse(response)
}

function parseSats(value) {
  if (!/^\d+$/.test(value)) {
    throw new InvalidArgumentError('Not a whole number of sats.')
  }
  return Number(value)
}

/**
 * @param {string[]} lines - Headers as "Name: value"
 * @returns {Map<string, string>} Lowercased names to values
 */
function parseHeaders(lines) {
  const headers = new Map()
  for (const line of lines) {
    const idx = line.indexOf(':')
    if (idx === -1) {
      throw new Error(`header ${JSON.stringify(line)} is not "Name: value"`)
    }
    headers.set(line.slice(0, idx).trim().toLowerCase(), line.slice(idx + 1).trim())
  }
  return headers
}

async function send(method, url, headers, body) {
  if (method !== 'GET' && method !== 'POST') {
    throw new Error(`unsupported method ${method}: use GET or POST`)
  }
  const response = await fetch(url, {
    method,
    headers: Object.fromEntries(headers),
    body: method === 'POST' ? body : undefined
  })
  // Redirects are followed. One that could not be (no usable location) is an
  // error, as in the Rust client.
  if ([301, 302, 303, 307, 308].includes(response.status)) {
    const location = response.headers.get('location') ?? '<no location>'
    throw new Error(
      `endpoint returned a ${response.status} redirect to ${location}, which this client does not follow`
    )
  }
  return {
    status: response.status,
    headers: response.headers,
    body: await response.text()
  }
}

function printResponse(response) {
  console.log(`HTTP ${response.status}`)
  const settlement = response.headers.get(PAYMENT_RESPONSE_HEADER)
  const decoded = settlement !== null ? decodeBase64(settlement) : undefined
  if (decoded !== undefined) {
    console.log(`PAYMENT-RESPONSE: ${decoded}`)
  }
  console.log(response.body)
}

// Strict standard base64 with padding. Buffer.from skips invalid characters
// instead of rejecting them.
function decodeBase64(value) {
  if (value.length % 4 !== 0 || !/^[A-Za-z0-9+/]*={0,2}$/.test(value)) {
    return undefined
  }
  return Buffer.from(value, 'base64').toString('utf8')
}

module.exports = { registerX402Commands }
