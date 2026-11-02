import assert from 'node:assert/strict'
import http from 'node:http'
import test from 'node:test'
import { gzipSync } from 'node:zlib'

import express from 'express'
import express4 from 'express-4'
import { Hono } from 'hono'
import { cors } from 'hono/cors'

import { exposeX402Headers, requirePayment, x402Express, x402Hono } from './index.js'

const OPTIONS = {
  publicOrigin: 'https://api.example.com',
  priceSats: 21,
  boundHeaders: ['Accept']
}

function fakeX402(settleResults = [], challengeError = undefined) {
  const calls = { createChallenge: [], settle: [] }
  const x402 = {
    createChallenge: async (request) => {
      calls.createChallenge.push(request)
      if (challengeError !== undefined) {
        throw challengeError
      }
      return { paymentRequired: 'challenge', paymentHash: 'hash' }
    },
    settle: async (request) => {
      calls.settle.push(request)
      return settleResults.shift()
    }
  }
  return { x402, calls }
}

function paidRequest(url, init = {}) {
  return new Request(url, { ...init, headers: { ...init.headers, 'PAYMENT-SIGNATURE': 'proof' } })
}

test('answers an unpaid request with a challenge for its public URL', async () => {
  const { x402, calls } = fakeX402()
  // The Host the client sent is not trusted for the bound URL.
  const request = new Request('http://evil.example/article/A?x=1', {
    headers: { accept: 'application/json' }
  })
  const result = await requirePayment(request, x402, OPTIONS)
  assert.equal(result.paid, false)
  assert.equal(result.response.status, 402)
  assert.equal(result.response.headers.get('PAYMENT-REQUIRED'), 'challenge')
  assert.equal(result.response.headers.get('Cache-Control'), 'no-store')
  const created = calls.createChallenge[0]
  assert.equal(created.request.url, 'https://api.example.com/article/A?x=1')
  assert.deepEqual(created.boundNames, ['accept'])
  assert.deepEqual(created.amounts, [{ type: 'lightning', amountSats: 21 }])
})

test('settles a paid request with the same request, bound names and price', async () => {
  const { x402, calls } = fakeX402([{ status: 'settled', paymentResponse: 'settled' }])
  const request = paidRequest('https://api.example.com/article/A', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: '{"a":1}'
  })
  const result = await requirePayment(request, x402, OPTIONS)
  assert.deepEqual(result, { paid: true, paymentResponse: 'settled' })
  const settled = calls.settle[0]
  assert.equal(settled.paymentSignature, 'proof')
  assert.deepEqual(settled.boundNames, ['accept'])
  assert.deepEqual(settled.amounts, [{ type: 'lightning', amountSats: 21 }])
  assert.deepEqual(settled.request.body, new TextEncoder().encode('{"a":1}'))
  assert.ok(settled.request.headers.every((h) => h.name !== 'payment-signature'))
})

test('settles and challenges at the price and timeout it quotes now', async () => {
  const { x402, calls } = fakeX402([
    { status: 'refused', errorReason: 'invalid_exact_lnbtc_amount_mismatch', paymentResponse: 'failed' }
  ])
  const priced = []
  const options = {
    ...OPTIONS,
    maxTimeoutSecs: 60,
    priceSats: async (request) => {
      priced.push(request)
      return 42
    }
  }
  const request = paidRequest('https://api.example.com/a')
  await requirePayment(request, x402, options)
  const quote = { amounts: [{ type: 'lightning', amountSats: 42 }], maxTimeoutSecs: 60 }
  for (const { amounts, maxTimeoutSecs } of [calls.settle[0], calls.createChallenge[0]]) {
    assert.deepEqual({ amounts, maxTimeoutSecs }, quote)
  }
  assert.deepEqual(priced, [request])
})

