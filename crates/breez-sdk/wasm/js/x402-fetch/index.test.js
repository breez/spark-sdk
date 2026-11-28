import assert from 'node:assert/strict'
import test from 'node:test'
import util from 'node:util'

import { X402PaymentPendingError, X402PaymentRefusedError, withX402 } from './index.js'

const URL_A = 'https://api.example.com/article/A'

function fakeFetch(responses) {
  const calls = []
  const fetchFn = async (request, init) => {
    calls.push({
      method: request.method,
      url: request.url,
      signature: request.headers.get('PAYMENT-SIGNATURE'),
      body: await request.text(),
      request,
      init
    })
    const next = responses.shift()
    if (next instanceof Error) {
      throw next
    }
    return next
  }
  return { fetchFn, calls }
}

function challenge() {
  return new Response(null, { status: 402, headers: { 'PAYMENT-REQUIRED': 'challenge' } })
}

/** A `PAYMENT-RESPONSE` value refusing a payment for `errorReason`. */
function refusal(errorReason) {
  return Buffer.from(JSON.stringify({ success: false, errorReason, transaction: '' })).toString('base64')
}

function notSettled() {
  return new Response(null, {
    status: 402,
    headers: { 'PAYMENT-RESPONSE': refusal('settlement_pending') }
  })
}

/** `outcomes` are `'paid'`, `'pending'` or an error to throw. */
function fakeX402(outcomes) {
  const requests = []
  const x402 = {
    pay: async (request) => {
      requests.push(request)
      const outcome = outcomes.shift()
      if (outcome instanceof Error) {
        throw outcome
      }
      return {
        status: outcome,
        paymentSignature: outcome === 'paid' ? 'signature' : undefined,
        payment: { id: 'payment', amount: 21n, fees: 1n }
      }
    }
  }
  return { x402, requests }
}

test('passes through responses that are not x402 challenges', async () => {
  const { fetchFn, calls } = fakeFetch([new Response('ok')])
  const { x402, requests } = fakeX402([])
  const response = await withX402(fetchFn, x402, { maxAmountSats: 100 })(URL_A)
  assert.equal(await response.text(), 'ok')
  assert.equal(calls.length, 1)
  assert.equal(requests.length, 0)
})

test('pays a challenge and sends the request again with the signature', async () => {
  const { fetchFn, calls } = fakeFetch([challenge(), new Response('paid content')])
  const { x402, requests } = fakeX402(['paid'])
  const response = await withX402(fetchFn, x402, { maxAmountSats: 100 })(URL_A, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: '{"a":1}'
  })
  assert.equal(await response.text(), 'paid content')

  assert.equal(requests.length, 1)
  const paid = requests[0]
  assert.equal(paid.paymentRequired, 'challenge')
  assert.deepEqual(paid.maxAmount, { type: 'lightning', amountSats: 100 })
  // fetch sends `accept: */*` when the request has none, and the server may bind it.
  assert.deepEqual(paid.request.headers, [
    { name: 'accept', value: '*/*' },
    { name: 'content-type', value: 'application/json' }
  ])
  assert.equal(paid.request.method, 'POST')
  assert.equal(paid.request.url, URL_A)
  assert.deepEqual(paid.request.body, new TextEncoder().encode('{"a":1}'))

  const { method, url, signature, body, request } = calls[1]
  assert.deepEqual({ method, url, signature, body }, {
    method: 'POST',
    url: URL_A,
    signature: 'signature',
    body: '{"a":1}'
  })
  assert.equal(request.headers.get('accept'), '*/*')
})

test('binds an accept header the request sets', async () => {
  const { fetchFn } = fakeFetch([challenge(), new Response('paid content')])
  const { x402, requests } = fakeX402(['paid'])
  await withX402(fetchFn, x402, { maxAmountSats: 100 })(URL_A, {
    headers: { accept: 'application/json' }
  })
  assert.deepEqual(requests[0].request.headers, [{ name: 'accept', value: 'application/json' }])
})

