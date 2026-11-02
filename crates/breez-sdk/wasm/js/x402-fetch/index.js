/* global atob */
const PAYMENT_REQUIRED_HEADER = 'PAYMENT-REQUIRED'
const PAYMENT_SIGNATURE_HEADER = 'PAYMENT-SIGNATURE'
const PAYMENT_RESPONSE_HEADER = 'PAYMENT-RESPONSE'
const DEFAULT_PENDING_RETRIES = 3
const DEFAULT_SETTLE_RETRIES = 5
const DEFAULT_SETTLE_RETRY_DELAY_MS = 2000
// The challenge offers nothing this client pays: another scheme, network or
// protocol version.
const OTHER_RAIL_REASONS = ['unsupported_scheme', 'unsupported_network', 'invalid_x402_version']
// The core x402 specification's non-final reason: the same payment is sent again.
const SETTLEMENT_PENDING = 'settlement_pending'
const MAX_U32 = 4294967295
// Any other key of `init` is for the runtime's fetch, such as undici's
// `dispatcher`, and a Request doesn't keep it.
const REQUEST_INIT_KEYS = new Set([
  'method', 'headers', 'body', 'mode', 'credentials', 'cache', 'redirect', 'referrer',
  'referrerPolicy', 'integrity', 'keepalive', 'signal', 'window', 'duplex', 'priority'
])

/**
 * Thrown when a payment may have been made but the request hasn't been served
 * with it: the payment is pending, the server hasn't confirmed it or answered
 * with an error, or something failed after it may have been sent (see
 * `cause`). Nothing is paid again. To resume, pay `payRequest` again if
 * `paymentSignature` is unset, then send the request to `url` with the
 * signature.
 */
export class X402PaymentPendingError extends Error {
  constructor({ message, url, payRequest, payment, paymentSignature, response, cause }) {
    super(message, cause === undefined ? undefined : { cause })
    this.name = 'X402PaymentPendingError'
    this.url = url
    this.response = response
    hideFromLogs(this, { payRequest, payment, paymentSignature })
  }
}

/**
 * Thrown when the server answers the paid request with a 402 that doesn't say
 * the payment may still settle, or with a 400 because it couldn't read the
 * payment. The payment was made, and isn't made again.
 */
export class X402PaymentRefusedError extends Error {
  constructor({ url, payRequest, payment, paymentSignature, errorReason, response }) {
    super(
      errorReason === undefined
        ? 'x402 payment was made, but the server refused it'
        : `x402 payment was made, but the server refused it: ${errorReason}`
    )
    this.name = 'X402PaymentRefusedError'
    this.url = url
    this.errorReason = errorReason
    this.response = response
    hideFromLogs(this, { payRequest, payment, paymentSignature })
  }
}

/**
 * Wraps `fetch` so that a 402 response carrying an x402 Lightning challenge is
 * paid with `x402` and the request is sent again with the payment.
 */