for (const [status, errorReason, httpStatus, challenged] of [
  ['refused', 'duplicate_settlement', 402, true],
  ['pending', 'settlement_pending', 402, false],
  ['error', 'unexpected_settle_error', 500, false],
  ['malformed', 'invalid_exact_lnbtc_preimage_malformed', 400, false]
]) {
  test(`answers a ${status} payment (${errorReason}) with ${httpStatus}`, async () => {
    const { x402, calls } = fakeX402([{ status, errorReason, paymentResponse: 'failed' }])
    const result = await requirePayment(paidRequest('https://api.example.com/a'), x402, OPTIONS)
    assert.equal(result.response.status, httpStatus)
    assert.equal(result.response.headers.get('PAYMENT-RESPONSE'), 'failed')
    assert.equal(result.response.headers.get('PAYMENT-REQUIRED'), challenged ? 'challenge' : null)
    assert.equal(result.response.headers.get('Cache-Control'), 'no-store')
    assert.equal(calls.createChallenge.length, challenged ? 1 : 0)
  })
}

test('answers as the settle status says, whatever the reason', async () => {
  const { x402, calls } = fakeX402([
    { status: 'pending', paymentResponse: 'failed' },
    { status: 'malformed', paymentResponse: 'failed' }
  ])
  const pending = await requirePayment(paidRequest('https://api.example.com/a'), x402, OPTIONS)
  assert.equal(pending.response.status, 402)
  const malformed = await requirePayment(paidRequest('https://api.example.com/a'), x402, OPTIONS)
  assert.equal(malformed.response.status, 400)
  assert.equal(calls.createChallenge.length, 0)
})

test('binds each header once, whatever its case', async () => {
  const { x402, calls } = fakeX402()
  const options = { ...OPTIONS, boundHeaders: ['accept', 'Authorization', 'Accept'] }
  await requirePayment(new Request('https://api.example.com/a'), x402, options)
  assert.deepEqual(calls.createChallenge[0].boundNames, ['accept', 'authorization'])
})

test('exposes the x402 headers to browsers', async () => {
  const { x402 } = fakeX402()
  const result = await requirePayment(new Request('https://api.example.com/a'), x402, OPTIONS)
  assert.equal(
    result.response.headers.get('Access-Control-Expose-Headers'),
    'PAYMENT-REQUIRED, PAYMENT-RESPONSE'
  )
})

test('adds each x402 header to the exposed headers once', () => {
  assert.equal(
    exposeX402Headers('X-Request-Id, payment-response'),
    'X-Request-Id, payment-response, PAYMENT-REQUIRED'
  )
  assert.equal(exposeX402Headers(undefined), 'PAYMENT-REQUIRED, PAYMENT-RESPONSE')
})

for (const [what, url, bound] of [
  ['a path starting with two slashes', 'http://evil.example//other.example/a', '//other.example/a'],
  ['a bare query mark', 'http://evil.example/a?', '/a?'],
  ['a fragment', 'http://evil.example/a?b=1#c', '/a?b=1']
]) {
  test(`binds a URL with ${what} under the public origin`, async () => {
    const { x402, calls } = fakeX402()
    await requirePayment(new Request(url), x402, OPTIONS)
    assert.equal(calls.createChallenge[0].request.url, `https://api.example.com${bound}`)
  })
}

test('refuses a public origin with a path', async () => {
  const { x402 } = fakeX402()
  const options = { ...OPTIONS, publicOrigin: 'https://api.example.com/api' }
  const request = new Request('https://api.example.com/api/a')
  await assert.rejects(requirePayment(request, x402, options), TypeError)
  assert.throws(() => x402Hono(x402, options), TypeError)
  assert.throws(() => x402Express(x402, options), TypeError)
})

test('answers a URL that cannot be bound with 400', async () => {
  const { x402 } = fakeX402(
    [],
    Object.assign(new Error('x402 refused'), {
      x402: {
        kind: 'refused',
        reason: 'invalid_exact_lnbtc_request_binding',
        details: 'URI "https://api.example.com/a|b" has a character outside URI syntax'
      }
    })
  )
  const result = await requirePayment(new Request('https://api.example.com/a|b'), x402, OPTIONS)
  assert.equal(result.response.status, 400)
  assert.equal(result.response.headers.get('PAYMENT-REQUIRED'), null)
  assert.equal(result.response.headers.get('Cache-Control'), 'no-store')
})

