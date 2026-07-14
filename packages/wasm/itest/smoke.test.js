'use strict'

// Smoke test for the npm package API, exercised the way the reference web
// app uses it (packages/wasm/examples/web): SdkBuilder + default storage,
// events, receive, parse, send between two instances, payment listing, and
// lnurl-pay against the docker LNURL fixture. Deep per-command behavior
// lives in the shared CLI scenarios; this pins the API surface itself.

require('dotenv').config()

const assert = require('node:assert/strict')
const fs = require('fs')
const os = require('os')
const path = require('path')
const test = require('node:test')

const { defaultConfig, SdkBuilder } = require('@breeztech/breez-sdk-spark/nodejs')

const { fundAddress } = require('./helpers/faucet')
const { LnurlFixture, dockerAvailable } = require('./helpers/lnurl-fixture')

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

/** Queue-backed event listener with a predicate wait. */
class EventQueue {
  constructor() {
    this.events = []
  }

  onEvent(event) {
    this.events.push(event)
  }

  async waitFor(predicate, timeoutMs) {
    const deadline = Date.now() + timeoutMs
    for (;;) {
      const event = this.events.find(predicate)
      if (event !== undefined) {
        return event
      }
      if (Date.now() >= deadline) {
        throw new Error(
          `timed out waiting for event; seen: ${this.events.map((e) => e.type).join(', ')}`
        )
      }
      await sleep(200)
    }
  }
}

async function connectWallet(name, lnurlDomain) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), `breez-smoke-${name}-`))
  const config = defaultConfig('regtest')
  if (lnurlDomain !== undefined) {
    config.lnurlDomain = lnurlDomain
  }
  const mnemonic = require('bip39').generateMnemonic()
  let builder = SdkBuilder.new(config, { type: 'mnemonic', mnemonic })
  builder = await builder.withDefaultStorage(dir)
  const sdk = await builder.build()
  const events = new EventQueue()
  await sdk.addEventListener(events)
  return { sdk, events, dir }
}

async function waitForBalance(sdk, floorSats, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  for (;;) {
    const info = await sdk.getInfo({ ensureSynced: true })
    if (info.balanceSats >= floorSats) {
      return info
    }
    if (Date.now() >= deadline) {
      throw new Error(`balance stayed below ${floorSats} (last: ${info.balanceSats})`)
    }
    await sleep(5000)
  }
}

test('npm API smoke: connect, receive, parse, pay, list, lnurl-pay', async (t) => {
  if (!process.env.FAUCET_USERNAME) {
    t.skip('FAUCET_USERNAME not set')
    return
  }
  const haveDocker = dockerAvailable()

  let lnurl
  const wallets = []
  try {
    if (haveDocker) {
      lnurl = await LnurlFixture.start()
    }
    const alice = await connectWallet('alice')
    const bob = await connectWallet('bob', lnurl?.httpUrl)
    wallets.push(alice, bob)

    // Balance and event surface on a fresh wallet.
    const info = await alice.sdk.getInfo({ ensureSynced: true })
    assert.equal(info.balanceSats, 0)
    assert.equal(typeof info.identityPubkey, 'string')
    await alice.events.waitFor((e) => e.type === 'synced', 30_000)

    // Receive requests and parsing.
    const bolt11 = await alice.sdk.receivePayment({
      paymentMethod: { type: 'bolt11Invoice', description: 'smoke test', amountSats: 2500 }
    })
    const parsed = await alice.sdk.parse(bolt11.paymentRequest)
    assert.equal(parsed.type, 'bolt11Invoice')
    assert.equal(parsed.amountMsat, 2_500_000)
    assert.equal(parsed.description, 'smoke test')

    const bobAddress = await bob.sdk.receivePayment({ paymentMethod: { type: 'sparkAddress' } })
    assert.equal((await bob.sdk.parse(bobAddress.paymentRequest)).type, 'sparkAddress')

    // Fund alice through an on-chain deposit and wait for the auto-claim.
    const deposit = await alice.sdk.receivePayment({ paymentMethod: { type: 'bitcoinAddress' } })
    await fundAddress(deposit.paymentRequest, 50_000)
    await waitForBalance(alice.sdk, 40_000, 180_000)

    // Spark payment alice -> bob.
    const prepared = await alice.sdk.prepareSendPayment({
      paymentRequest: { type: 'input', input: bobAddress.paymentRequest },
      amount: 1000n
    })
    assert.equal(prepared.paymentMethod.type, 'sparkAddress')
    const sent = await alice.sdk.sendPayment({ prepareResponse: prepared })
    assert.equal(sent.payment.status, 'completed')
    assert.equal(sent.payment.paymentType, 'send')

    const fetched = await alice.sdk.getPayment({ paymentId: sent.payment.id })
    assert.equal(fetched.payment.id, sent.payment.id)
    const listed = await alice.sdk.listPayments({})
    assert.ok(listed.payments.some((p) => p.id === sent.payment.id))

    await waitForBalance(bob.sdk, 1000, 90_000)

    // lnurl-pay against the local LNURL server (docker only).
    if (haveDocker) {
      const registered = await bob.sdk.registerLightningAddress({
        username: 'smoketest',
        description: 'smoke test address'
      })
      const lnAddress = await alice.sdk.parse(registered.lightningAddress)
      assert.equal(lnAddress.type, 'lightningAddress')
      const preparedLnurl = await alice.sdk.prepareLnurlPay({
        amount: 1500n,
        payRequest: lnAddress.payRequest,
        comment: 'smoke comment'
      })
      const paid = await alice.sdk.lnurlPay({ prepareResponse: preparedLnurl })
      assert.equal(paid.payment.status, 'completed')
    } else {
      console.error('skipping lnurl-pay part: docker not available')
    }
  } finally {
    for (const wallet of wallets) {
      await wallet.sdk.disconnect().catch(() => {})
      fs.rmSync(wallet.dir, { recursive: true, force: true })
    }
    lnurl?.stop()
  }
})
