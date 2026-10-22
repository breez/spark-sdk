const PAYMENT_REQUIRED_HEADER = 'PAYMENT-REQUIRED'
const PAYMENT_SIGNATURE_HEADER = 'PAYMENT-SIGNATURE'
const DEFAULT_PENDING_RETRIES = 3
const DEFAULT_SETTLE_RETRIES = 5
const DEFAULT_SETTLE_RETRY_DELAY_MS = 2000

/**
 * Thrown when a payment is still in flight after the configured retries. The
 * payment may still complete, so the request is not sent again.
 */
export class X402PaymentPendingError extends Error {
  constructor(payment) {
    super('x402 payment is still pending')
    this.name = 'X402PaymentPendingError'
    this.payment = payment
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
    completionTimeoutSecs,
    pendingRetries = DEFAULT_PENDING_RETRIES,
    settleRetries = DEFAULT_SETTLE_RETRIES,
    settleRetryDelayMs = DEFAULT_SETTLE_RETRY_DELAY_MS
  } = options
  if (!(maxAmountSats > 0)) {
    throw new TypeError('maxAmountSats must be a positive number')
  }
  let spentSats = 0
  // Caps of payments in progress, so concurrent requests can't overrun the budget.
  let reservedSats = 0

  return async function x402Fetch(input, init) {
    const request = new Request(input, init)
    // The body is hashed into the payment and sent twice, so read it once.
    const body =
      request.body === null ? undefined : new Uint8Array(await request.clone().arrayBuffer())

    const response = await fetchFn(request.clone())
    if (response.status !== 402) {
      return response
    }
    const paymentRequired = response.headers.get(PAYMENT_REQUIRED_HEADER)
    if (paymentRequired === null) {
      return response
    }
    // The challenge is for the URL that answered, which differs from the one
    // requested when the server redirected.
    const url = response.url || request.url
    if (allowedHosts !== undefined && !allowedHosts.includes(new URL(url).host)) {
      return response
    }
    const remaining = budgetSats === undefined ? Infinity : budgetSats - spentSats - reservedSats
    const capSats = Math.min(maxAmountSats, remaining)
    if (!(capSats > 0)) {
      return response
    }

    const headers = []
    request.headers.forEach((value, name) => headers.push({ name, value }))
    const payRequest = {
      paymentRequired,
      request: {
        type: 'http',
        method: request.method,
        url,
        headers,
        body: body === undefined ? undefined : Array.from(body)
      },
      maxAmount: { type: 'lightning', amountSats: capSats },
      conversionOptions: undefined,
      completionTimeoutSecs
    }

    reservedSats += capSats
    let paid
    try {
      paid = await x402.pay(payRequest)
      // Paying the same challenge again picks up a pending payment. Sending the
      // request again instead would bring a new challenge to pay.
      for (let i = 0; i < pendingRetries && paid.status !== 'paid'; i++) {
        paid = await x402.pay(payRequest)
      }
    } finally {
      reservedSats -= capSats
    }
    if (paid.paymentSignature === undefined || paid.paymentSignature === null) {
      throw new X402PaymentPendingError(paid.payment)
    }
    spentSats += Number(paid.payment.amount) + Number(paid.payment.fees)

    const retryHeaders = new Headers(request.headers)
    retryHeaders.set(PAYMENT_SIGNATURE_HEADER, paid.paymentSignature)
    const paidInit = {
      method: request.method,
      headers: retryHeaders,
      body,
      credentials: request.credentials,
      redirect: request.redirect,
      signal: request.signal
    }
    const sendPaid = () => fetchFn(new Request(url, paidInit))
    let paidResponse = await sendPaid()
    // A 402 without a new challenge means the payment may still settle, so the
    // same payment is sent again.
    for (let i = 0; i < settleRetries && awaitingSettlement(paidResponse); i++) {
      await new Promise((resolve) => setTimeout(resolve, settleRetryDelayMs))
      paidResponse = await sendPaid()
    }
    return paidResponse
  }
}

function awaitingSettlement(response) {
  return response.status === 402 && !response.headers.has(PAYMENT_REQUIRED_HEADER)
}