test('answers a CORS preflight with 204, without charging', async () => {
  const { x402, calls } = fakeX402()
  const request = new Request('https://api.example.com/a', {
    method: 'OPTIONS',
    headers: { origin: 'https://app.example', 'access-control-request-method': 'POST' }
  })
  const result = await requirePayment(request, x402, OPTIONS)
  assert.equal(result.paid, false)
  assert.equal(result.response.status, 204)
  assert.equal(calls.createChallenge.length, 0)
})

test('refuses a request whose body was already read', async () => {
  const { x402 } = fakeX402()
  const request = new Request('https://api.example.com/a', { method: 'POST', body: 'x' })
  await request.text()
  await assert.rejects(requirePayment(request, x402, OPTIONS), /body unread/)
})

test('answers a Content-Length over maxBodyBytes with 413, unread', { timeout: 5000 }, async () => {
  const { x402, calls } = fakeX402()
  // Only part of the body arrives, so reading it first would wait forever.
  const body = new ReadableStream({
    start(controller) {
      controller.enqueue(new Uint8Array([1, 2]))
    }
  })
  const request = new Request('https://api.example.com/a', {
    method: 'POST',
    body,
    duplex: 'half',
    headers: { 'content-length': '5' }
  })
  const result = await requirePayment(request, x402, { ...OPTIONS, maxBodyBytes: 4 })
  assert.equal(result.response.status, 413)
  assert.equal(calls.createChallenge.length, 0)
})

for (const [length, status] of [
  ['1048577', 413],
  ['1048576', 402]
]) {
  test(`answers a body whose Content-Length is ${length} with ${status}`, async () => {
    const { x402, calls } = fakeX402()
    const request = new Request('https://api.example.com/a', {
      method: 'POST',
      body: 'x',
      headers: { 'content-length': length }
    })
    const result = await requirePayment(request, x402, OPTIONS)
    assert.equal(result.response.status, status)
    assert.equal(result.response.headers.get('Cache-Control'), 'no-store')
    assert.equal(calls.createChallenge.length, status === 413 ? 0 : 1)
  })
}

for (const [what, chunks, ends, status] of [
  ['exactly maxBodyBytes', [[1, 2], [3, 4]], true, 402],
  ['one byte more than maxBodyBytes', [[1, 2, 3, 4], [5]], true, 413],
  ['more than maxBodyBytes that never ends', [[1, 2, 3, 4, 5]], false, 413]
]) {
  test(`answers a streamed body of ${what} with ${status}`, { timeout: 5000 }, async () => {
    const { x402, calls } = fakeX402()
    const body = new ReadableStream({
      start(controller) {
        for (const chunk of chunks) {
          controller.enqueue(new Uint8Array(chunk))
        }
        if (ends) {
          controller.close()
        }
      }
    })
    const request = new Request('https://api.example.com/a', { method: 'POST', body, duplex: 'half' })
    const result = await requirePayment(request, x402, { ...OPTIONS, maxBodyBytes: 4 })
    assert.equal(result.response.status, status)
    assert.equal(result.response.headers.get('Cache-Control'), 'no-store')
    assert.deepEqual(
      calls.createChallenge.map((created) => created.request.body),
      status === 413 ? [] : [new Uint8Array([1, 2, 3, 4])]
    )
  })
}

test('refuses an invalid maxBodyBytes', async () => {
  const { x402 } = fakeX402()
  for (const maxBodyBytes of [-1, 1.5, '1024']) {
    const options = { ...OPTIONS, maxBodyBytes }
    await assert.rejects(requirePayment(new Request('https://api.example.com/a'), x402, options), TypeError)
    assert.throws(() => x402Hono(x402, options), TypeError)
    assert.throws(() => x402Express(x402, options), TypeError)
  }
})

