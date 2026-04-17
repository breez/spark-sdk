const { initLogging, defaultConfig, defaultPostgresStorageConfig, SdkBuilder } = require('@breeztech/breez-sdk-spark/nodejs')
const fs = require('fs')
const qrcode = require('qrcode')
const { question, confirm } = require('./prompt.js')
require('dotenv').config()

const postgresConnectionString = process.env.POSTGRES_CONNECTION_STRING

BigInt.prototype.toJSON = function () {
    return this.toString()
}

Map.prototype.toJSON = function () {
    return Object.fromEntries(this)
}

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

class JsPaymentObserver {
    beforeSend = async (payments) => {
        fileLogger.log({
            level: 'INFO',
            line: `Before send payments: ${JSON.stringify(payments)}`
        })
    }
}

const paymentObserver = new JsPaymentObserver()

let sdk = null

const initSdk = async () => {
    if (sdk) return sdk

    // Set the logger to trace
    await initLogging(fileLogger)

    // Get the mnemonic
    const mnemonic = process.env.MNEMONIC

    // Connect using the config
    const network = process.env.NETWORK || 'regtest'
    let config = defaultConfig(network)
    config.apiKey = process.env.BREEZ_API_KEY

    let sdkBuilder = SdkBuilder.new(config, { type: 'mnemonic', mnemonic: mnemonic })
    if (postgresConnectionString) {
        sdkBuilder = sdkBuilder.withPostgresBackend(defaultPostgresStorageConfig(postgresConnectionString))
    } else {
        sdkBuilder = await sdkBuilder.withDefaultStorage('./.data')
    }
    sdkBuilder = sdkBuilder.withPaymentObserver(paymentObserver)
    if (process.env.CHAIN_SERVICE_USERNAME && process.env.CHAIN_SERVICE_PASSWORD) {
        sdkBuilder = sdkBuilder.withRestChainService(
            process.env.CHAIN_SERVICE_URL || 'https://regtest-mempool.us-west-2.sparkinfra.net/api',
            'mempoolSpace',
            {
                username: process.env.CHAIN_SERVICE_USERNAME,
                password: process.env.CHAIN_SERVICE_PASSWORD
            }
        )
    }
    sdk = await sdkBuilder.build()

    await sdk.addEventListener(eventListener)
    return sdk
}

const getInfo = async () => {
    const sdk = await initSdk()
    const res = await sdk.getInfo({})
    console.log(JSON.stringify(res, null, 2))
}

const disconnect = async () => {
    if (sdk) {
        await sdk.disconnect()
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

    const res = await sdk.receivePayment({ paymentMethod })
    console.log(JSON.stringify(res, null, 2))
    qrcode.toString(res.paymentRequest, { type: 'terminal', small: true }, (_err, url) => {
        console.log(url)
    })
    if (res.feeSats > 0) {
        console.log(`Fees: ${res.feeSats} sat`)
    }
}

const sendPayment = async (options) => {
    const sdk = await initSdk()

    let conversionOptions
    if (options.fromBitcoin) {
        conversionOptions = {
            conversionType: { type: 'fromBitcoin' },
            maxSlippageBps: options.maxSlippageBps,
        }
    } else if (options.fromToken) {
        conversionOptions = {
            conversionType: { type: 'toBitcoin', fromTokenIdentifier: options.fromToken },
            maxSlippageBps: options.maxSlippageBps,
        }
    }

    const request = {
        paymentRequest: options.paymentRequest,
        amount: options.amount,
        tokenIdentifier: options.tokenIdentifier,
        conversionOptions
    }

    const prepareResponse = await sdk.prepareSendPayment(request)

    if (prepareResponse.conversionEstimate) {
        const est = prepareResponse.conversionEstimate
        console.log(`Conversion estimate: ${est.amountIn} in -> ${est.amountOut} out (fee: ${est.fee})`)
    } else {
        console.log('No conversion estimate in response')
    }

    const paymentMethod = prepareResponse.paymentMethod
    if (paymentMethod.type == 'bolt11Invoice') {
        const fees =
            paymentMethod.sparkTransferFeeSats != null
                ? paymentMethod.sparkTransferFeeSats
                : paymentMethod.lightningFeeSats
        const amount = prepareResponse.amount

        const message = `Amount: ${amount}. Fees: ${fees} sat. Are the fees acceptable?`
        if (await confirm(message)) {
            const res = await sdk.sendPayment({
                prepareResponse
            })
            console.log(JSON.stringify(res, null, 2))
        }
    } else if (paymentMethod.type == 'sparkAddress') {
        const fees = paymentMethod.fee
        const amount = prepareResponse.amount

        const message = `Amount: ${amount}. Fees: ${fees}. Are the fees acceptable?`
        if (await confirm(message)) {
            const res = await sdk.sendPayment({
                prepareResponse
            })
            console.log(JSON.stringify(res, null, 2))
        }
    }
}

const lnurlPay = async (options) => {
    const sdk = await initSdk()
    const input = await sdk.parse(options.lnurl)

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
        payRequest: data,
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