test('sends the paid request with the original request options', async () => {
  const { fetchFn, calls } = fakeFetch([challenge(), new Response('paid content')])
  const { x402 } = fakeX402(['paid'])
  const controller = new AbortController()
  await withX402(fetchFn, x402, { maxAmountSats: 100 })(URL_A, {
    mode: 'same-origin',
    cache: 'no-store',
    credentials: 'omit',
    redirect: 'error',
    referrer: '',
    referrerPolicy: 'no-referrer',
    integrity: 'sha256-abc',
    keepalive: true,
    signal: controller.signal
  })
  const { request } = calls[1]
  assert.equal(request.mode, 'same-origin')
  assert.equal(request.cache, 'no-store')
  assert.equal(request.credentials, 'omit')
  assert.equal(request.redirect, 'manual')
  assert.equal(request.referrer, '')
  assert.equal(request.referrerPolicy, 'no-referrer')
  assert.equal(request.integrity, 'sha256-abc')
  assert.equal(request.keepalive, true)
  controller.abort()
  assert.equal(request.signal.aborted, true)
})

test('does not pay hosts outside allowedHosts', async () => {
  const { fetchFn } = fakeFetch([challenge()])
  const { x402, requests } = fakeX402([])
  const response = await withX402(fetchFn, x402, {
    maxAmountSats: 100,
    allowedHosts: ['other.example.com']
  })(URL_A)
  assert.equal(response.status, 402)
  assert.equal(requests.length, 0)
})

test('matches allowedHosts with the port', async () => {
  const { fetchFn } = fakeFetch([challenge()])
  const { x402, requests } = fakeX402([])
  const response = await withX402(fetchFn, x402, {
    maxAmountSats: 100,
    allowedHosts: ['api.example.com']
  })('https://api.example.com:8443/article/A')
  assert.equal(response.status, 402)
  assert.equal(requests.length, 0)
})

test('pays the same challenge again while pending', async () => {
  const { fetchFn, calls } = fakeFetch([challenge(), new Response('paid content')])
  const { x402, requests } = fakeX402(['pending', 'paid'])
  await withX402(fetchFn, x402, { maxAmountSats: 100 })(URL_A)
  assert.equal(requests.length, 2)
  assert.equal(requests[1].paymentRequired, 'challenge')
  assert.equal(calls.length, 2)
})

test('throws when the payment stays pending, with what resuming needs', async () => {
  const { fetchFn, calls } = fakeFetch([challenge()])
  const { x402, requests } = fakeX402(['pending', 'pending'])
  const x402Fetch = withX402(fetchFn, x402, { maxAmountSats: 100, pendingRetries: 1 })
  await assert.rejects(x402Fetch(URL_A), (error) => {
    assert.ok(error instanceof X402PaymentPendingError)
    assert.equal(error.url, URL_A)
    assert.equal(error.payRequest, requests[0])
    assert.equal(error.payment.id, 'payment')
    assert.equal(error.paymentSignature, undefined)
    return true
  })
  assert.equal(calls.length, 1)
})

function redirectedTo(response, url) {
  Object.defineProperty(response, 'url', { value: url })
  Object.defineProperty(response, 'redirected', { value: true })
  return response
}

test('pays for the URL that answered a same-origin redirect', async () => {
  const finalUrl = 'https://api.example.com/article/B'
  const { fetchFn, calls } = fakeFetch([
    redirectedTo(challenge(), finalUrl),
    new Response('paid content')
  ])
  const { x402, requests } = fakeX402(['paid'])
  const response = await withX402(fetchFn, x402, { maxAmountSats: 100 })(URL_A, {
    headers: { authorization: 'Bearer secret' }
  })
  assert.equal(await response.text(), 'paid content')
  assert.equal(requests[0].request.url, finalUrl)
  assert.equal(calls[1].url, finalUrl)
  assert.equal(calls[1].signature, 'signature')
  assert.equal(calls[1].request.headers.get('authorization'), 'Bearer secret')
})

