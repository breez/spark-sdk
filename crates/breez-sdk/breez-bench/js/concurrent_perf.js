'use strict'

require('./bigint_json')

const fs = require('fs')
const os = require('os')
const path = require('path')
const bip39 = require('bip39')

const {
  SdkBuilder,
  defaultConfig,
  defaultPostgresStorageConfig,
  initLogging,
} = require('@breeztech/breez-sdk-spark/nodejs')

const { fundAddress } = require('./faucet.js')

function parseArgs() {
  const args = process.argv.slice(2)
  const opts = {
    totalPayments: 100,
    concurrency: 6,
    minAmount: 100,
    maxAmount: 2000,
    fundingBuffer: 1.5,
    network: 'regtest',
    label: null,
    senderMnemonic: null,
    receiverMnemonic: null,
    senderDataDir: null,
    receiverDataDir: null,
    cleanData: true,
    bucketSecs: 60,
    apiKey: process.env.BREEZ_API_KEY,
    autoOptimize: true,
    multiplicity: null,
    senderPostgres: null,
    receiverPostgres: null,
    senderInstances: 1,
  }
  for (let i = 0; i < args.length; i++) {
    const a = args[i]
    switch (a) {
      case '--total-payments': opts.totalPayments = parseInt(args[++i], 10); break
      case '--concurrency': opts.concurrency = parseInt(args[++i], 10); break
      case '--min-amount': opts.minAmount = parseInt(args[++i], 10); break
      case '--max-amount': opts.maxAmount = parseInt(args[++i], 10); break
      case '--funding-buffer': opts.fundingBuffer = parseFloat(args[++i]); break
      case '--network': opts.network = args[++i]; break
      case '--label': opts.label = args[++i]; break
      case '--sender-mnemonic': opts.senderMnemonic = args[++i]; break
      case '--receiver-mnemonic': opts.receiverMnemonic = args[++i]; break
      case '--sender-data-dir': opts.senderDataDir = args[++i]; break
      case '--receiver-data-dir': opts.receiverDataDir = args[++i]; break
      case '--keep-data': opts.cleanData = false; break
      case '--bucket-secs': opts.bucketSecs = parseInt(args[++i], 10); break
      case '--api-key': opts.apiKey = args[++i]; break
      case '--no-auto-optimize': opts.autoOptimize = false; break
      case '--multiplicity': opts.multiplicity = parseInt(args[++i], 10); break
      case '--sender-postgres': opts.senderPostgres = args[++i]; break
      case '--receiver-postgres': opts.receiverPostgres = args[++i]; break
      case '--sender-instances': opts.senderInstances = parseInt(args[++i], 10); break
      case '-h':
      case '--help': {
        printHelp()
        process.exit(0)
      }
      default:
        console.error(`Unknown arg: ${a}`)
        printHelp()
        process.exit(1)
    }
  }
  return opts
}

function printHelp() {
  console.log('Concurrent payment benchmark for Breez SDK Spark on Node.js (wasm)')
  console.log('')
  console.log('Options:')
  console.log('  --total-payments N          (default: 100)')
  console.log('  --concurrency N             (default: 6)')
  console.log('  --min-amount SATS           (default: 100)')
  console.log('  --max-amount SATS           (default: 2000)')
  console.log('  --funding-buffer F          (default: 1.5)')
  console.log('  --network regtest|mainnet   (default: regtest)')
  console.log('  --label NAME                Optional label for the run')
  console.log('  --sender-mnemonic WORDS     Reuse a sender wallet (default: random)')
  console.log('  --receiver-mnemonic WORDS   Reuse a receiver wallet (default: random)')
  console.log('  --sender-data-dir PATH      Override sender data dir')
  console.log('  --receiver-data-dir PATH    Override receiver data dir')
  console.log('  --keep-data                 Do not remove data dirs at the end')
  console.log('  --bucket-secs N             Throughput histogram bucket size (default: 60)')
  console.log('')
  console.log('Required env: FAUCET_USERNAME, FAUCET_PASSWORD')
  console.log('Optional env: FAUCET_URL, BREEZ_API_KEY')
}

function tmpDir(prefix) {
  return fs.mkdtempSync(path.join(os.tmpdir(), prefix))
}

function rmrf(p) {
  if (!p) return
  try { fs.rmSync(p, { recursive: true, force: true }) } catch (_e) {}
}

function pickAmount(rng, min, max) {
  return Math.floor(rng() * (max - min + 1)) + min
}