export function withX402(fetchFn, x402, options) {
  const {
    maxAmountSats,
    budgetSats,
    allowedHosts,
    allowHttp = false,
    completionTimeoutSecs,
    pendingRetries = DEFAULT_PENDING_RETRIES,
    settleRetries = DEFAULT_SETTLE_RETRIES,
    settleRetryDelayMs = DEFAULT_SETTLE_RETRY_DELAY_MS
  } = options
  if (!(Number.isSafeInteger(maxAmountSats) && maxAmountSats > 0)) {
    throw new TypeError('maxAmountSats must be a positive whole number')
  }
  if (budgetSats !== undefined && !isCount(budgetSats)) {
    throw new TypeError('budgetSats must be a whole number of at least 0')
  }
  if (
    allowedHosts !== undefined &&
    !(Array.isArray(allowedHosts) && allowedHosts.every((host) => typeof host === 'string'))
  ) {
    throw new TypeError('allowedHosts must be an array of strings')
  }
  if (typeof allowHttp !== 'boolean') {
    throw new TypeError('allowHttp must be a boolean')
  }
  if (
    completionTimeoutSecs !== undefined &&
    !(
      Number.isSafeInteger(completionTimeoutSecs) &&
      completionTimeoutSecs > 0 &&
      completionTimeoutSecs <= MAX_U32
    )
  ) {
    throw new TypeError(`completionTimeoutSecs must be a whole number from 1 to ${MAX_U32}`)
  }
  if (!isCount(pendingRetries)) {
    throw new TypeError('pendingRetries must be a whole number of at least 0')
  }
  if (!isCount(settleRetries)) {
    throw new TypeError('settleRetries must be a whole number of at least 0')
  }
  if (!(Number.isFinite(settleRetryDelayMs) && settleRetryDelayMs >= 0)) {
    throw new TypeError('settleRetryDelayMs must be a number of at least 0')
  }
  let spentSats = 0
  // Caps of payments in progress, so concurrent requests can't overrun the budget.
  let reservedSats = 0

  return async function x402Fetch(input, init) {
    const request = new Request(input, init)
    // The body is hashed into the payment and sent twice, so read it once.
    const body =
      request.body === null ? undefined : new Uint8Array(await request.clone().arrayBuffer())
    const runtimeInit = runtimeOptions(init)

    const response = await fetchFn(request.clone(), runtimeInit)
    if (response.status !== 402) {
      return response
    }
    const paymentRequired = response.headers.get(PAYMENT_REQUIRED_HEADER)
    if (paymentRequired === null) {
      return response
    }
    // The caller is resuming a payment, and the server didn't take it. Paying
    // the new challenge would pay twice.
    if (request.headers.has(PAYMENT_SIGNATURE_HEADER)) {
      return response
    }
    // The challenge is for the URL that answered. After a redirect, fetch
    // hides whether the method changed and which headers were dropped, so
    // only a same-origin GET or HEAD can be bound faithfully.
    const url = response.url || withoutFragment(request.url)
    if (response.redirected && !(sameOrigin(url, request.url) && ['GET', 'HEAD'].includes(request.method))) {
      return response
    }
    const { protocol, host, hostname } = new URL(url)
    if (allowedHosts !== undefined && !allowedHosts.includes(host)) {
      return response
    }
    // The proof of payment is a bearer token, so it isn't sent unencrypted
    // beyond this machine.
    if (protocol === 'http:' && !allowHttp && !isLoopback(hostname)) {
      return response
    }
    const remaining = budgetSats === undefined ? Infinity : budgetSats - spentSats - reservedSats
    const capSats = Math.min(maxAmountSats, remaining)
    if (!(capSats > 0)) {
      return response
    }

    const headers = sentHeaders(request)
    const payRequest = {
      paymentRequired,
      request: {
        type: 'http',
        method: request.method,
        url,
        headers: Array.from(headers, ([name, value]) => ({ name, value })),
        body
      },
      maxAmount: { type: 'lightning', amountSats: capSats },
      conversionOptions: undefined,
      completionTimeoutSecs
    }

    // Reserved before anything is awaited, so concurrent requests see it.
    reservedSats += capSats
    let paid
    try {
      paid = await x402.pay(payRequest)
      // Paying the same challenge again waits on the pending payment. Sending
      // the request again instead would bring a new challenge to pay.
      for (let i = 0; i < pendingRetries && paid.status !== 'paid'; i++) {
        paid = await x402.pay(payRequest)
      }
      // Recorded before the reservation is released, with nothing awaited in
      // between. A pending payment may still complete, so it counts too.
      spentSats += Number(paid.payment.amount) + Number(paid.payment.fees)
    } catch (error) {
      if (OTHER_RAIL_REASONS.includes(error?.x402?.reason)) {
        return response
      }
      await discard(response)
      if (failedBeforePaying(error)) {
        throw error
      }
      spentSats += capSats
      throw new X402PaymentPendingError({
        message: 'x402 payment may have been sent',
        url,
        payRequest,
        cause: error
      })
    } finally {
      reservedSats -= capSats
    }
    await discard(response)
    if (paid.paymentSignature === undefined || paid.paymentSignature === null) {
      throw new X402PaymentPendingError({
        message: 'x402 payment is still pending',
        url,
        payRequest,
        payment: paid.payment
      })
    }
    const paidError = (message, { cause, response } = {}) =>
      new X402PaymentPendingError({
        message,
        url,
        payRequest,
        payment: paid.payment,
        paymentSignature: paid.paymentSignature,
        response,
        cause
      })

    headers.set(PAYMENT_SIGNATURE_HEADER, paid.paymentSignature)
    const paidInit = {
      method: request.method,
      headers,
      body,
      // A service worker's navigation request can't be constructed as such.
      mode: request.mode === 'navigate' ? 'same-origin' : request.mode,
      credentials: request.credentials,
      cache: request.cache,
      // The proof goes only to the server that charged: a redirect answering
      // the paid request is its response, and isn't followed.
      redirect: 'manual',
      referrer: request.referrer,
      referrerPolicy: request.referrerPolicy,
      integrity: request.integrity,
      keepalive: request.keepalive,
      signal: request.signal
    }
    const sendPaid = async () => {
      try {
        return await fetchFn(new Request(url, paidInit), runtimeInit)
      } catch (cause) {
        throw paidError('x402 payment was made, but sending the request with it failed', { cause })
      }
    }
    let paidResponse = await sendPaid()
    for (let i = 0; i < settleRetries && awaitingSettlement(paidResponse); i++) {
      await discard(paidResponse)
      await new Promise((resolve) => setTimeout(resolve, settleRetryDelayMs))
      paidResponse = await sendPaid()
    }
    if (awaitingSettlement(paidResponse)) {
      await discard(paidResponse)
      throw paidError('x402 payment is not yet confirmed by the server')
    }
    // A 400 means the server couldn't read the payment, unless it settled it
    // and the resource itself answered 400.
    const unread = paidResponse.status === 400 && paymentResponse(paidResponse)?.success !== true
    if (paidResponse.status === 402 || unread) {
      throw new X402PaymentRefusedError({
        url,
        payRequest,
        payment: paid.payment,
        paymentSignature: paid.paymentSignature,
        errorReason: paymentErrorReason(paidResponse),
        response: paidResponse
      })
    }
    // Any other error, such as a 500, answers before the payment was used, so
    // it can still be sent again.
    if (paidResponse.status >= 400 && paymentResponse(paidResponse)?.success !== true) {
      throw paidError(`x402 payment was made, but the server answered ${paidResponse.status}`, {
        response: paidResponse
      })
    }
    return paidResponse
  }
}