for (const [what, requested, finalUrl, init] of [
  ['a redirect to another origin', URL_A, 'https://cdn.example.com/article/A'],
  ['a redirect from http to https', 'http://api.example.com/article/A', URL_A],
  ['a redirected POST', URL_A, 'https://api.example.com/article/B', { method: 'POST', body: 'x' }]
]) {
  test(`returns a challenge reached through ${what} unpaid`, async () => {
    const { fetchFn } = fakeFetch([redirectedTo(challenge(), finalUrl)])
    const { x402, requests } = fakeX402([])
    const response = await withX402(fetchFn, x402, { maxAmountSats: 100 })(requested, init)
    assert.equal(response.status, 402)
    assert.equal(requests.length, 0)
  })
}

test('does not follow a redirect answering the paid request', async () => {
  const moved = new Response(null, { status: 303, headers: { location: 'https://other.example/' } })
  const { fetchFn, calls } = fakeFetch([challenge(), moved])
  const { x402 } = fakeX402(['paid'])
  const response = await withX402(fetchFn, x402, { maxAmountSats: 100 })(URL_A)
  assert.equal(response, moved)
  assert.equal(calls[1].request.redirect, 'manual')
})

test('sends the payment again while it may still settle', async () => {
  const { fetchFn, calls } = fakeFetch([
    challenge(),
    notSettled(),
    notSettled(),
    new Response('paid content')
  ])
  const { x402, requests } = fakeX402(['paid'])
  const options = { maxAmountSats: 100, settleRetryDelayMs: 0 }
  const response = await withX402(fetchFn, x402, options)(URL_A)
  assert.equal(await response.text(), 'paid content')
  assert.equal(requests.length, 1)
  assert.deepEqual(
    calls.map((c) => c.signature),
    [null, 'signature', 'signature', 'signature']
  )
})

test('does not send the payment again when the server offers a new challenge', async () => {
  const pendingWithChallenge = new Response(null, {
    status: 402,
    headers: { 'PAYMENT-REQUIRED': 'challenge', 'PAYMENT-RESPONSE': refusal('settlement_pending') }
  })
  const { fetchFn, calls } = fakeFetch([challenge(), pendingWithChallenge])
  const { x402 } = fakeX402(['paid'])
  const options = { maxAmountSats: 100, settleRetryDelayMs: 0 }
  await assert.rejects(withX402(fetchFn, x402, options)(URL_A), X402PaymentRefusedError)
  assert.equal(calls.length, 2)
})

for (const status of [500, 502, 401, 429]) {
  test(`throws with the payment when the paid request gets a ${status}`, async () => {
    const failed = new Response('server error', {
      status,
      headers: { 'PAYMENT-RESPONSE': refusal('unexpected_settle_error') }
    })
    const { fetchFn, calls } = fakeFetch([challenge(), failed])
    const { x402, requests } = fakeX402(['paid'])
    await assert.rejects(withX402(fetchFn, x402, { maxAmountSats: 100 })(URL_A), (error) => {
      assert.ok(error instanceof X402PaymentPendingError)
      assert.equal(error.paymentSignature, 'signature')
      assert.equal(error.response, failed)
      return true
    })
    assert.equal(await failed.text(), 'server error')
    assert.equal(calls.length, 2)
    assert.equal(requests.length, 1)
  })
}

test('returns an error from the resource once the payment settled', async () => {
  const settled = Buffer.from(JSON.stringify({ success: true, transaction: 'aa' })).toString('base64')
  const notFound = new Response(null, { status: 404, headers: { 'PAYMENT-RESPONSE': settled } })
  const { fetchFn } = fakeFetch([challenge(), notFound])
  const { x402 } = fakeX402(['paid'])
  const response = await withX402(fetchFn, x402, { maxAmountSats: 100 })(URL_A)
  assert.equal(response, notFound)
})

