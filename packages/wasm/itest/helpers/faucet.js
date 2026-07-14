'use strict'

// Minimal client for the Lightspark regtest faucet, mirroring the Rust
// harness (crates/breez-sdk/cli/tests/harness/faucet.rs).

const DEFAULT_URL = 'https://api.lightspark.com/graphql/spark/rc'
const MAX_RETRIES = 3

/**
 * Fund a regtest bitcoin address, returning the funding txid. Retries with
 * exponential backoff. Reads FAUCET_URL, FAUCET_USERNAME, and
 * FAUCET_PASSWORD from the environment.
 *
 * @param {string} address
 * @param {number} amountSats
 * @returns {Promise<string>}
 */
async function fundAddress(address, amountSats) {
  let lastError
  for (let attempt = 0; attempt <= MAX_RETRIES; attempt++) {
    if (attempt > 0) {
      await new Promise((resolve) => setTimeout(resolve, 2 ** attempt * 1000))
    }
    try {
      return await tryFundAddress(address, amountSats)
    } catch (e) {
      lastError = e
    }
  }
  throw lastError
}

async function tryFundAddress(address, amountSats) {
  const url = process.env.FAUCET_URL || DEFAULT_URL
  const headers = { 'Content-Type': 'application/json' }
  const { FAUCET_USERNAME, FAUCET_PASSWORD } = process.env
  if (FAUCET_USERNAME && FAUCET_PASSWORD) {
    const credentials = Buffer.from(`${FAUCET_USERNAME}:${FAUCET_PASSWORD}`).toString('base64')
    headers.Authorization = `Basic ${credentials}`
  }

  const response = await fetch(url, {
    method: 'POST',
    headers,
    body: JSON.stringify({
      operationName: 'RequestRegtestFunds',
      variables: { amount_sats: amountSats, address },
      query:
        'mutation RequestRegtestFunds($address: String!, $amount_sats: Long!) { ' +
        'request_regtest_funds(input: {address: $address, amount_sats: $amount_sats}) ' +
        '{ transaction_hash}}'
    })
  })
  const body = await response.text()
  if (!response.ok) {
    throw new Error(`faucet request failed with status ${response.status}: ${body}`)
  }
  const parsed = JSON.parse(body)
  if (parsed.errors && parsed.errors.length > 0) {
    throw new Error(`faucet returned errors: ${parsed.errors.map((e) => e.message).join(', ')}`)
  }
  if (!parsed.data) {
    throw new Error(`faucet response has no data: ${body}`)
  }
  return parsed.data.request_regtest_funds.transaction_hash
}

module.exports = { fundAddress }