/**
 * Sets fields carrying the proof of payment or the request's credentials as
 * non-enumerable, so logging or serializing the error leaves them out.
 */
function hideFromLogs(error, fields) {
  for (const [name, value] of Object.entries(fields)) {
    Object.defineProperty(error, name, { value, enumerable: false })
  }
}

function isCount(value) {
  return Number.isSafeInteger(value) && value >= 0
}

/** The keys of `init` that are for the runtime's fetch, or undefined when there are none. */
function runtimeOptions(init) {
  const entries = Object.entries(init ?? {}).filter(([key]) => !REQUEST_INIT_KEYS.has(key))
  return entries.length === 0 ? undefined : Object.fromEntries(entries)
}

/**
 * The headers the request is sent with, which the payment commits to. fetch
 * adds `accept: *\/*` when the request has none.
 */
function sentHeaders(request) {
  const headers = new Headers(request.headers)
  if (!headers.has('accept')) {
    headers.set('accept', '*/*')
  }
  return headers
}

function withoutFragment(url) {
  const parsed = new URL(url)
  parsed.hash = ''
  return parsed.href
}

function sameOrigin(a, b) {
  return new URL(a).origin === new URL(b).origin
}

/** Whether `hostname`, as `URL` normalizes it, is this machine. */
function isLoopback(hostname) {
  return hostname === 'localhost' || hostname === '[::1]' || /^127(\.\d+){3}$/.test(hostname)
}

/**
 * Whether `pay` failed without paying: an x402 refusal or failed payment, or a
 * balance that can't cover it. Any other error may follow a payment.
 */
function failedBeforePaying(error) {
  return error?.x402 !== undefined || String(error?.message).startsWith('Insufficient funds')
}

/** Whether the server answered that the payment may still settle, to be sent again. */
function awaitingSettlement(response) {
  return (
    response.status === 402 &&
    !response.headers.has(PAYMENT_REQUIRED_HEADER) &&
    paymentErrorReason(response) === SETTLEMENT_PENDING
  )
}

/** A response's `PAYMENT-RESPONSE`, decoded from base64 JSON, if it has one. */
function paymentResponse(response) {
  const header = response.headers.get(PAYMENT_RESPONSE_HEADER)
  try {
    return header === null ? undefined : JSON.parse(atob(header))
  } catch {
    return undefined
  }
}

function paymentErrorReason(response) {
  const reason = paymentResponse(response)?.errorReason
  return typeof reason === 'string' ? reason : undefined
}

/** Releases the connection behind a response that won't be read. */
async function discard(response) {
  await response.body?.cancel().catch(() => {})
}
