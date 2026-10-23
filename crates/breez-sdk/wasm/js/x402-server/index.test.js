import assert from 'node:assert/strict'
import test from 'node:test'

import { exposeX402Headers, requirePayment, x402Express, x402Hono } from './index.js'

const OPTIONS = {
  publicOrigin: 'https://api.example.com',
  priceSats: 21,
  boundHeaders: ['Accept']
}

function fakeX402(settleResults = []) {
  const calls = { createChallenge: [], settle: [] }
  const x402 = {
    createChallenge: async (request) => {
      calls.createChallenge.push(request)
      return { paymentRequired: 'challenge', paymentHash: 'hash' }
    },
    settle: async (request) => {
      calls.settle.push(request)
      return settleResults.shift()
    }
  }
  return { x402, calls }
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
  const created = calls.createChallenge[0]
  assert.equal(created.request.url, 'https://api.example.com/article/A?x=1')
  assert.deepEqual(created.boundNames, ['accept'])
  assert.deepEqual(created.amounts, [{ type: 'lightning', amountSats: 21 }])
})

test('settles a paid request with the same request and bound names', async () => {
  const { x402, calls } = fakeX402([{ success: true, paymentResponse: 'settled' }])
  const request = new Request('https://api.example.com/article/A', {
    method: 'POST',
    headers: { 'PAYMENT-SIGNATURE': 'proof', 'content-type': 'application/json' },
    body: '{"a":1}'
  })
  const result = await requirePayment(request, x402, OPTIONS)
  assert.deepEqual(result, { paid: true, paymentResponse: 'settled' })
  const settled = calls.settle[0]
  assert.equal(settled.payment, 'proof')
  assert.deepEqual(settled.boundNames, ['accept'])
  assert.deepEqual(settled.request.body, Array.from(new TextEncoder().encode('{"a":1}')))
  assert.ok(settled.request.headers.every((h) => h.name !== 'payment-signature'))
})

test('answers a refused payment with a new challenge', async () => {
  const { x402, calls } = fakeX402([
    { success: false, errorReason: 'duplicate_settlement', paymentResponse: 'failed' }
  ])
  const request = new Request('https://api.example.com/a', {
    headers: { 'PAYMENT-SIGNATURE': 'proof' }
  })
  const result = await requirePayment(request, x402, OPTIONS)
  assert.equal(result.response.status, 402)
  assert.equal(result.response.headers.get('PAYMENT-RESPONSE'), 'failed')
  assert.equal(result.response.headers.get('PAYMENT-REQUIRED'), 'challenge')
  assert.equal(calls.createChallenge.length, 1)
})

test('does not offer a new invoice while the payment may still settle', async () => {
  const { x402, calls } = fakeX402([
    {
      success: false,
      errorReason: 'exact_lnbtc_invoice_not_settled',
      paymentResponse: 'pending'
    }
  ])
  const request = new Request('https://api.example.com/a', {
    headers: { 'PAYMENT-SIGNATURE': 'proof' }
  })
  const result = await requirePayment(request, x402, OPTIONS)
  assert.equal(result.response.status, 402)
  assert.equal(result.response.headers.get('PAYMENT-REQUIRED'), null)
  assert.equal(calls.createChallenge.length, 0)
})

function fakeHonoContext(request, appHeaders = {}) {
  const res = new Response(null, { headers: appHeaders })
  return {
    req: { raw: request, method: request.method },
    res,
    header: (name, value) => res.headers.set(name, value)
  }
}

function fakeExpressResponse(appHeaders = {}) {
  const sent = { ...appHeaders }
  const res = {
    status: (status) => {
      sent.status = status
    },
    setHeader: (name, value) => {
      sent[name.toLowerCase()] = value
    },
    getHeader: (name) => sent[name.toLowerCase()]
  }
  return { res, sent }
}

test('answers a payment that cannot be read with 400 and no new challenge', async () => {
  const { x402, calls } = fakeX402([
    { success: false, errorReason: 'invalid_payload', paymentResponse: 'failed' }
  ])
  const request = new Request('https://api.example.com/a', {
    headers: { 'PAYMENT-SIGNATURE': 'garbage' }
  })
  const result = await requirePayment(request, x402, OPTIONS)
  assert.equal(result.response.status, 400)
  assert.equal(result.response.headers.get('PAYMENT-RESPONSE'), 'failed')
  assert.equal(result.response.headers.get('PAYMENT-REQUIRED'), null)
  assert.equal(calls.createChallenge.length, 0)
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

test('Hono middleware serves a paid request with the payment response', async () => {
  const { x402 } = fakeX402([{ success: true, paymentResponse: 'settled' }])
  const c = fakeHonoContext(
    new Request('https://api.example.com/a', { headers: { 'PAYMENT-SIGNATURE': 'proof' } })
  )
  let served = false
  await x402Hono(x402, OPTIONS)(c, async () => {
    served = true
  })
  assert.ok(served)
  assert.equal(c.res.headers.get('PAYMENT-RESPONSE'), 'settled')
})

test('Hono middleware merges the exposed headers with the app\'s', async () => {
  const { x402 } = fakeX402()
  const c = fakeHonoContext(new Request('https://api.example.com/a'), {
    'Access-Control-Expose-Headers': 'X-Request-Id'
  })
  const response = await x402Hono(x402, OPTIONS)(c, async () => {})
  assert.equal(response.status, 402)
  assert.equal(
    c.res.headers.get('Access-Control-Expose-Headers'),
    'X-Request-Id, PAYMENT-REQUIRED, PAYMENT-RESPONSE'
  )
})

test('Hono middleware lets preflight requests through', async () => {
  const { x402, calls } = fakeX402()
  const c = fakeHonoContext(new Request('https://api.example.com/a', { method: 'OPTIONS' }))
  let passed = false
  await x402Hono(x402, OPTIONS)(c, async () => {
    passed = true
  })
  assert.ok(passed)
  assert.equal(calls.createChallenge.length, 0)
})

test('Express middleware lets preflight requests through', async () => {
  const { x402, calls } = fakeX402()
  const req = { headers: {}, method: 'OPTIONS', originalUrl: '/a' }
  const error = await new Promise((resolve) => x402Express(x402, OPTIONS)(req, {}, resolve))
  assert.equal(error, undefined)
  assert.equal(calls.createChallenge.length, 0)
})

test('Express middleware refuses a parsed body', async () => {
  const { x402 } = fakeX402()
  const req = { body: { parsed: true }, headers: {}, method: 'POST', originalUrl: '/a' }
  const error = await new Promise((resolve) => x402Express(x402, OPTIONS)(req, {}, resolve))
  assert.match(String(error), /raw body/)
})

test('Express middleware answers an unpaid request with a challenge', async () => {
  const { x402, calls } = fakeX402()
  const req = { headers: { accept: 'application/json' }, method: 'GET', originalUrl: '/a?b=1' }
  const { res, sent } = fakeExpressResponse({ 'access-control-expose-headers': 'X-Request-Id' })
  await new Promise((resolve) => {
    res.end = resolve
    x402Express(x402, OPTIONS)(req, res, resolve)
  })
  assert.equal(sent.status, 402)
  assert.equal(sent['payment-required'], 'challenge')
  assert.equal(
    sent['access-control-expose-headers'],
    'X-Request-Id, PAYMENT-REQUIRED, PAYMENT-RESPONSE'
  )
  assert.equal(calls.createChallenge[0].request.url, 'https://api.example.com/a?b=1')
})
