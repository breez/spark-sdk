const PAYMENT_REQUIRED_HEADER = 'PAYMENT-REQUIRED'
const PAYMENT_SIGNATURE_HEADER = 'PAYMENT-SIGNATURE'
const PAYMENT_RESPONSE_HEADER = 'PAYMENT-RESPONSE'
const EXPOSE_HEADERS = 'Access-Control-Expose-Headers'
// The payment may still settle, so the client should retry with the same proof
// rather than be handed a new invoice to pay.
const RETRYABLE_REASONS = ['exact_lnbtc_invoice_not_settled', 'exact_lnbtc_receiver_unavailable']
// A payment that can't be read is a bad request rather than a failed payment.
const MALFORMED_REASONS = [
  'invalid_payload',
  'invalid_x402_version',
  'invalid_exact_lnbtc_preimage_missing',
  'invalid_exact_lnbtc_preimage_malformed',
  'invalid_exact_lnbtc_preimage_length'
]

/**
 * Charges for a request with x402 over Lightning. Resolves to `{ paid: true,
 * paymentResponse }` when the request carries a valid payment, or to
 * `{ paid: false, response }` with the 402 response to send instead.
 */
export async function requirePayment(request, x402, options) {
  const { publicOrigin, priceSats, boundHeaders = [], description, mimeType, maxTimeoutSecs } =
    options
  // The bound URL is the public one: its origin comes from configuration, never
  // from the Host or forwarding headers, which the client controls.
  const target = new URL(request.url)
  const url = new URL(target.pathname + target.search, publicOrigin).toString()
  const headers = []
  request.headers.forEach((value, name) => {
    if (name !== PAYMENT_SIGNATURE_HEADER.toLowerCase()) {
      headers.push({ name, value })
    }
  })
  const body =
    request.body === null ? undefined : Array.from(new Uint8Array(await request.clone().arrayBuffer()))
  const x402Request = { type: 'http', method: request.method, url, headers, body }
  const boundNames = [...boundHeaders].map((name) => name.toLowerCase()).sort()

  const challenge = async (extraHeaders) => {
    const amountSats = typeof priceSats === 'function' ? await priceSats(request) : priceSats
    const created = await x402.createChallenge({
      request: x402Request,
      boundNames,
      amounts: [{ type: 'lightning', amountSats }],
      maxTimeoutSecs,
      description,
      mimeType
    })
    return new Response(null, {
      status: 402,
      headers: { ...extraHeaders, [PAYMENT_REQUIRED_HEADER]: created.paymentRequired }
    })
  }

  const payment = request.headers.get(PAYMENT_SIGNATURE_HEADER)
  const exposed = { [EXPOSE_HEADERS]: exposeX402Headers() }
  if (payment === null) {
    return { paid: false, response: await challenge(exposed) }
  }
  const settled = await x402.settle({ payment, request: x402Request, boundNames })
  if (settled.success) {
    return { paid: true, paymentResponse: settled.paymentResponse }
  }
  const failure = { ...exposed, [PAYMENT_RESPONSE_HEADER]: settled.paymentResponse }
  if (MALFORMED_REASONS.includes(settled.errorReason)) {
    return { paid: false, response: new Response(null, { status: 400, headers: failure }) }
  }
  if (RETRYABLE_REASONS.includes(settled.errorReason)) {
    return { paid: false, response: new Response(null, { status: 402, headers: failure }) }
  }
  return { paid: false, response: await challenge(failure) }
}

/**
 * Adds the x402 response headers to an `Access-Control-Expose-Headers` value,
 * so cross-origin browser code can read them.
 */
export function exposeX402Headers(existing) {
  const names = String(existing ?? '')
    .split(',')
    .map((name) => name.trim())
    .filter((name) => name !== '')
  for (const name of [PAYMENT_REQUIRED_HEADER, PAYMENT_RESPONSE_HEADER]) {
    if (!names.some((n) => n.toLowerCase() === name.toLowerCase())) {
      names.push(name)
    }
  }
  return names.join(', ')
}

/**
 * Hono middleware charging for every route it is mounted on. CORS preflight
 * requests pass through to the app's CORS handling.
 */
export function x402Hono(x402, options) {
  return async (c, next) => {
    if (c.req.method === 'OPTIONS') {
      return next()
    }
    const result = await requirePayment(c.req.raw, x402, options)
    // Hono copies the headers already on `c.res` over a returned response, so
    // the exposed headers are merged there rather than on the response.
    const expose = () => c.header(EXPOSE_HEADERS, exposeX402Headers(c.res.headers.get(EXPOSE_HEADERS)))
    if (!result.paid) {
      expose()
      return result.response
    }
    await next()
    c.header(PAYMENT_RESPONSE_HEADER, result.paymentResponse)
    expose()
  }
}

/**
 * Express middleware charging for every route it is mounted on. The payment
 * commits to the raw body bytes, so mount `express.raw({ type: '*\/*' })`
 * first on routes with a body. CORS preflight requests pass through to the
 * app's CORS handling.
 */
export function x402Express(x402, options) {
  return (req, res, next) => {
    if (req.method === 'OPTIONS') {
      next()
      return
    }
    if (req.body !== undefined && !(req.body instanceof Uint8Array) && !isEmpty(req.body)) {
      next(new Error('x402Express needs the raw body: mount express.raw() before it'))
      return
    }
    const headers = new Headers()
    for (const [name, value] of Object.entries(req.headers)) {
      for (const line of Array.isArray(value) ? value : [value]) {
        headers.append(name, line)
      }
    }
    const hasBody = req.body instanceof Uint8Array && req.body.length > 0
    const request = new Request(new URL(req.originalUrl, options.publicOrigin), {
      method: req.method,
      headers,
      body: hasBody ? req.body : undefined
    })
    requirePayment(request, x402, options)
      .then(async (result) => {
        res.setHeader(EXPOSE_HEADERS, exposeX402Headers(res.getHeader(EXPOSE_HEADERS)))
        if (result.paid) {
          res.setHeader(PAYMENT_RESPONSE_HEADER, result.paymentResponse)
          next()
          return
        }
        res.status(result.response.status)
        result.response.headers.forEach((value, name) => {
          // Already merged with any value the app set.
          if (name !== EXPOSE_HEADERS.toLowerCase()) {
            res.setHeader(name, value)
          }
        })
        res.end()
      })
      .catch(next)
  }
}

function isEmpty(value) {
  return typeof value === 'object' && value !== null && Object.keys(value).length === 0
}