test('does not pay again when resuming a payment the server refuses', async () => {
  const { fetchFn, calls } = fakeFetch([challenge()])
  const { x402, requests } = fakeX402([])
  const response = await withX402(fetchFn, x402, { maxAmountSats: 100 })(URL_A, {
    headers: { 'PAYMENT-SIGNATURE': 'resumed' }
  })
  assert.equal(response.status, 402)
  assert.equal(calls[0].signature, 'resumed')
  assert.equal(requests.length, 0)
})

test('throws with the payment once the settle retries run out', async () => {
  const { fetchFn, calls } = fakeFetch([challenge(), notSettled(), notSettled()])
  const { x402 } = fakeX402(['paid'])
  const options = { maxAmountSats: 100, settleRetries: 1, settleRetryDelayMs: 0 }
  await assert.rejects(withX402(fetchFn, x402, options)(URL_A), (error) => {
    assert.ok(error instanceof X402PaymentPendingError)
    assert.equal(error.paymentSignature, 'signature')
    assert.equal(error.url, URL_A)
    return true
  })
  assert.equal(calls.length, 3)
})

test('throws with what was paid when the paid request gets a new challenge', async () => {
  const refused = new Response('refused', {
    status: 402,
    headers: {
      'PAYMENT-REQUIRED': 'challenge',
      'PAYMENT-RESPONSE': refusal('invalid_exact_lnbtc_request_mismatch')
    }
  })
  const { fetchFn, calls } = fakeFetch([challenge(), refused])
  const { x402, requests } = fakeX402(['paid'])
  const options = { maxAmountSats: 100, settleRetryDelayMs: 0 }
  await assert.rejects(withX402(fetchFn, x402, options)(URL_A), (error) => {
    assert.ok(error instanceof X402PaymentRefusedError)
    assert.equal(error.url, URL_A)
    assert.equal(error.payRequest, requests[0])
    assert.equal(error.payment.id, 'payment')
    assert.equal(error.paymentSignature, 'signature')
    assert.equal(error.errorReason, 'invalid_exact_lnbtc_request_mismatch')
    assert.equal(error.response, refused)
    return true
  })
  assert.equal(await refused.text(), 'refused')
  assert.equal(calls.length, 2)
  assert.equal(requests.length, 1)
})

for (const [what, headers, errorReason] of [
  [
    'a reason other than settling',
    { 'PAYMENT-RESPONSE': refusal('duplicate_settlement') },
    'duplicate_settlement'
  ],
  ['no reason', {}, undefined],
  ['a reason that cannot be read', { 'PAYMENT-RESPONSE': 'not base64 JSON' }, undefined]
]) {
  test(`throws at once when the paid request gets a 402 with ${what}`, async () => {
    const { fetchFn, calls } = fakeFetch([challenge(), new Response(null, { status: 402, headers })])
    const { x402 } = fakeX402(['paid'])
    const options = { maxAmountSats: 100, settleRetryDelayMs: 0 }
    await assert.rejects(withX402(fetchFn, x402, options)(URL_A), (error) => {
      assert.ok(error instanceof X402PaymentRefusedError)
      assert.equal(error.errorReason, errorReason)
      return true
    })
    assert.equal(calls.length, 2)
  })
}

for (const [what, headers, errorReason] of [
  [
    'a reason',
    { 'PAYMENT-RESPONSE': refusal('invalid_exact_lnbtc_preimage_malformed') },
    'invalid_exact_lnbtc_preimage_malformed'
  ],
  ['no reason', {}, undefined]
]) {
  test(`throws with what was paid when the paid request gets a 400 with ${what}`, async () => {
    const unread = new Response('bad request', { status: 400, headers })
    const { fetchFn, calls } = fakeFetch([challenge(), unread])
    const { x402, requests } = fakeX402(['paid'])
    await assert.rejects(withX402(fetchFn, x402, { maxAmountSats: 100 })(URL_A), (error) => {
      assert.ok(error instanceof X402PaymentRefusedError)
      assert.equal(error.payRequest, requests[0])
      assert.equal(error.paymentSignature, 'signature')
      assert.equal(error.errorReason, errorReason)
      assert.equal(error.response, unread)
      return true
    })
    assert.equal(calls.length, 2)
  })
}

