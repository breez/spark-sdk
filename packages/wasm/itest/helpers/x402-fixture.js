'use strict'

// An HTTP resource charged for with x402, served in-process by its own SDK on
// regtest through the package's x402-server helper. Mirrors the Rust runner's
// fixture (crates/breez-sdk/cli/tests/harness/x402.rs). The SDK only receives,
// so it needs no funds.

const fs = require('fs')
const http = require('http')
const os = require('os')
const path = require('path')

const { defaultConfig, SdkBuilder } = require('@breeztech/breez-sdk-spark/nodejs')

const PRICE_SATS = 100
const CONTENT = '{"content":"paid article"}'

class X402Fixture {
  /** @param {string} dir */
  constructor(dir) {
    this.dir = dir
    this.sdk = undefined
    this.server = undefined
    this.url = ''
  }

  static async start() {
    const fixture = new X402Fixture(fs.mkdtempSync(path.join(os.tmpdir(), 'breez-x402-fixture-')))
    try {
      const config = defaultConfig('regtest')
      config.apiKey = process.env.BREEZ_API_KEY
      const mnemonic = require('bip39').generateMnemonic()
      const builder = await SdkBuilder.new(config, { type: 'mnemonic', mnemonic }).withDefaultStorage(
        fixture.dir
      )
      fixture.sdk = await builder.build()
      // The helper is an ES module.
      const { requirePayment } = await import('@breeztech/breez-sdk-spark/x402-server')
      const x402 = fixture.sdk.getX402()

      let origin = ''
      fixture.server = http.createServer((req, res) => {
        serve(req, res, (request) =>
          requirePayment(request, x402, {
            publicOrigin: origin,
            priceSats: PRICE_SATS,
            boundHeaders: ['accept'],
            description: 'A paid article',
            mimeType: 'application/json'
          })
        ).catch((e) => res.writeHead(500).end(String(e)))
      })
      await new Promise((resolve) => fixture.server.listen(0, '127.0.0.1', resolve))
      origin = `http://127.0.0.1:${fixture.server.address().port}`
      fixture.url = `${origin}/article`
      return fixture
    } catch (e) {
      await fixture.stop()
      throw e
    }
  }

  async stop() {
    if (this.server !== undefined) {
      this.server.closeAllConnections()
      await new Promise((resolve) => this.server.close(resolve))
    }
    await this.sdk?.disconnect().catch(() => {})
    fs.rmSync(this.dir, { recursive: true, force: true })
  }
}

async function serve(req, res, charge) {
  const chunks = []
  for await (const chunk of req) {
    chunks.push(chunk)
  }
  const body = Buffer.concat(chunks)
  const headers = new Headers()
  for (let i = 0; i < req.rawHeaders.length; i += 2) {
    headers.append(req.rawHeaders[i], req.rawHeaders[i + 1])
  }
  // Appended, not resolved: a path starting `//` would otherwise replace the host.
  const request = new Request(`http://fixture.invalid${req.url}`, {
    method: req.method,
    headers,
    body: body.length > 0 ? body : undefined
  })

  const result = await charge(request)
  if (!result.paid) {
    res.writeHead(result.response.status, Object.fromEntries(result.response.headers))
    res.end()
    return
  }
  res.writeHead(200, {
    'content-type': 'application/json',
    'payment-response': result.paymentResponse
  })
  res.end(CONTENT)
}

module.exports = { X402Fixture }
