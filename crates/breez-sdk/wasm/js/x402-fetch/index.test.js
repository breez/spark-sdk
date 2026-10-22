import assert from 'node:assert/strict'
import test from 'node:test'

import { X402PaymentPendingError, withX402 } from './index.js'

const URL_A = 'https://api.example.com/article/A'

function fakeFetch(responses) {
  const calls = []
  const fetchFn = async (request) => {
    calls.push({
      method: request.method,
      url: request.url,
      signature: request.headers.get('PAYMENT-SIGNATURE'),
      body: await request.text()
    })
    return responses.shift()
  }
  return { fetchFn, calls }
}

function challenge() {
  return new Response(null, { status: 402, headers: { 'PAYMENT-REQUIRED': 'challenge' } })
}

function fakeX402(statuses) {
  const requests = []
  const x402 = {
    pay: async (request) => {
      requests.push(request)
      const status = statuses.shift()
      return {
        status,
        paymentSignature: status === 'paid' ? 'signature' : undefined,
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
  assert.deepEqual(paid.request.headers, [{ name: 'content-type', value: 'application/json' }])
  assert.equal(paid.request.method, 'POST')
  assert.equal(paid.request.url, URL_A)
  assert.deepEqual(paid.request.body, Array.from(new TextEncoder().encode('{"a":1}')))

  assert.deepEqual(calls[1], {
    method: 'POST',
    url: URL_A,
    signature: 'signature',
    body: '{"a":1}'
  })
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

test('pays the same challenge again while pending', async () => {
  const { fetchFn, calls } = fakeFetch([challenge(), new Response('paid content')])
  const { x402, requests } = fakeX402(['pending', 'paid'])
  await withX402(fetchFn, x402, { maxAmountSats: 100 })(URL_A)
  assert.equal(requests.length, 2)
  assert.equal(requests[1].paymentRequired, 'challenge')
  assert.equal(calls.length, 2)
})

test('throws when the payment stays pending', async () => {
  const { fetchFn, calls } = fakeFetch([challenge()])
  const { x402 } = fakeX402(['pending', 'pending'])
  const x402Fetch = withX402(fetchFn, x402, { maxAmountSats: 100, pendingRetries: 1 })
  await assert.rejects(x402Fetch(URL_A), X402PaymentPendingError)
  assert.equal(calls.length, 1)
})

function redirectedTo(response, url) {
  Object.defineProperty(response, 'url', { value: url })
  return response
}

test('pays for the URL that answered after a redirect', async () => {
  const finalUrl = 'https://cdn.example.com/article/A'
  const { fetchFn, calls } = fakeFetch([
    redirectedTo(challenge(), finalUrl),
    new Response('paid content')
  ])
  const { x402, requests } = fakeX402(['paid'])
  const response = await withX402(fetchFn, x402, { maxAmountSats: 100 })(URL_A)
  assert.equal(await response.text(), 'paid content')
  assert.equal(requests[0].request.url, finalUrl)
  assert.equal(calls[1].url, finalUrl)
  assert.equal(calls[1].signature, 'signature')
})

test('checks allowed hosts against the URL that answered', async () => {
  const { fetchFn } = fakeFetch([redirectedTo(challenge(), 'https://other.example/a')])
  const { x402, requests } = fakeX402([])
  const options = { maxAmountSats: 100, allowedHosts: ['api.example.com'] }
  const response = await withX402(fetchFn, x402, options)(URL_A)
  assert.equal(response.status, 402)
  assert.equal(requests.length, 0)
})

test('sends the payment again while it may still settle', async () => {
  const notSettled = () => new Response(null, { status: 402 })
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

test('stops sending the payment once the retries run out', async () => {
  const notSettled = () => new Response(null, { status: 402 })
  const { fetchFn, calls } = fakeFetch([challenge(), notSettled(), notSettled()])
  const { x402 } = fakeX402(['paid'])
  const options = { maxAmountSats: 100, settleRetries: 1, settleRetryDelayMs: 0 }
  const response = await withX402(fetchFn, x402, options)(URL_A)
  assert.equal(response.status, 402)
  assert.equal(calls.length, 3)
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