function mulberry32(seed) {
  let s = seed >>> 0
  return function () {
    s = (s + 0x6D2B79F5) >>> 0
    let t = s
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

class EventCollector {
  constructor() {
    this.events = []
    this.waiters = []
    this.onEvent = (ev) => {
      this.events.push(ev)
      const remaining = []
      for (const w of this.waiters) {
        if (w.predicate(ev)) {
          w.resolve(ev)
        } else {
          remaining.push(w)
        }
      }
      this.waiters = remaining
    }
  }

  waitFor(predicate, timeoutMs) {
    const matched = this.events.find(predicate)
    if (matched) return Promise.resolve(matched)
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.waiters = this.waiters.filter((w) => w.timer !== timer)
        reject(new Error(`Timeout waiting for event after ${timeoutMs}ms`))
      }, timeoutMs)
      this.waiters.push({ predicate, resolve: (ev) => { clearTimeout(timer); resolve(ev) }, timer })
    })
  }
}

class FileLogger {
  constructor(filePath) {
    this.stream = fs.createWriteStream(filePath, { flags: 'a' })
    this.log = (entry) => {
      try {
        this.stream.write(`[${new Date().toISOString()} ${entry.level}]: ${entry.line}\n`)
      } catch (_e) {}
    }
  }
}

async function buildSdk(opts, role, mnemonic, dataDir, postgres) {
  const config = defaultConfig(opts.network)
  if (opts.apiKey) config.apiKey = opts.apiKey
  if (config.optimizationConfig) {
    config.optimizationConfig.autoEnabled = opts.autoOptimize
    if (opts.multiplicity != null) {
      config.optimizationConfig.multiplicity = opts.multiplicity
    }
  }

  const seed = { type: 'mnemonic', mnemonic, passphrase: undefined }
  let builder = SdkBuilder.new(config, seed)
  if (postgres) {
    builder = builder.withPostgresBackend(defaultPostgresStorageConfig(postgres))
  } else {
    builder = await builder.withDefaultStorage(dataDir)
  }
  const sdk = await builder.build()

  const collector = new EventCollector()
  await sdk.addEventListener(collector)
  return { sdk, events: collector, role }
}

const FAUCET_MAX_PER_CALL = 50000n
const FAUCET_MIN_PER_CALL = 1000n

async function fundAndWait(sender, amountSats, neededSats) {
  const recv = await sender.sdk.receivePayment({ paymentMethod: { type: 'bitcoinAddress' } })
  const address = recv.paymentRequest
  console.log(`[fund] Sender deposit address: ${address}`)

  let chunkIdx = 0
  for (;;) {
    await sender.sdk.syncWallet({})
    const info = await sender.sdk.getInfo({})
    const balance = BigInt(info.balanceSats)
    if (balance >= BigInt(neededSats)) {
      console.log(`[fund] Sender balance: ${balance} sats (target: ${neededSats})`)
      return
    }
    const remaining = BigInt(amountSats) - balance
    if (remaining <= 0n) {
      console.log(`[fund] Reached target funding: ${balance} sats`)
      return
    }
    let chunk = remaining < FAUCET_MAX_PER_CALL ? remaining : FAUCET_MAX_PER_CALL
    if (chunk < FAUCET_MIN_PER_CALL) chunk = FAUCET_MIN_PER_CALL
    chunkIdx++
    console.log(`[fund] Chunk #${chunkIdx}: requesting ${chunk} sats (balance ${balance}/${neededSats})`)
    const txid = await fundAddress(address, Number(chunk))
    console.log(`[fund] Chunk #${chunkIdx} faucet txid: ${txid}`)
    try {
      await sender.events.waitFor((ev) => ev.type === 'claimedDeposits', 240_000)
    } catch (_e) {
      // fall through; balance check below decides
    }
  }
}

async function runOne(sender, receiverAddress, amount) {
  const prep = await sender.sdk.prepareSendPayment({
    paymentRequest: receiverAddress,
    amount: BigInt(amount),
  })
  await sender.sdk.sendPayment({ prepareResponse: prep })
}

async function executePayments(senders, receiverAddress, payments, concurrency) {
  const results = new Array(payments.length)
  const total = payments.length
  const numInstances = senders.length
  let nextIdx = 0
  let completed = 0

  const totalStart = Date.now()

  async function worker(instance, workerId) {
    const sender = senders[instance]
    for (;;) {
      const i = nextIdx++
      if (i >= total) return
      const { id, amount } = payments[i]
      const start = Date.now()
      console.log(`[START] #${id} (inst ${instance} worker ${workerId}): ${amount} sats`)
      let success = false
      let error = null
      try {
        await runOne(sender, receiverAddress, amount)
        success = true
      } catch (e) {
        error = e && e.message ? e.message : String(e)
      }
      const duration = Date.now() - start
      const completedAt = Date.now() - totalStart
      results[i] = { id, amount, durationMs: duration, completedAtMs: completedAt, success, error, instance }
      completed++
      if (success) {
        console.log(`[OK]    #${id} (inst ${instance} worker ${workerId}): ${amount} sats in ${(duration / 1000).toFixed(2)}s   (${completed}/${total})`)
      } else {
        console.log(`[FAIL]  #${id} (inst ${instance} worker ${workerId}): ${error}   (${completed}/${total})`)
      }
    }
  }

  const workers = []
  for (let inst = 0; inst < numInstances; inst++) {
    for (let w = 0; w < concurrency; w++) {
      workers.push(worker(inst, w))
    }
  }
  await Promise.all(workers)

  return { results, totalDurationMs: Date.now() - totalStart }
}