const preflight = {
  method: 'OPTIONS',
  headers: { origin: 'https://app.example', 'access-control-request-method': 'POST' }
}

function honoApp(x402, { before, withCors = true, handler } = {}) {
  const app = new Hono()
  if (withCors) {
    app.use(cors({ origin: '*', exposeHeaders: ['X-Request-Id'] }))
  }
  if (before !== undefined) {
    app.use(before)
  }
  app.use('/paid/*', x402Hono(x402, OPTIONS))
  app.all('/paid/*', handler ?? (async (c) => c.text(`content ${await c.req.text()}`)))
  app.onError((error, c) => c.text(error.message, 500))
  return app
}

test('Hono middleware answers an unpaid request with an exposed challenge', async () => {
  const { x402 } = fakeX402()
  const response = await honoApp(x402).request('https://api.example.com/paid/a')
  assert.equal(response.status, 402)
  assert.equal(response.headers.get('PAYMENT-REQUIRED'), 'challenge')
  assert.equal(
    response.headers.get('Access-Control-Expose-Headers'),
    'X-Request-Id, PAYMENT-REQUIRED, PAYMENT-RESPONSE'
  )
})

test('Hono middleware keeps a challenge out of caches the app allowed', async () => {
  const { x402 } = fakeX402()
  const cacheable = async (c, next) => {
    c.header('Cache-Control', 'public, max-age=60')
    await next()
  }
  const response = await honoApp(x402, { before: cacheable }).request('https://api.example.com/paid/a')
  assert.equal(response.status, 402)
  assert.equal(response.headers.get('Cache-Control'), 'no-store')
})

test('Hono middleware binds a bare query mark', async () => {
  const { x402, calls } = fakeX402()
  await honoApp(x402).request('https://api.example.com/paid/a?')
  assert.equal(calls.createChallenge[0].request.url, 'https://api.example.com/paid/a?')
})

test('Hono middleware serves a paid request with the payment response', async () => {
  const { x402, calls } = fakeX402([{ status: 'settled', paymentResponse: 'settled' }])
  const response = await honoApp(x402).request('https://api.example.com/paid/a', {
    method: 'POST',
    headers: { 'PAYMENT-SIGNATURE': 'proof' },
    body: 'body'
  })
  assert.equal(await response.text(), 'content body')
  assert.equal(response.headers.get('PAYMENT-RESPONSE'), 'settled')
  assert.equal(
    response.headers.get('Access-Control-Expose-Headers'),
    'X-Request-Id, PAYMENT-REQUIRED, PAYMENT-RESPONSE'
  )
  assert.equal(response.headers.get('Cache-Control'), 'private')
  assert.deepEqual(calls.settle[0].request.body, new TextEncoder().encode('body'))
})

test('Hono middleware marks a paid response private, in place of public', async () => {
  const { x402 } = fakeX402([{ status: 'settled', paymentResponse: 'settled' }])
  const handler = (c) => {
    c.header('Cache-Control', 'public, max-age=60')
    return c.text('content')
  }
  const response = await honoApp(x402, { handler }).request('https://api.example.com/paid/a', {
    headers: { 'PAYMENT-SIGNATURE': 'proof' }
  })
  assert.equal(response.headers.get('Cache-Control'), 'max-age=60, private')
})

test('Hono middleware leaves preflight requests to CORS handling mounted before it', async () => {
  const { x402, calls } = fakeX402()
  const response = await honoApp(x402).request('https://api.example.com/paid/a', preflight)
  assert.equal(response.status, 204)
  assert.equal(response.headers.get('Access-Control-Allow-Origin'), '*')
  assert.equal(calls.createChallenge.length, 0)
})

test('Hono middleware answers a preflight without charging or reaching the handler', async () => {
  const { x402, calls } = fakeX402()
  let handled = false
  const handler = (c) => {
    handled = true
    return c.text('content')
  }
  const response = await honoApp(x402, { withCors: false, handler }).request(
    'https://api.example.com/paid/a',
    preflight
  )
  assert.equal(response.status, 204)
  assert.equal(handled, false)
  assert.equal(calls.createChallenge.length, 0)
})