test('returns a 400 from the resource once the payment settled', async () => {
  const settled = Buffer.from(JSON.stringify({ success: true, transaction: 'hash' })).toString('base64')
  const invalid = new Response('invalid input', {
    status: 400,
    headers: { 'PAYMENT-RESPONSE': settled }
  })
  const { fetchFn } = fakeFetch([challenge(), invalid])
  const { x402 } = fakeX402(['paid'])
  const response = await withX402(fetchFn, x402, { maxAmountSats: 100 })(URL_A)
  assert.equal(response, invalid)
})

test('keeps the proof of payment and the request\'s credentials out of logs', () => {
  const secrets = {
    url: URL_A,
    payRequest: { request: { headers: [{ name: 'authorization', value: 'Bearer secret-token' }] } },
    payment: { id: 'payment', details: { preimage: 'secret-preimage' } },
    paymentSignature: 'secret-signature'
  }
  for (const error of [
    new X402PaymentPendingError({ message: 'pending', ...secrets }),
    new X402PaymentRefusedError({ ...secrets, response: new Response(null, { status: 402 }) })
  ]) {
    for (const logged of [JSON.stringify(error), util.inspect(error, { depth: Infinity })]) {
      assert.doesNotMatch(logged, /secret/)
      assert.match(logged, /api\.example\.com/)
    }
    assert.equal(error.payRequest, secrets.payRequest)
    assert.equal(error.payment, secrets.payment)
    assert.equal(error.paymentSignature, 'secret-signature')
  }
})

test('stops paying once the budget is spent', async () => {
  const { fetchFn } = fakeFetch([challenge(), new Response('paid content'), challenge()])
  const { x402, requests } = fakeX402(['paid'])
  const x402Fetch = withX402(fetchFn, x402, { maxAmountSats: 100, budgetSats: 22 })
  assert.equal(requests.length, 0)
  await x402Fetch(URL_A)
  assert.deepEqual(requests[0].maxAmount, { type: 'lightning', amountSats: 22 })
  const response = await x402Fetch(URL_A)
  assert.equal(response.status, 402)
  assert.equal(requests.length, 1)
})

test('counts a pending payment against the budget', async () => {
  const { fetchFn } = fakeFetch([challenge(), challenge()])
  const { x402, requests } = fakeX402(['pending'])
  const x402Fetch = withX402(fetchFn, x402, {
    maxAmountSats: 100,
    budgetSats: 22,
    pendingRetries: 0
  })
  await assert.rejects(x402Fetch(URL_A), X402PaymentPendingError)
  const response = await x402Fetch(URL_A)
  assert.equal(response.status, 402)
  assert.equal(requests.length, 1)
})

/** An error as the SDK throws it for an x402 failure. */
function x402Error(kind, reason) {
  return Object.assign(new Error(`x402 ${kind} (${reason}): details`), {
    x402: { kind, reason, details: 'details' }
  })
}

test('counts an error that may follow a payment at its cap, and says how to resume', async () => {
  const { fetchFn } = fakeFetch([challenge(), challenge()])
  const failure = new Error('Network error: connection reset')
  const { x402, requests } = fakeX402([failure])
  const x402Fetch = withX402(fetchFn, x402, { maxAmountSats: 30, budgetSats: 30 })
  await assert.rejects(x402Fetch(URL_A), (error) => {
    assert.ok(error instanceof X402PaymentPendingError)
    assert.equal(error.cause, failure)
    assert.equal(error.payRequest, requests[0])
    assert.equal(error.payment, undefined)
    return true
  })
  const response = await x402Fetch(URL_A)
  assert.equal(response.status, 402)
  assert.equal(requests.length, 1)
})

