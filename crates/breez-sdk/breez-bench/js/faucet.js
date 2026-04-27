'use strict'

const FAUCET_URL = process.env.FAUCET_URL || 'https://api.lightspark.com/graphql/spark/rc'
const FAUCET_USERNAME = process.env.FAUCET_USERNAME
const FAUCET_PASSWORD = process.env.FAUCET_PASSWORD

const QUERY = 'mutation RequestRegtestFunds($address: String!, $amount_sats: Long!) { request_regtest_funds(input: {address: $address, amount_sats: $amount_sats}) { transaction_hash}}'

async function fundAddress(address, amountSats) {
  if (!FAUCET_USERNAME || !FAUCET_PASSWORD) {
    throw new Error('FAUCET_USERNAME and FAUCET_PASSWORD env vars are required')
  }

  const body = JSON.stringify({
    operationName: 'RequestRegtestFunds',
    variables: { amount_sats: amountSats, address },
    query: QUERY,
  })

  const auth = Buffer.from(`${FAUCET_USERNAME}:${FAUCET_PASSWORD}`).toString('base64')

  const maxRetries = 3
  let lastErr = null
  for (let attempt = 0; attempt <= maxRetries; attempt++) {
    if (attempt > 0) {
      const backoffMs = 2 ** attempt * 1000
      await new Promise((r) => setTimeout(r, backoffMs))
    }
    try {
      const res = await fetch(FAUCET_URL, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          Authorization: `Basic ${auth}`,
        },
        body,
      })
      const json = await res.json()
      if (json.errors && json.errors.length > 0) {
        throw new Error(`Faucet GraphQL error: ${JSON.stringify(json.errors)}`)
      }
      const txid = json.data && json.data.request_regtest_funds && json.data.request_regtest_funds.transaction_hash
      if (!txid) {
        throw new Error(`Unexpected faucet response: ${JSON.stringify(json)}`)
      }
      return txid
    } catch (e) {
      lastErr = e
    }
  }
  throw lastErr
}

module.exports = { fundAddress }