test('Hono middleware charges OPTIONS like any method', async () => {
  const { x402, calls } = fakeX402()
  const response = await honoApp(x402, { withCors: false }).request(
    'https://api.example.com/paid/a',
    { method: 'OPTIONS' }
  )
  assert.equal(response.status, 402)
  assert.equal(calls.createChallenge.length, 1)
})

test('Hono middleware answers a body over maxBodyBytes with 413', async () => {
  const { x402, calls } = fakeX402()
  const response = await honoApp(x402).request('https://api.example.com/paid/a', {
    method: 'POST',
    body: 'x',
    headers: { 'content-length': '1048577' }
  })
  assert.equal(response.status, 413)
  assert.equal(response.headers.get('Cache-Control'), 'no-store')
  assert.equal(calls.createChallenge.length, 0)
})

test('Hono middleware explains a body read before it', async () => {
  const { x402 } = fakeX402()
  const readBody = async (c, next) => {
    await c.req.raw.text()
    await next()
  }
  const response = await honoApp(x402, { before: readBody }).request(
    'https://api.example.com/paid/a',
    { method: 'POST', body: 'body' }
  )
  assert.equal(response.status, 500)
  assert.match(await response.text(), /body unread/)
})

/** Echoes the body the handler sees: its bytes, or what a parser made of it. */
function echoBody(req, res) {
  const body = req.body instanceof Uint8Array ? { bytes: Buffer.from(req.body).toString() } : req.body
  res.json({ body: body ?? null })
}

/**
 * Runs `run` against an Express app charging with `x402` and `options`, with
 * `before` and `after` mounted around the middleware and `handler` serving
 * every route.
 */
async function withExpressServer(
  framework,
  x402,
  { before = [], after = [], handler = echoBody, options = OPTIONS },
  run
) {
  const app = framework()
  app.use((req, res, next) => {
    res.setHeader('Access-Control-Expose-Headers', 'X-Request-Id')
    next()
  })
  for (const middleware of before) {
    app.use(middleware)
  }
  app.use(x402Express(x402, options))
  for (const middleware of after) {
    app.use(middleware)
  }
  app.all(/.*/, handler)
  // Express recognizes an error handler by its four parameters.
  // eslint-disable-next-line no-unused-vars
  app.use((error, req, res, next) => res.status(500).send(error.message))
  const server = await new Promise((resolve) => {
    const listening = app.listen(0, '127.0.0.1', () => resolve(listening))
  })
  try {
    return await run(`http://127.0.0.1:${server.address().port}`)
  } finally {
    server.closeAllConnections()
    await new Promise((resolve) => server.close(resolve))
  }
}

/** Sends a request whose target fetch would normalize, resolving to its status and headers. */
function rawRequest(origin, { method = 'GET', path, headers = {}, body }) {
  const { hostname, port } = new URL(origin)
  return new Promise((resolve, reject) => {
    const request = http.request({ hostname, port, method, path, headers }, (response) => {
      response.resume()
      response.on('end', () => resolve({ status: response.statusCode, headers: response.headers }))
    })
    request.on('error', reject)
    request.end(body)
  })
}

/**
 * Starts a body with `chunk` and never ends it, resolving to the response's
 * status and headers. Rejects if no response comes within a second.
 */
function unfinishedRequest(
  origin,
  path,
  chunk,
  headers = { 'transfer-encoding': 'chunked' },
  method = 'POST'
) {
  const { hostname, port } = new URL(origin)
  return new Promise((resolve, reject) => {
    const request = http.request({ hostname, port, method, path, headers }, (response) => {
      resolve({ status: response.statusCode, headers: response.headers })
      request.destroy()
    })
    request.on('error', reject)
    request.setTimeout(1000, () => request.destroy(new Error('no response before the body ended')))
    request.write(chunk)
  })
}

const answerPreflights = (req, res, next) => {
  if (req.method === 'OPTIONS') {
    res.status(204).end()
    return
  }
  next()
}