function summarize(results, totalDurationMs, concurrency, bucketSecs, label) {
  const successful = results.filter((r) => r.success)
  const failed = results.filter((r) => !r.success)
  const succN = successful.length

  console.log('')
  console.log('============================================================')
  console.log(`SUMMARY${label ? ` [${label}]` : ''}`)
  console.log('============================================================')
  console.log(`Total payments:       ${results.length}`)
  console.log(`Successful:           ${succN}`)
  console.log(`Failed:               ${failed.length}`)
  console.log(`Concurrency:          ${concurrency}`)
  console.log(`Wall-clock:           ${(totalDurationMs / 1000).toFixed(2)}s`)
  if (succN > 0) {
    const minutes = totalDurationMs / 60000
    console.log(`Throughput:           ${(succN / minutes).toFixed(1)} payments/min`)
    const durations = successful.map((r) => r.durationMs).sort((a, b) => a - b)
    const sum = durations.reduce((a, b) => a + b, 0)
    const mean = sum / durations.length
    const p50 = durations[Math.floor(durations.length * 0.5)]
    const p90 = durations[Math.floor(durations.length * 0.9)]
    const p99 = durations[Math.floor(durations.length * 0.99)]
    console.log(`Per-payment latency: mean ${(mean / 1000).toFixed(2)}s, p50 ${(p50 / 1000).toFixed(2)}s, p90 ${(p90 / 1000).toFixed(2)}s, p99 ${(p99 / 1000).toFixed(2)}s`)
  }

  if (failed.length > 0) {
    console.log('')
    console.log('Failures:')
    const counts = new Map()
    for (const r of failed) {
      const key = (r.error || '').slice(0, 200)
      counts.set(key, (counts.get(key) || 0) + 1)
    }
    for (const [k, v] of counts) {
      console.log(`  ${v}x  ${k}`)
    }
  }

  if (results.length > 0 && bucketSecs > 0) {
    const bucketMs = bucketSecs * 1000
    const totalBuckets = Math.max(1, Math.ceil(totalDurationMs / bucketMs))
    const succBuckets = new Array(totalBuckets).fill(0)
    const failBuckets = new Array(totalBuckets).fill(0)
    for (const r of results) {
      const b = Math.min(totalBuckets - 1, Math.floor(r.completedAtMs / bucketMs))
      if (r.success) succBuckets[b]++
      else failBuckets[b]++
    }
    const maxCount = Math.max(...succBuckets.map((s, i) => s + failBuckets[i]))
    if (maxCount > 0) {
      console.log('')
      console.log(`Throughput histogram (${bucketSecs}s buckets):`)
      console.log(`  ${'window'.padEnd(14)} ${'ok'.padStart(5)}  ${'fail'.padStart(5)}  ${'rate/min'.padStart(10)}  bar`)
      for (let i = 0; i < totalBuckets; i++) {
        const startSec = i * bucketSecs
        const endSec = Math.min(totalDurationMs / 1000, (i + 1) * bucketSecs)
        const succ = succBuckets[i]
        const fail = failBuckets[i]
        const rate = (succ * 60) / Math.max(1, endSec - startSec)
        const bar = '#'.repeat(Math.round(((succ + fail) / maxCount) * 40))
        console.log(`  ${`${startSec}-${endSec.toFixed(0)}s`.padEnd(14)} ${String(succ).padStart(5)}  ${String(fail).padStart(5)}  ${rate.toFixed(1).padStart(10)}  ${bar}`)
      }
    }
  }
}

