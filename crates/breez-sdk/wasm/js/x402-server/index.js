/* global Buffer */
const PAYMENT_REQUIRED_HEADER = 'PAYMENT-REQUIRED'
const PAYMENT_SIGNATURE_HEADER = 'PAYMENT-SIGNATURE'
const PAYMENT_RESPONSE_HEADER = 'PAYMENT-RESPONSE'
const EXPOSE_HEADERS = 'Access-Control-Expose-Headers'
const CACHE_CONTROL = 'Cache-Control'
const DEFAULT_MAX_BODY_BYTES = 1024 * 1024
const RAW_BODY_NEEDED =
  'x402Express needs the body bytes as sent, not parsed or inflated: mount it before any body ' +
  'parser, or keep them with express.json({ verify: (req, res, buf) => { req.rawBody = buf }, ' +
  'inflate: false })'

/**
 * Charges for a request with x402 over Lightning. Resolves to `{ paid: true,
 * paymentResponse }` when the request carries a valid payment, or to
 * `{ paid: false, response }` with the response to send instead. Mark a paid
 * response `private` in Cache-Control, so shared caches don't serve it to others.
 */
export async function requirePayment(request, x402, options) {
  const { publicOrigin, priceSats, boundHeaders = [], description, mimeType, maxTimeoutSecs } =
    options
  // The bound URL is the public one: its origin comes from configuration, never
  // from the Host or forwarding headers, which the client controls.
  const origin = originOf(publicOrigin)
  const maxBodyBytes = maxBodyBytesOf(options)
  // A preflight never carries a payment, so charging it would block the request it is for.
  if (request.method === 'OPTIONS' && request.headers.has('Access-Control-Request-Method')) {
    return { paid: false, response: new Response(null, { status: 204 }) }
  }
  if (request.bodyUsed) {
    throw new TypeError('x402 needs the request body unread: charge before anything reads it')
  }
  const unpaid = { [EXPOSE_HEADERS]: exposeX402Headers(), [CACHE_CONTROL]: 'no-store' }
  const tooLarge = () => ({
    paid: false,
    response: new Response(null, { status: 413, headers: unpaid })
  })
  if (Number(request.headers.get('Content-Length')) > maxBodyBytes) {
    return tooLarge()
  }
  const body = request.body === null ? undefined : await readBody(request.clone().body, maxBodyBytes)
  if (body?.length > maxBodyBytes) {
    return tooLarge()
  }
  const url = origin + requestTarget(request.url)
  const headers = []
  request.headers.forEach((value, name) => {
    if (name !== PAYMENT_SIGNATURE_HEADER.toLowerCase()) {
      headers.push({ name, value })
    }
  })
  const x402Request = { type: 'http', method: request.method, url, headers, body }
  const boundNames = [...new Set(boundHeaders.map((name) => name.toLowerCase()))].sort()
  const amountSats = typeof priceSats === 'function' ? await priceSats(request) : priceSats
  const amounts = [{ type: 'lightning', amountSats }]

  const challenge = async (extraHeaders) => {
    let created
    try {
      created = await x402.createChallenge({
        request: x402Request,
        boundNames,
        amounts,
        maxTimeoutSecs,
        description,
        mimeType
      })
    } catch (error) {
      // A request that can't be bound to a payment, such as a URL outside URI
      // syntax, can't be paid for.
      if (error?.x402?.reason === 'invalid_exact_lnbtc_request_binding') {
        return new Response(null, { status: 400, headers: extraHeaders })
      }
      throw error
    }
    return new Response(null, {
      status: 402,
      headers: { ...extraHeaders, [PAYMENT_REQUIRED_HEADER]: created.paymentRequired }
    })
  }

  const paymentSignature = request.headers.get(PAYMENT_SIGNATURE_HEADER)
  if (paymentSignature === null) {
    return { paid: false, response: await challenge(unpaid) }
  }
  const settled = await x402.settle({
    paymentSignature,
    request: x402Request,
    boundNames,
    amounts,
    maxTimeoutSecs
  })
  if (settled.status === 'settled') {
    return { paid: true, paymentResponse: settled.paymentResponse }
  }
  const failure = { ...unpaid, [PAYMENT_RESPONSE_HEADER]: settled.paymentResponse }
  if (settled.status === 'malformed') {
    return { paid: false, response: new Response(null, { status: 400, headers: failure }) }
  }
  if (settled.status === 'pending') {
    return { paid: false, response: new Response(null, { status: 402, headers: failure }) }
  }
  if (settled.status === 'error') {
    return { paid: false, response: new Response(null, { status: 500, headers: failure }) }
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
 * Hono middleware charging for every request that reaches it, whatever its
 * method. It answers a CORS preflight with 204 without charging, but without
 * CORS headers: mount CORS handling before it.
 */
export function x402Hono(x402, options) {
  originOf(options.publicOrigin)
  maxBodyBytesOf(options)
  return async (c, next) => {
    const result = await requirePayment(c.req.raw, x402, options)
    // Hono copies the headers already on `c.res` over a returned response, so
    // the headers an app may have set are merged there rather than on the
    // response. Before Hono 4.8, `c.header` doesn't reach `c.res` until a
    // handler responds.
    const merge = (name, value) => c.res.headers.set(name, value(c.res.headers.get(name)))
    if (!result.paid) {
      merge(EXPOSE_HEADERS, exposeX402Headers)
      const cacheControl = result.response.headers.get(CACHE_CONTROL)
      if (cacheControl !== null) {
        c.res.headers.set(CACHE_CONTROL, cacheControl)
      }
      return result.response
    }
    await next()
    c.header(PAYMENT_RESPONSE_HEADER, result.paymentResponse)
    merge(EXPOSE_HEADERS, exposeX402Headers)
    merge(CACHE_CONTROL, privateCacheControl)
  }
}

/**
 * Express middleware charging for every request that reaches it, whatever its
 * method. The payment commits to the body bytes as sent, which it reads itself
 * when nothing has read the body yet. Behind a body parser, keep the bytes in
 * `req.rawBody` with the parser's `verify` option and `inflate: false`. It
 * answers a CORS preflight with 204 without charging, but without CORS
 * headers: mount CORS handling before it.
 */
export function x402Express(x402, options) {
  const origin = originOf(options.publicOrigin)
  const maxBodyBytes = maxBodyBytesOf(options)
  return (req, res, next) => {
    expressRequest(req, origin, maxBodyBytes)
      .then(async (request) => {
        if (typeof request === 'number') {
          res.status(request)
          res.setHeader(CACHE_CONTROL, 'no-store')
          if (request === 413) {
            // The body may be left unread, so the connection can't carry another request.
            res.setHeader('Connection', 'close')
          }
          res.end()
          return
        }
        const result = await requirePayment(request, x402, options)
        res.setHeader(EXPOSE_HEADERS, exposeX402Headers(res.getHeader(EXPOSE_HEADERS)))
        if (result.paid) {
          res.setHeader(PAYMENT_RESPONSE_HEADER, result.paymentResponse)
          writePrivate(res)
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

/** The origin `publicOrigin` names. A path there would be dropped from the bound URL. */
function originOf(publicOrigin) {
  const url = new URL(publicOrigin)
  if (url.href !== `${url.origin}/`) {
    throw new TypeError(
      `publicOrigin must be an origin such as https://api.example.com, without a path: ${publicOrigin}`
    )
  }
  return url.origin
}

function maxBodyBytesOf({ maxBodyBytes = DEFAULT_MAX_BODY_BYTES }) {
  if (!(Number.isSafeInteger(maxBodyBytes) && maxBodyBytes >= 0)) {
    throw new TypeError('maxBodyBytes must be a whole number of at least 0')
  }
  return maxBodyBytes
}

/** Reads `stream` until it ends or holds more than `maxBytes`. */
async function readBody(stream, maxBytes) {
  const reader = stream.getReader()
  const chunks = []
  let length = 0
  while (length <= maxBytes) {
    const { done, value } = await reader.read()
    if (done) {
      break
    }
    chunks.push(value)
    length += value.length
  }
  const body = new Uint8Array(length)
  let offset = 0
  for (const chunk of chunks) {
    body.set(chunk, offset)
    offset += chunk.length
  }
  return body
}

/** The path and query of `url`, keeping a bare `?` that `search` drops. */
function requestTarget(url) {
  const parsed = new URL(url)
  parsed.hash = ''
  return parsed.href.slice(parsed.origin.length)
}

/**
 * The Fetch API view of an Express request, or the status to refuse it with:
 * 413 for a body over `maxBodyBytes`, or 400 when it has no such view. Express
 * routes on the target as sent, so a target that URL parsing would rewrite
 * (dot segments, an absolute form) could be routed as one resource and paid
 * for as another: it has none either.
 */
async function expressRequest(req, origin, maxBodyBytes) {
  const target = req.originalUrl
  if (!target.startsWith('/') || (sentBody(req) && ['GET', 'HEAD'].includes(req.method))) {
    return 400
  }
  let url
  try {
    // Appended, not resolved: a path starting `//` would otherwise replace the host.
    url = new URL(origin + target)
  } catch {
    return 400
  }
  if (requestTarget(url) !== target) {
    return 400
  }
  if (Number(req.headers['content-length']) > maxBodyBytes) {
    return 413
  }
  const body = await rawBody(req, maxBodyBytes)
  if (body?.length > maxBodyBytes) {
    return 413
  }
  const headers = new Headers()
  for (const [name, value] of Object.entries(req.headers)) {
    for (const line of Array.isArray(value) ? value : [value]) {
      headers.append(name, line)
    }
  }
  try {
    return new Request(url, { method: req.method, headers, body: body?.length > 0 ? body : undefined })
  } catch {
    // A method the Fetch API refuses, such as TRACE.
    return 400
  }
}

function sentBody(req) {
  const length = req.headers['content-length']
  return req.headers['transfer-encoding'] !== undefined || (length !== undefined && length !== '0')
}

/**
 * The body bytes as sent: as a body parser kept them, or read from the stream
 * when nothing has read it, stopping once there are more than `maxBytes`. A
 * parsed or inflated body is no longer those bytes.
 */
async function rawBody(req, maxBytes) {
  if (!sentBody(req)) {
    return undefined
  }
  const kept = [req.rawBody, req.body].find((body) => body instanceof Uint8Array)
  if (kept !== undefined) {
    const encoding = req.headers['content-encoding']
    if (encoding !== undefined && encoding.toLowerCase() !== 'identity') {
      throw new Error(RAW_BODY_NEEDED)
    }
    return kept
  }
  if (!req.readable || req.readableDidRead) {
    throw new Error(RAW_BODY_NEEDED)
  }
  const chunks = []
  let length = 0
  for await (const chunk of req) {
    chunks.push(chunk)
    length += chunk.length
    if (length > maxBytes) {
      break
    }
  }
  req.body = Buffer.concat(chunks)
  // body-parser 1.x skips a request marked read, and 2.x one whose stream has ended.
  req._body = true
  return req.body
}

/**
 * Merges `private` into the response's Cache-Control as its headers are sent,
 * covering what the handler sets. Headers passed to `writeHead` override those
 * set before, so they are applied first.
 */
function writePrivate(res) {
  const writeHead = res.writeHead
  res.writeHead = function (statusCode, reason, headers) {
    if (typeof reason !== 'string') {
      headers = reason
      reason = undefined
    }
    if (Array.isArray(headers)) {
      for (let i = 0; i < headers.length; i += 2) {
        res.removeHeader(headers[i])
      }
      for (let i = 0; i < headers.length; i += 2) {
        res.appendHeader(headers[i], headers[i + 1])
      }
    } else if (headers) {
      for (const [name, value] of Object.entries(headers)) {
        res.setHeader(name, value)
      }
    }
    res.setHeader(CACHE_CONTROL, privateCacheControl(res.getHeader(CACHE_CONTROL)))
    return writeHead.call(this, statusCode, reason)
  }
}

/** `value` with `private` in place of `public`, keeping a paid response out of shared caches. */
function privateCacheControl(value) {
  const directives = String(value ?? '')
    .split(',')
    .map((directive) => directive.trim())
    .filter((directive) => directive !== '' && directive.toLowerCase() !== 'public')
  if (!directives.some((directive) => directive.toLowerCase() === 'private')) {
    directives.push('private')
  }
  return directives.join(', ')
}