const keepRawBody = (req, res, buf) => {
  req.rawBody = buf
}

for (const [name, framework] of [
  ['Express 5', express],
  ['Express 4', express4]
]) {
  const withServer = (x402, setup, run) => withExpressServer(framework, x402, setup, run)
  const raw = framework.raw({ type: () => true, inflate: false })
  const rawMessage = /mount it before any body parser.*req\.rawBody = buf.*inflate: false/
  const json = (body) => ({ body, headers: { 'content-type': 'application/json' } })

  test(`${name} middleware answers an unpaid request with an exposed challenge`, async () => {
    const { x402, calls } = fakeX402()
    await withServer(x402, {}, async (origin) => {
      const response = await fetch(`${origin}/a?b=1`, { headers: { accept: 'application/json' } })
      assert.equal(response.status, 402)
      assert.equal(response.headers.get('PAYMENT-REQUIRED'), 'challenge')
      assert.equal(
        response.headers.get('Access-Control-Expose-Headers'),
        'X-Request-Id, PAYMENT-REQUIRED, PAYMENT-RESPONSE'
      )
    })
    assert.equal(calls.createChallenge[0].request.url, 'https://api.example.com/a?b=1')
  })

  test(`${name} middleware keeps a challenge out of caches the app allowed`, async () => {
    const { x402 } = fakeX402()
    const cacheable = (req, res, next) => {
      res.setHeader('Cache-Control', 'public, max-age=60')
      next()
    }
    await withServer(x402, { before: [cacheable] }, async (origin) => {
      const response = await fetch(`${origin}/a`)
      assert.equal(response.status, 402)
      assert.equal(response.headers.get('Cache-Control'), 'no-store')
    })
  })

  for (const [what, path, bound] of [
    ['a path starting with two slashes', '//other.example/a', '//other.example/a'],
    ['a bare query mark', '/a?', '/a?']
  ]) {
    test(`${name} middleware binds ${what} under the public origin`, async () => {
      const { x402, calls } = fakeX402()
      await withServer(x402, {}, async (origin) => {
        assert.equal((await rawRequest(origin, { path })).status, 402)
      })
      assert.equal(calls.createChallenge[0].request.url, `https://api.example.com${bound}`)
    })
  }

  for (const [what, path] of [
    ['dot segments', '/premium/../free'],
    ['encoded dot segments', '/premium/%2e%2e/free'],
    ['an absolute form', 'http://cheap.example/premium/report'],
    ['a fragment', '/premium#free']
  ]) {
    test(`${name} middleware refuses a target with ${what} with 400`, async () => {
      const { x402, calls } = fakeX402()
      await withServer(x402, {}, async (origin) => {
        const response = await rawRequest(origin, { path })
        assert.equal(response.status, 400)
        assert.equal(response.headers['cache-control'], 'no-store')
      })
      assert.equal(calls.createChallenge.length, 0)
    })
  }

  for (const [what, request] of [
    ['a method the Fetch API refuses', { method: 'TRACE', path: '/a' }],
    ['a GET with a body', { path: '/a', headers: { 'content-length': '1' }, body: 'x' }]
  ]) {
    test(`${name} middleware answers ${what} with 400`, async () => {
      const { x402, calls } = fakeX402()
      await withServer(x402, { before: [raw] }, async (origin) => {
        assert.equal((await rawRequest(origin, request)).status, 400)
      })
      assert.equal(calls.createChallenge.length, 0)
    })
  }

  test(`${name} middleware answers a GET with a body before reading it`, async () => {
    const { x402, calls } = fakeX402()
    await withServer(x402, {}, async (origin) => {
      const response = await unfinishedRequest(origin, '/a', 'x', undefined, 'GET')
      assert.equal(response.status, 400)
    })
    assert.equal(calls.createChallenge.length, 0)
  })

  test(`${name} middleware settles a paid request with its raw body`, async () => {
    const { x402, calls } = fakeX402([{ status: 'settled', paymentResponse: 'settled' }])
    await withServer(x402, { before: [raw] }, async (origin) => {
      const response = await fetch(`${origin}/a`, {
        method: 'POST',
        headers: { 'PAYMENT-SIGNATURE': 'proof', 'content-type': 'application/json' },
        body: '{"a":1}'
      })
      assert.deepEqual(await response.json(), { body: { bytes: '{"a":1}' } })
      assert.equal(response.headers.get('PAYMENT-RESPONSE'), 'settled')
      assert.equal(response.headers.get('Cache-Control'), 'private')
    })
    assert.deepEqual(calls.settle[0].request.body, new TextEncoder().encode('{"a":1}'))
  })

  for (const [what, handler, cacheControl] of [
    [
      'set on the response',
      (req, res) => res.set('Cache-Control', 'public, max-age=60').send('content'),
      'max-age=60, private'
    ],
    [
      'passed to writeHead',
      (req, res) => res.writeHead(200, { 'Cache-Control': 'public' }).end('content'),
      'private'
    ],
    [
      'passed to writeHead as a list',
      (req, res) => res.writeHead(200, 'OK', ['Cache-Control', 'max-age=5']).end('content'),
      'max-age=5, private'
    ]
  ]) {
    test(`${name} middleware marks a paid response private, with caching ${what}`, async () => {
      const { x402 } = fakeX402([{ status: 'settled', paymentResponse: 'settled' }])
      await withServer(x402, { handler }, async (origin) => {
        const response = await fetch(`${origin}/a`, { headers: { 'PAYMENT-SIGNATURE': 'proof' } })
        assert.equal(await response.text(), 'content')
        assert.equal(response.headers.get('Cache-Control'), cacheControl)
      })
    })
  }

  for (const [what, before, init] of [
    ['no content type', [raw], { body: new Uint8Array([1, 2, 3]) }],
    ['an identity encoding', [raw], { body: 'x', headers: { 'content-encoding': 'identity' } }],
    ['no body parser', [], { body: 'x' }],
    [
      'a body of a type the parser skips',
      [framework.raw()],
      { body: 'x', headers: { 'content-type': 'text/plain' } }
    ],
    [
      'a compressed body no parser read',
      [],
      { body: gzipSync('x'), headers: { 'content-encoding': 'gzip' } }
    ],
    [
      'a body kept in req.rawBody',
      [framework.json({ verify: keepRawBody, inflate: false })],
      json('{"a":1}')
    ]
  ]) {
    test(`${name} middleware binds a body with ${what}`, async () => {
      const { x402, calls } = fakeX402()
      await withServer(x402, { before }, async (origin) => {
        const response = await fetch(`${origin}/a`, { method: 'POST', ...init })
        assert.equal(response.status, 402)
      })
      const sent = typeof init.body === 'string' ? new TextEncoder().encode(init.body) : init.body
      assert.deepEqual(calls.createChallenge[0].request.body, new Uint8Array(sent))
    })
  }

  test(`${name} middleware hands the body it read to the handler, past later parsers`, async () => {
    const { x402, calls } = fakeX402([{ status: 'settled', paymentResponse: 'settled' }])
    await withServer(x402, { after: [framework.json()] }, async (origin) => {
      const response = await fetch(`${origin}/a`, {
        method: 'POST',
        body: '{"a":1}',
        headers: { 'PAYMENT-SIGNATURE': 'proof', 'content-type': 'application/json' }
      })
      assert.deepEqual(await response.json(), { body: { bytes: '{"a":1}' } })
    })
    assert.deepEqual(calls.settle[0].request.body, new TextEncoder().encode('{"a":1}'))
  })

  test(`${name} middleware leaves the parsed body of one kept in req.rawBody`, async () => {
    const { x402 } = fakeX402([{ status: 'settled', paymentResponse: 'settled' }])
    const before = [framework.json({ verify: keepRawBody, inflate: false })]
    await withServer(x402, { before }, async (origin) => {
      const response = await fetch(`${origin}/a`, {
        method: 'POST',
        body: '{"a":1}',
        headers: { 'PAYMENT-SIGNATURE': 'proof', 'content-type': 'application/json' }
      })
      assert.deepEqual(await response.json(), { body: { a: 1 } })
    })
  })

  for (const [what, parser, init] of [
    ['a parsed body', framework.json(), json('{"a":1}')],
    [
      'an inflated body',
      framework.raw({ type: () => true }),
      { body: gzipSync('x'), headers: { 'content-encoding': 'gzip' } }
    ],
    [
      'an inflated body kept in req.rawBody',
      framework.json({ verify: keepRawBody }),
      {
        body: gzipSync('{"a":1}'),
        headers: { 'content-type': 'application/json', 'content-encoding': 'gzip' }
      }
    ]
  ]) {
    test(`${name} middleware refuses ${what}`, async () => {
      const { x402, calls } = fakeX402()
      await withServer(x402, { before: [parser] }, async (origin) => {
        const response = await fetch(`${origin}/a`, { method: 'POST', ...init })
        assert.equal(response.status, 500)
        assert.match(await response.text(), rawMessage)
      })
      assert.equal(calls.createChallenge.length, 0)
    })
  }

  const limited = { ...OPTIONS, maxBodyBytes: 4 }

  test(`${name} middleware answers a Content-Length over maxBodyBytes with 413, unread`, async () => {
    const { x402, calls } = fakeX402()
    await withServer(x402, { options: limited }, async (origin) => {
      // Only part of the body is sent, so reading it first would wait forever.
      const response = await unfinishedRequest(origin, '/a', '12', { 'content-length': '5' })
      assert.equal(response.status, 413)
      assert.equal(response.headers['cache-control'], 'no-store')
    })
    assert.equal(calls.createChallenge.length, 0)
  })

  test(`${name} middleware stops reading a body over maxBodyBytes and answers 413`, async () => {
    const { x402, calls } = fakeX402()
    await withServer(x402, { options: limited }, async (origin) => {
      const response = await unfinishedRequest(origin, '/a', '12345')
      assert.equal(response.status, 413)
      assert.equal(response.headers['cache-control'], 'no-store')
      assert.equal(response.headers.connection, 'close')
    })
    assert.equal(calls.createChallenge.length, 0)
  })

  test(`${name} middleware answers a body a parser kept over maxBodyBytes with 413`, async () => {
    const { x402, calls } = fakeX402()
    await withServer(x402, { before: [raw], options: limited }, async (origin) => {
      const request = {
        method: 'POST',
        path: '/a',
        headers: { 'transfer-encoding': 'chunked' },
        body: '12345'
      }
      assert.equal((await rawRequest(origin, request)).status, 413)
    })
    assert.equal(calls.createChallenge.length, 0)
  })

  test(`${name} middleware charges OPTIONS like any method`, async () => {
    const { x402, calls } = fakeX402()
    await withServer(x402, {}, async (origin) => {
      const response = await fetch(`${origin}/a`, { method: 'OPTIONS' })
      assert.equal(response.status, 402)
    })
    assert.equal(calls.createChallenge.length, 1)
  })

  test(`${name} middleware answers a preflight without charging or reaching the handler`, async () => {
    const { x402, calls } = fakeX402()
    let handled = false
    const handler = (req, res) => {
      handled = true
      res.send('content')
    }
    await withServer(x402, { handler }, async (origin) => {
      assert.equal((await rawRequest(origin, { path: '/a', ...preflight })).status, 204)
    })
    assert.equal(handled, false)
    assert.equal(calls.createChallenge.length, 0)
  })

  test(`${name} middleware leaves preflight requests to CORS handling mounted before it`, async () => {
    const { x402, calls } = fakeX402()
    await withServer(x402, { before: [answerPreflights] }, async (origin) => {
      const response = await fetch(`${origin}/a`, { method: 'OPTIONS' })
      assert.equal(response.status, 204)
    })
    assert.equal(calls.createChallenge.length, 0)
  })
}