for (const [what, error] of [
  ['a refused challenge', x402Error('refused', 'invalid_exact_lnbtc_request_mismatch')],
  ['a failed payment', x402Error('paymentFailed', 'exact_lnbtc_payment_not_paid')],
  ['a balance that can\'t cover it', new Error('Insufficient funds')]
]) {
  test(`does not count ${what} against the budget`, async () => {
    const { fetchFn } = fakeFetch([challenge(), challenge(), new Response('paid content')])
    const { x402, requests } = fakeX402([error, 'paid'])
    const x402Fetch = withX402(fetchFn, x402, { maxAmountSats: 30, budgetSats: 30 })
    await assert.rejects(x402Fetch(URL_A), (thrown) => thrown === error)
    const response = await x402Fetch(URL_A)
    assert.equal(await response.text(), 'paid content')
    assert.equal(requests.length, 2)
  })
}

test('keeps concurrent requests within the budget', async () => {
  const { fetchFn } = fakeFetch([
    challenge(),
    challenge(),
    new Response('paid content')
  ])
  const requests = []
  let release
  const released = new Promise((resolve) => {
    release = resolve
  })
  const x402 = {
    pay: async (request) => {
      requests.push(request)
      await released
      return {
        status: 'paid',
        paymentSignature: 'signature',
        payment: { id: 'payment', amount: 99n, fees: 1n }
      }
    }
  }
  const x402Fetch = withX402(fetchFn, x402, { maxAmountSats: 100, budgetSats: 100 })
  const first = x402Fetch(URL_A)
  const second = await x402Fetch(URL_A)
  release()
  assert.equal(await (await first).text(), 'paid content')
  assert.equal(second.status, 402)
  assert.equal(requests.length, 1)
})

test('counts a payment against the budget before another request can see the budget', async () => {
  let second
  const body = new ReadableStream({
    // The first request releases its challenge once paid, as the second's arrives.
    async cancel() {
      second = await x402Fetch(URL_A)
    }
  })
  const first402 = new Response(body, { status: 402, headers: { 'PAYMENT-REQUIRED': 'challenge' } })
  const { fetchFn } = fakeFetch([
    first402,
    challenge(),
    new Response('paid content'),
    new Response('paid content')
  ])
  const { x402, requests } = fakeX402(['paid', 'paid'])
  const x402Fetch = withX402(fetchFn, x402, { maxAmountSats: 22, budgetSats: 22 })
  assert.equal(await (await x402Fetch(URL_A)).text(), 'paid content')
  assert.equal(second.status, 402)
  assert.equal(requests.length, 1)
})

test('says how to resume when sending the paid request fails', async () => {
  const failure = new TypeError('fetch failed')
  const { fetchFn } = fakeFetch([challenge(), failure])
  const { x402, requests } = fakeX402(['paid'])
  await assert.rejects(withX402(fetchFn, x402, { maxAmountSats: 100 })(URL_A), (error) => {
    assert.ok(error instanceof X402PaymentPendingError)
    assert.equal(error.paymentSignature, 'signature')
    assert.equal(error.payRequest, requests[0])
    assert.equal(error.cause, failure)
    return true
  })
})

for (const reason of ['unsupported_scheme', 'unsupported_network', 'invalid_x402_version']) {
  test(`returns a challenge for another rail unchanged (${reason})`, async () => {
    const offer = new Response('pay with something else', {
      status: 402,
      headers: { 'PAYMENT-REQUIRED': 'challenge' }
    })
    const { fetchFn } = fakeFetch([offer])
    const { x402 } = fakeX402([x402Error('refused', reason)])
    const response = await withX402(fetchFn, x402, { maxAmountSats: 100 })(URL_A)
    assert.equal(response, offer)
    assert.equal(await response.text(), 'pay with something else')
  })
}