async function main() {
  const opts = parseArgs()

  if (!opts.totalPayments || opts.totalPayments <= 0) throw new Error('--total-payments must be > 0')
  if (!opts.concurrency || opts.concurrency <= 0) throw new Error('--concurrency must be > 0')
  if (opts.maxAmount < opts.minAmount) throw new Error('--max-amount must be >= --min-amount')

  const senderMnemonic = opts.senderMnemonic || bip39.generateMnemonic()
  const receiverMnemonic = opts.receiverMnemonic || bip39.generateMnemonic()
  const senderDataDir = opts.senderDataDir || tmpDir('breez-bench-sender-')
  const receiverDataDir = opts.receiverDataDir || tmpDir('breez-bench-receiver-')
  fs.mkdirSync(senderDataDir, { recursive: true })
  fs.mkdirSync(receiverDataDir, { recursive: true })

  const senderLog = path.join(senderDataDir, 'sdk.log')
  const receiverLog = path.join(receiverDataDir, 'sdk.log')

  if (opts.senderInstances > 1 && !opts.senderPostgres) {
    throw new Error('--sender-instances > 1 requires --sender-postgres so they share a tree store')
  }

  console.log('Concurrent Spark Transfer Test (Node.js / wasm)')
  console.log('==============================================')
  console.log(`Total payments:    ${opts.totalPayments}`)
  console.log(`Concurrency:       ${opts.concurrency} (per instance)`)
  console.log(`Sender instances:  ${opts.senderInstances}`)
  console.log(`Amount range:      ${opts.minAmount} - ${opts.maxAmount} sats`)
  console.log(`Network:           ${opts.network}`)
  console.log(`Sender backend:    ${opts.senderPostgres ? 'postgres' : 'sqlite'}`)
  console.log(`Receiver backend:  ${opts.receiverPostgres ? 'postgres' : 'sqlite'}`)
  console.log(`Sender data dir:   ${senderDataDir}`)
  console.log(`Receiver data dir: ${receiverDataDir}`)
  console.log(`Sender log:        ${senderLog}`)
  console.log(`Receiver log:      ${receiverLog}`)
  console.log('')

  try {
    await initLogging(new FileLogger(senderLog))
  } catch (_e) {}

  const seed = (Date.now() & 0xFFFFFFFF) >>> 0
  const rng = mulberry32(seed)
  const amounts = []
  for (let i = 0; i < opts.totalPayments; i++) {
    amounts.push(pickAmount(rng, opts.minAmount, opts.maxAmount))
  }
  const totalSend = amounts.reduce((a, b) => a + b, 0)
  const fundingAmount = Math.max(10_000, Math.ceil(totalSend * opts.fundingBuffer))
  console.log(`Total to send:     ${totalSend} sats`)
  console.log(`Funding amount:    ${fundingAmount} sats (buffer x${opts.fundingBuffer})`)

  console.log('')
  console.log('Initializing sender SDK (instance 0)...')
  const senders = []
  const firstSender = await buildSdk(opts, 'sender', senderMnemonic, senderDataDir, opts.senderPostgres)
  senders.push(firstSender)
  console.log('Initializing receiver SDK...')
  const receiver = await buildSdk(opts, 'receiver', receiverMnemonic, receiverDataDir, opts.receiverPostgres)

  console.log('Waiting for initial sync (sender 0)...')
  await firstSender.events.waitFor((ev) => ev.type === 'synced', 120_000).catch(() => {})
  console.log('Waiting for initial sync (receiver)...')
  await receiver.events.waitFor((ev) => ev.type === 'synced', 120_000).catch(() => {})

  console.log('')
  await fundAndWait(firstSender, fundingAmount, totalSend)

  for (let i = 1; i < opts.senderInstances; i++) {
    const dataDir = `${senderDataDir}-inst-${i}`
    fs.mkdirSync(dataDir, { recursive: true })
    console.log(`Initializing sender SDK (instance ${i})...`)
    const inst = await buildSdk(opts, 'sender', senderMnemonic, dataDir, opts.senderPostgres)
    senders.push(inst)
    await inst.events.waitFor((ev) => ev.type === 'synced', 120_000).catch(() => {})
  }

  const recv = await receiver.sdk.receivePayment({ paymentMethod: { type: 'sparkAddress' } })
  const receiverAddress = recv.paymentRequest
  console.log(`Receiver Spark address: ${receiverAddress}`)

  const payments = amounts.map((amount, id) => ({ id, amount }))

  console.log('')
  console.log(
    `Running ${payments.length} payments via ${senders.length} sender instance(s), each with concurrency=${opts.concurrency} (total in-flight cap = ${senders.length * opts.concurrency})...`
  )
  console.log('')

  const { results, totalDurationMs } = await executePayments(senders, receiverAddress, payments, opts.concurrency)

  summarize(results, totalDurationMs, opts.concurrency, opts.bucketSecs, opts.label)

  console.log('')
  console.log('Disconnecting SDKs...')
  for (let i = 0; i < senders.length; i++) {
    try { await senders[i].sdk.disconnect() } catch (_e) {}
  }
  try { await receiver.sdk.disconnect() } catch (_e) {}

  if (opts.cleanData) {
    rmrf(senderDataDir)
    rmrf(receiverDataDir)
    for (let i = 1; i < opts.senderInstances; i++) {
      rmrf(`${senderDataDir}-inst-${i}`)
    }
  }
  console.log('Done.')
}

main().catch((err) => {
  console.error('Fatal:', err)
  process.exit(1)
})
