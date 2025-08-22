const { initLogging, defaultConfig, SdkBuilder, parse } = require('@breeztech/breez-sdk-spark/nodejs')
const fs = require('fs')
const qrcode = require('qrcode')
const { question, confirm } = require('./prompt.js')
require('dotenv').config()

const logFile = fs.createWriteStream(__dirname + '/../sdk.log', { flags: 'a' })

class JsFileLogger {
  log = (logEntry) => {
    const logMessage = `[${new Date().toISOString()} ${logEntry.level}]: ${logEntry.line}\n`
    logFile.write(logMessage)
  }
}

const fileLogger = new JsFileLogger()

class JsEventListener {
  onEvent = (event) => {
    fileLogger.log({
      level: 'INFO',
      line: `Received event: ${JSON.stringify(event)}`
    })
  }
}

const eventListener = new JsEventListener()

let sdk = null

const initSdk = async () => {
  if (sdk) return sdk

  // Set the logger to trace
  initLogging(fileLogger)

  // Get the mnemonic
  const mnemonic = process.env.MNEMONIC

  // Connect using the config
  let config = defaultConfig('regtest')

  let sdkBuilder = SdkBuilder.new(config, mnemonic, './.data')
  sdkBuilder = sdkBuilder.withRestChainService('https://regtest-mempool.loadtest.dev.sparkinfra.net/api', {
    username: process.env.CHAIN_SERVICE_USERNAME,
    password: process.env.CHAIN_SERVICE_PASSWORD
  })

  sdk = await sdkBuilder.build()

  await sdk.addEventListener(eventListener)
  return sdk
}

const getInfo = async () => {
  const sdk = await initSdk()
  const res = await sdk.getInfo({})
  console.log(JSON.stringify(res, null, 2))
}

const disconnect = () => {
  if (sdk) {
    sdk.disconnect()
  }
  process.exit(0)
}

const receivePayment = async (options) => {
  const sdk = await initSdk()

  let paymentMethod
  if (options.paymentMethod === 'bolt11Invoice') {
    // For bolt11 invoice, we need description and optionally amount_sats
    let description = options.description
    let amountSats = options.amountSats

    // If description is not provided via command line, prompt for it
    if (!description) {
      description = await question('Enter description for the bolt11 invoice')
    }

    // If amount_sats is not provided via command line, ask if user wants to set it
    if (!amountSats) {
      const setAmount = await question('Do you want to set a specific amount? (y/n)')
      if (setAmount.toLowerCase() === 'y' || setAmount.toLowerCase() === 'yes') {
        const amountStr = await question('Enter amount in satoshis')
        amountSats = parseInt(amountStr)
        if (isNaN(amountSats)) {
          throw new Error('Invalid amount provided')
        }
      }
    }

    paymentMethod = {
      type: 'bolt11Invoice',
      description: description,
      amountSats: amountSats || null
    }
  } else {
    paymentMethod = { type: options.paymentMethod }
  }

  const prepareResponse = await sdk.prepareReceivePayment({ paymentMethod: paymentMethod })
  const fees = prepareResponse.feeSats

  const message = `Fees: ${fees} sat. Are the fees acceptable?`
  if (await confirm(message)) {
    const res = await sdk.receivePayment({ prepareResponse })
    console.log(JSON.stringify(res, null, 2))
    qrcode.toString(res.paymentRequest, { type: 'terminal', small: true }, (_err, url) => {
      console.log(url)
    })
  }
}

const sendPayment = async (options) => {
  const sdk = await initSdk()

  const prepareResponse = await sdk.prepareSendPayment({
    paymentRequest: options.paymentRequest,
    amountSats: options.amountSats
  })

  const paymentMethod = prepareResponse.paymentMethod;
  if (paymentMethod.type == 'bolt11Invoice') {
    console.error("prefer spark fees", paymentMethod.sparkTransferFeeSats)

    const fees = paymentMethod.sparkTransferFeeSats != null ? paymentMethod.sparkTransferFeeSats : paymentMethod.lightningFeeSats
    const amount = prepareResponse.amountSats

    const message = `Amount: ${amount} sat. Fees: ${fees} sat. Are the fees acceptable?`
    if (await confirm(message)) {
      const res = await sdk.sendPayment({ prepareResponse, options: { type: 'bolt11Invoice', useSpark: paymentMethod.sparkTransferFeeSats != null } })
      console.log(JSON.stringify(res, null, 2))
    }
  }

}

const lnurlPay = async (options) => {
  const sdk = await initSdk()
  const input = await parse(options.lnurl)

  if (input.type !== 'lnurlPay') {
    throw new Error('Invalid input: expected LNURL pay request')
  }

  const data = input
  const minSendable = Math.ceil(data.minSendable / 1000)
  const maxSendable = Math.floor(data.maxSendable / 1000)


  const amountStr = await question(`Amount to pay (min ${minSendable} sat, max ${maxSendable} sat): `)
  const amountSats = parseInt(amountStr)
  if (isNaN(amountSats)) {
    throw new Error('Invalid amount provided')
  }


  const prepareResponse = await sdk.prepareLnurlPay({
    amountSats: amountSats,
    comment: options.comment,
    data: data,
    validateSuccessActionUrl: options.validateSuccessUrl
  })

  console.log(`Prepared payment: ${JSON.stringify(prepareResponse, null, 2)}`)

  const message = `Amount: ${prepareResponse.amountSats} sat. Fees: ${prepareResponse.feeSats} sat. Are the fees acceptable?`
  if (await confirm(message)) {
    const payRes = await sdk.lnurlPay({ prepareResponse })
    console.log(JSON.stringify(payRes, null, 2))
  }
}

const syncWallet = async () => {
  const sdk = await initSdk()
  const res = await sdk.syncWallet({})
  console.log(JSON.stringify(res, null, 2))
}

const listPayments = async (options) => {
  const sdk = await initSdk()
  const res = await sdk.listPayments({
    offset: options.offset,
    limit: options.limit
  })
  console.log(JSON.stringify(res, null, 2))
}

const getPayment = async (options) => {
  const sdk = await initSdk()
  const res = await sdk.getPayment({ paymentId: options.paymentId })
  console.log(JSON.stringify(res, null, 2))
}

module.exports = {
  disconnect,
  getInfo,
  receivePayment,
  sendPayment,
  lnurlPay,
  syncWallet,
  listPayments,
  getPayment
}