test('releases the body of a challenge it pays', async () => {
  let cancelled = false
  const body = new ReadableStream({
    cancel() {
      cancelled = true
    }
  })
  const offer = new Response(body, { status: 402, headers: { 'PAYMENT-REQUIRED': 'challenge' } })
  const { fetchFn } = fakeFetch([offer, new Response('paid content')])
  const { x402 } = fakeX402(['paid'])
  await withX402(fetchFn, x402, { maxAmountSats: 100 })(URL_A)
  assert.ok(cancelled)
})

test('sends both requests with the runtime\'s own fetch options', async () => {
  const { fetchFn, calls } = fakeFetch([challenge(), new Response('paid content')])
  const { x402 } = fakeX402(['paid'])
  const runtime = {
    dispatcher: { name: 'proxy agent' },
    proxy: 'http://proxy.example',
    tls: { rejectUnauthorized: true },
    client: { name: 'deno client' },
    next: { revalidate: 60 },
    cf: { cacheTtl: 5 }
  }
  await withX402(fetchFn, x402, { maxAmountSats: 100 })(URL_A, {
    method: 'POST',
    headers: { 'content-type': 'text/plain' },
    body: 'x',
    signal: new AbortController().signal,
    ...runtime
  })
  assert.deepEqual(
    calls.map((call) => call.init),
    [runtime, runtime]
  )
})

test('binds the URL without its fragment', async () => {
  const { fetchFn, calls } = fakeFetch([challenge(), new Response('paid content')])
  const { x402, requests } = fakeX402(['paid'])
  await withX402(fetchFn, x402, { maxAmountSats: 100 })(`${URL_A}#section`)
  assert.equal(requests[0].request.url, URL_A)
  assert.equal(calls[1].url, URL_A)
})

for (const [what, url, options, paid] of [
  ['plain http', 'http://api.example.com/a', {}, false],
  ['plain http when allowed', 'http://api.example.com/a', { allowHttp: true }, true],
  ['plain http on localhost', 'http://localhost:8080/a', {}, true],
  ['plain http on 127.0.0.0/8', 'http://127.1.2.3/a', {}, true],
  ['plain http on [::1]', 'http://[::1]:8080/a', {}, true],
  ['plain http on a host named like a loopback address', 'http://127.0.0.1.example.com/a', {}, false]
]) {
  test(`${paid ? 'pays' : 'does not pay'} a challenge over ${what}`, async () => {
    const { fetchFn } = fakeFetch([challenge(), new Response('paid content')])
    const { x402, requests } = fakeX402(['paid'])
    const response = await withX402(fetchFn, x402, { maxAmountSats: 100, ...options })(url)
    assert.equal(response.status, paid ? 200 : 402)
    assert.equal(requests.length, paid ? 1 : 0)
  })
}

test('refuses invalid options', () => {
  const { fetchFn } = fakeFetch([])
  const { x402 } = fakeX402([])
  for (const options of [
    { maxAmountSats: 1.5 },
    { budgetSats: 2.5 },
    { allowedHosts: 'api.example.com' },
    { allowedHosts: [42] },
    { allowHttp: 'yes' },
    { completionTimeoutSecs: 0 },
    { completionTimeoutSecs: 1.5 },
    { completionTimeoutSecs: 2 ** 32 },
    { pendingRetries: -1 },
    { pendingRetries: 1.5 },
    { settleRetries: -1 },
    { settleRetries: Infinity },
    { settleRetryDelayMs: -1 },
    { settleRetryDelayMs: NaN }
  ]) {
    assert.throws(
      () => withX402(fetchFn, x402, { maxAmountSats: 10, ...options }),
      TypeError,
      util.inspect(options)
    )
  }
})
