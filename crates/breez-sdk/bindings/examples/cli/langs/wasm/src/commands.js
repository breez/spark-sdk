'use strict'

const { Command, Option } = require('commander')
const crypto = require('crypto')
const { printValue } = require('./serialization')
const { registerIssuerCommands } = require('./issuer')
const { registerContactsCommands } = require('./contacts')
const { registerWebhooksCommands } = require('./webhooks')

// ---------------------------------------------------------------------------
// Command names for tab completion
// ---------------------------------------------------------------------------

/**
 * All command names including subcommands, used for REPL tab completion.
 */
const COMMAND_NAMES = [
  'get-info',
  'get-payment',
  'sync',
  'list-payments',
  'receive',
  'pay',
  'lnurl-pay',
  'lnurl-withdraw',
  'lnurl-auth',
  'claim-htlc-payment',
  'claim-deposit',
  'parse',
  'refund-deposit',
  'list-unclaimed-deposits',
  'buy-bitcoin',
  'check-lightning-address-available',
  'get-lightning-address',
  'register-lightning-address',
  'delete-lightning-address',
  'list-fiat-currencies',
  'list-fiat-rates',
  'recommended-fees',
  'get-tokens-metadata',
  'fetch-conversion-limits',
  'get-user-settings',
  'set-user-settings',
  'get-spark-status',
  'issuer token-balance',
  'issuer token-metadata',
  'issuer create-token',
  'issuer mint-token',
  'issuer burn-token',
  'issuer freeze-token',
  'issuer unfreeze-token',
  'contacts add',
  'contacts update',
  'contacts delete',
  'contacts list',
  'webhooks register',
  'webhooks unregister',
  'webhooks list'
]

/**
 * Prompt the user for input via readline.
 *
 * @param {import('readline').Interface} rl - The readline interface
 * @param {string} prompt - The prompt to display
 * @returns {Promise<string>} The user's input
 */
function question(rl, prompt) {
  return new Promise((resolve) => {
    rl.question(prompt, (answer) => {
      resolve(answer)
    })
  })
}

/**
 * Prompt the user for input with a default value.
 *
 * @param {import('readline').Interface} rl - The readline interface
 * @param {string} prompt - The prompt to display
 * @param {string} defaultVal - The default value if user presses enter
 * @returns {Promise<string>} The user's input or default value
 */
async function questionWithDefault(rl, prompt, defaultVal) {
  const answer = await question(rl, prompt)
  return answer.trim() === '' ? defaultVal : answer.trim()
}

/**
 * Build the commander program with all commands registered.
 *
 * @param {() => object} getSdk - Function that returns the SDK instance
 * @param {() => object} getTokenIssuer - Function that returns the TokenIssuer instance
 * @param {() => object} getGetSparkStatus - Function that returns the getSparkStatus function
 * @param {import('readline').Interface} rl - The readline interface for interactive prompts
 * @returns {Command} The configured commander program
 */
function buildProgram(getSdk, getTokenIssuer, getGetSparkStatus, rl) {
  const program = new Command()
  program.exitOverride()
  program.name('breez-cli').description('CLI client for Breez SDK with Spark')

  // --- get-info ---
  program
    .command('get-info')
    .description('Get balance information')
    .option('-e, --ensure-synced [value]', 'Force sync')
    .action(async (options) => {
      const sdk = getSdk()
      const ensureSynced = options.ensureSynced != null
        ? options.ensureSynced === 'true' || options.ensureSynced === true
        : undefined
      const value = await sdk.getInfo({ ensureSynced })
      printValue(value)
    })

  // --- get-payment ---
  program
    .command('get-payment')
    .description('Get the payment with the given ID')
    .argument('<payment_id>', 'The ID of the payment to retrieve')
    .action(async (paymentId) => {
      const sdk = getSdk()
      const value = await sdk.getPayment({ paymentId })
      printValue(value)
    })

  // --- sync ---
  program
    .command('sync')
    .description('Sync wallet state')
    .action(async () => {
      const sdk = getSdk()
      const value = await sdk.syncWallet({})
      printValue(value)
    })

  // --- list-payments ---
  program
    .command('list-payments')
    .description('List payments')
    .option('-t, --type-filter <types...>', 'Filter by payment type')
    .option('-s, --status-filter <statuses...>', 'Filter by payment status')
    .option('-a, --asset-filter <filter>', 'Filter by asset (bitcoin or token:<identifier>)')
    .option('--spark-htlc-status-filter <statuses...>', 'Filter by Spark HTLC status')
    .option('--tx-hash <hash>', 'Filter by token transaction hash')
    .option('--tx-type <type>', 'Filter by token transaction type')
    .option('--from-timestamp <timestamp>', 'Only include payments created after this timestamp (inclusive)', parseInt)
    .option('--to-timestamp <timestamp>', 'Only include payments created before this timestamp (exclusive)', parseInt)
    .option('-l, --limit <number>', 'Number of payments to show', parseInt, 10)
    .option('-o, --offset <number>', 'Number of payments to skip', parseInt, 0)
    .option('--sort-ascending [value]', 'Sort payments in ascending order')
    .action(async (options) => {
      const sdk = getSdk()

      // Build payment details filter
      const paymentDetailsFilter = []
      if (options.sparkHtlcStatusFilter) {
        paymentDetailsFilter.push({
          type: 'spark',
          htlcStatus: options.sparkHtlcStatusFilter
        })
      }
      if (options.txHash) {
        paymentDetailsFilter.push({
          type: 'token',
          txHash: options.txHash
        })
      }
      if (options.txType) {
        paymentDetailsFilter.push({
          type: 'token',
          txType: options.txType
        })
      }

      // Build asset filter
      let assetFilter
      if (options.assetFilter) {
        if (options.assetFilter === 'bitcoin') {
          assetFilter = { type: 'bitcoin' }
        } else if (options.assetFilter.startsWith('token:')) {
          assetFilter = { type: 'token', tokenIdentifier: options.assetFilter.slice(6) }
        }
      }

      const sortAscending = options.sortAscending != null
        ? options.sortAscending === 'true' || options.sortAscending === true
        : undefined

      const value = await sdk.listPayments({
        typeFilter: options.typeFilter,
        statusFilter: options.statusFilter,
        assetFilter,
        paymentDetailsFilter: paymentDetailsFilter.length > 0 ? paymentDetailsFilter : undefined,
        fromTimestamp: options.fromTimestamp,
        toTimestamp: options.toTimestamp,
        limit: options.limit,
        offset: options.offset,
        sortAscending
      })
      printValue(value)
    })

  // --- receive ---
  program
    .command('receive')
    .description('Receive a payment')
    .requiredOption('-m, --method <method>', 'Payment method: sparkaddress, sparkinvoice, bitcoin, bolt11')
    .option('-d, --description <text>', 'Optional description for the invoice')
    .option('-a, --amount <number>', 'The amount the payer should send, in sats or token base units')
    .option('-t, --token-identifier <id>', 'Optional token identifier (spark invoice only)')
    .option('-e, --expiry-secs <seconds>', 'Optional expiry time in seconds from now', parseInt)
    .option('-s, --sender-public-key <key>', 'Optional sender public key (spark invoice only)')
    .option('--hodl', 'Create a HODL invoice (bolt11 only)', false)
    .option('--new-address', 'Request a new bitcoin deposit address instead of reusing the current one', false)
    .action(async (options) => {
      const sdk = getSdk()
      let paymentMethod

      const method = options.method.toLowerCase()

      switch (method) {
        case 'sparkaddress': {
          paymentMethod = { type: 'sparkAddress' }
          break
        }
        case 'sparkinvoice': {
          const amount = options.amount != null ? options.amount : undefined
          let expiryTime
          if (options.expirySecs != null) {
            expiryTime = Math.floor(Date.now() / 1000) + options.expirySecs
          }
          paymentMethod = {
            type: 'sparkInvoice',
            amount,
            tokenIdentifier: options.tokenIdentifier,
            expiryTime,
            description: options.description,
            senderPublicKey: options.senderPublicKey
          }
          break
        }
        case 'bitcoin': {
          paymentMethod = { type: 'bitcoinAddress', newAddress: options.newAddress || undefined }
          break
        }
        case 'bolt11': {
          let paymentHash
          if (options.hodl) {
            const preimageBytes = crypto.randomBytes(32)
            const preimage = preimageBytes.toString('hex')
            const hash = crypto.createHash('sha256').update(preimageBytes).digest('hex')

            console.log(`HODL invoice preimage: ${preimage}`)
            console.log(`Payment hash: ${hash}`)
            console.log("Save the preimage! Use `claim-htlc-payment` with it to settle.")

            paymentHash = hash
          }

          paymentMethod = {
            type: 'bolt11Invoice',
            description: options.description || '',
            amountSats: options.amount != null ? parseInt(options.amount, 10) : undefined,
            expirySecs: options.expirySecs,
            paymentHash
          }
          break
        }
        default: {
          throw new Error(`Invalid payment method: ${method}. Use: sparkaddress, sparkinvoice, bitcoin, bolt11`)
        }
      }

      const receiveResult = await sdk.receivePayment({ paymentMethod })

      if (receiveResult.fee > 0) {
        console.log(`Prepared payment requires fee of ${receiveResult.fee} sats/token base units\n`)
      }

      printValue(receiveResult)
    })

  // --- pay ---
  program
    .command('pay')
    .description('Pay the given payment request')
    .requiredOption('-r, --payment-request <request>', 'The payment request to pay')
    .option('-a, --amount <number>', 'Optional amount to pay')
    .option('-t, --token-identifier <id>', 'Optional token identifier')
    .option('-i, --idempotency-key <key>', 'Optional idempotency key')
    .option('--from-bitcoin', 'Convert from Bitcoin to fulfill the payment')
    .option('--from-token <identifier>', 'Convert from the specified token to Bitcoin')
    .option('-s, --convert-max-slippage-bps <bps>', 'Max slippage in basis points for conversion', parseInt)
    .option('--fees-included', 'Deduct fees from the specified amount', false)
    .action(async (options) => {
      const sdk = getSdk()

      // Build conversion options
      let conversionOptions
      if (options.fromBitcoin) {
        conversionOptions = {
          conversionType: { type: 'fromBitcoin' },
          maxSlippageBps: options.convertMaxSlippageBps,
          completionTimeoutSecs: undefined
        }
      } else if (options.fromToken) {
        conversionOptions = {
          conversionType: { type: 'toBitcoin', fromTokenIdentifier: options.fromToken },
          maxSlippageBps: options.convertMaxSlippageBps,
          completionTimeoutSecs: undefined
        }
      }

      const feePolicy = options.feesIncluded ? 'feesIncluded' : undefined

      const prepareResponse = await sdk.prepareSendPayment({
        paymentRequest: options.paymentRequest,
        amount: options.amount != null ? BigInt(options.amount) : undefined,
        tokenIdentifier: options.tokenIdentifier,
        conversionOptions,
        feePolicy
      })

      // Handle conversion estimate confirmation
      if (prepareResponse.conversionEstimate) {
        const estimate = prepareResponse.conversionEstimate
        const units = estimate.options && estimate.options.conversionType &&
          estimate.options.conversionType.type === 'fromBitcoin' ? 'sats' : 'token base units'
        console.log(`Estimated conversion of ${estimate.amount} ${units} with a ${estimate.fee} ${units} fee`)
        const answer = await questionWithDefault(rl, 'Do you want to continue (y/n): ', 'y')
        if (answer.toLowerCase() !== 'y') {
          throw new Error('Payment cancelled')
        }
      }

      // Read payment options based on method type
      const paymentOptions = await readPaymentOptions(prepareResponse.paymentMethod, rl)

      const sendPaymentResponse = await sdk.sendPayment({
        prepareResponse,
        options: paymentOptions,
        idempotencyKey: options.idempotencyKey
      })

      printValue(sendPaymentResponse)
    })

  // --- lnurl-pay ---
  program
    .command('lnurl-pay')
    .description('Pay using LNURL')
    .argument('<lnurl>', 'LN Address or LNURL-pay endpoint')
    .option('-c, --comment <text>', 'Optional comment for the invoice')
    .option('-v, --validate [value]', 'Validate the success action URL')
    .option('-i, --idempotency-key <key>', 'Optional idempotency key')
    .option('--from-token <identifier>', 'Convert from the specified token to Bitcoin')
    .option('-s, --convert-max-slippage-bps <bps>', 'Max slippage in basis points for conversion', parseInt)
    .option('--fees-included', 'Deduct fees from the specified amount', false)
    .action(async (lnurl, options) => {
      const sdk = getSdk()

      // Build conversion options
      let conversionOptions
      if (options.fromToken) {
        conversionOptions = {
          conversionType: { type: 'toBitcoin', fromTokenIdentifier: options.fromToken },
          maxSlippageBps: options.convertMaxSlippageBps,
          completionTimeoutSecs: undefined
        }
      }

      const feePolicy = options.feesIncluded ? 'feesIncluded' : undefined

      const input = await sdk.parse(lnurl)
      let payRequest
      if (input.type === 'lightningAddress') {
        payRequest = input.payRequest
      } else if (input.type === 'lnurlPay') {
        payRequest = input
      } else {
        throw new Error('Invalid input: expected LNURL-pay or lightning address')
      }

      const minSendable = Math.ceil(payRequest.minSendable / 1000)
      const maxSendable = Math.floor(payRequest.maxSendable / 1000)
      const amountStr = await question(rl, `Amount to pay (min ${minSendable} sat, max ${maxSendable} sat): `)
      const amountSats = parseInt(amountStr, 10)
      if (isNaN(amountSats)) {
        throw new Error('Invalid amount provided')
      }

      const validateSuccessActionUrl = options.validate != null
        ? options.validate === 'true' || options.validate === true
        : undefined

      const prepareResponse = await sdk.prepareLnurlPay({
        amountSats,
        comment: options.comment,
        payRequest,
        validateSuccessActionUrl,
        conversionOptions,
        feePolicy
      })

      // Handle conversion estimate confirmation
      if (prepareResponse.conversionEstimate) {
        const estimate = prepareResponse.conversionEstimate
        console.log(`Estimated conversion of ${estimate.amount} token base units with a ${estimate.fee} token base units fee`)
        const answer = await questionWithDefault(rl, 'Do you want to continue (y/n): ', 'y')
        if (answer.toLowerCase() !== 'y') {
          throw new Error('Payment cancelled')
        }
      }

      console.log(`Prepared payment:`)
      printValue(prepareResponse)
      console.log('Do you want to continue? (y/n)')
      const confirm = await questionWithDefault(rl, '', 'y')
      if (confirm.toLowerCase() !== 'y') {
        return
      }

      const payRes = await sdk.lnurlPay({
        prepareResponse,
        idempotencyKey: options.idempotencyKey
      })

      printValue(payRes)
    })

  // --- lnurl-withdraw ---
  program
    .command('lnurl-withdraw')
    .description('Withdraw using LNURL')
    .argument('<lnurl>', 'LNURL-withdraw endpoint')
    .option('-t, --timeout <seconds>', 'Optional completion timeout in seconds', parseInt)
    .action(async (lnurl, options) => {
      const sdk = getSdk()
      const input = await sdk.parse(lnurl)

      if (input.type !== 'lnurlWithdraw') {
        throw new Error('Invalid input: expected LNURL-withdraw')
      }

      const minWithdrawable = Math.ceil(input.minWithdrawable / 1000)
      const maxWithdrawable = Math.floor(input.maxWithdrawable / 1000)
      const amountStr = await question(rl, `Amount to withdraw (min ${minWithdrawable} sat, max ${maxWithdrawable} sat): `)
      const amountSats = parseInt(amountStr, 10)
      if (isNaN(amountSats)) {
        throw new Error('Invalid amount provided')
      }

      const res = await sdk.lnurlWithdraw({
        amountSats,
        withdrawRequest: input,
        completionTimeoutSecs: options.timeout
      })

      printValue(res)
    })

  // --- lnurl-auth ---
  program
    .command('lnurl-auth')
    .description('Authenticate using LNURL')
    .argument('<lnurl>', 'LNURL-auth endpoint')
    .action(async (lnurl) => {
      const sdk = getSdk()
      const input = await sdk.parse(lnurl)

      if (input.type !== 'lnurlAuth') {
        throw new Error('Invalid input: expected LNURL-auth')
      }

      const action = input.action || 'auth'
      const answer = await questionWithDefault(rl, `Authenticate with ${input.domain} (action: ${action})? (y/n): `, 'y')
      if (answer.toLowerCase() !== 'y') {
        return
      }

      const res = await sdk.lnurlAuth(input)
      printValue(res)
    })

  // --- claim-htlc-payment ---
  program
    .command('claim-htlc-payment')
    .description('Claim an HTLC payment')
    .argument('<preimage>', 'The preimage of the HTLC (hex string)')
    .action(async (preimage) => {
      const sdk = getSdk()
      const res = await sdk.claimHtlcPayment({ preimage })
      printValue(res.payment)
    })

  // --- claim-deposit ---
  program
    .command('claim-deposit')
    .description('Claim an on-chain deposit')
    .argument('<txid>', 'The txid of the deposit')
    .argument('<vout>', 'The vout of the deposit')
    .option('--fee-sat <amount>', 'The max fee to claim the deposit', parseInt)
    .option('--sat-per-vbyte <rate>', 'The max fee per vbyte to claim the deposit', parseInt)
    .option('--recommended-fee-leeway <amount>', 'Use recommended fee with this leeway in sat/vbyte', parseInt)
    .action(async (txid, vout, options) => {
      const sdk = getSdk()
      const voutNum = parseInt(vout, 10)

      let maxFee
      if (options.recommendedFeeLeeway != null) {
        if (options.feeSat != null || options.satPerVbyte != null) {
          throw new Error('Cannot specify fee_sat or sat_per_vbyte when using recommended fee')
        }
        maxFee = { type: 'networkRecommended', leewaySatPerVbyte: options.recommendedFeeLeeway }
      } else if (options.feeSat != null && options.satPerVbyte != null) {
        throw new Error('Cannot specify both fee_sat and sat_per_vbyte')
      } else if (options.feeSat != null) {
        maxFee = { type: 'fixed', amount: options.feeSat }
      } else if (options.satPerVbyte != null) {
        maxFee = { type: 'rate', satPerVbyte: options.satPerVbyte }
      }

      const value = await sdk.claimDeposit({
        txid,
        vout: voutNum,
        maxFee
      })
      printValue(value)
    })

  // --- parse ---
  program
    .command('parse')
    .description('Parse an input (invoice, address, LNURL)')
    .argument('<input>', 'The input to parse')
    .action(async (input) => {
      const sdk = getSdk()
      const value = await sdk.parse(input)
      printValue(value)
    })

  // --- refund-deposit ---
  program
    .command('refund-deposit')
    .description('Refund an on-chain deposit')
    .argument('<txid>', 'The txid of the deposit')
    .argument('<vout>', 'The vout of the deposit')
    .argument('<destination_address>', 'Destination address')
    .option('--fee-sat <amount>', 'The fee in sats (fixed)', parseInt)
    .option('--sat-per-vbyte <rate>', 'The fee per vbyte (rate)', parseInt)
    .action(async (txid, vout, destinationAddress, options) => {
      const sdk = getSdk()
      const voutNum = parseInt(vout, 10)

      let fee
      if (options.feeSat != null && options.satPerVbyte != null) {
        throw new Error('Cannot specify both fee_sat and sat_per_vbyte')
      } else if (options.feeSat != null) {
        fee = { type: 'fixed', amount: options.feeSat }
      } else if (options.satPerVbyte != null) {
        fee = { type: 'rate', satPerVbyte: options.satPerVbyte }
      } else {
        throw new Error('Must specify either --fee-sat or --sat-per-vbyte')
      }

      const value = await sdk.refundDeposit({
        txid,
        vout: voutNum,
        destinationAddress,
        fee
      })
      printValue(value)
    })

  // --- list-unclaimed-deposits ---
  program
    .command('list-unclaimed-deposits')
    .description('List unclaimed on-chain deposits')
    .action(async () => {
      const sdk = getSdk()
      const value = await sdk.listUnclaimedDeposits({})
      printValue(value)
    })

  // --- buy-bitcoin ---
  program
    .command('buy-bitcoin')
    .description('Buy Bitcoin using an external provider')
    .option('--provider <provider>', 'Provider to use: "moonpay" (default) or "cashapp"', 'moonpay')
    .option('--amount-sat <amount>', 'Amount in satoshis (meaning depends on provider)', parseInt)
    .option('--redirect-url <url>', 'Custom redirect URL after purchase completion (MoonPay only)')
    .action(async (options) => {
      const sdk = getSdk()
      const provider = (options.provider || 'moonpay').toLowerCase()
      let request
      if (provider === 'cashapp' || provider === 'cash_app' || provider === 'cash-app') {
        request = { type: 'cashApp', amountSats: options.amountSat }
      } else {
        request = { type: 'moonpay', lockedAmountSat: options.amountSat, redirectUrl: options.redirectUrl }
      }
      const value = await sdk.buyBitcoin(request)
      console.log('Open this URL in a browser to complete the purchase:')
      console.log(value.url)
    })

  // --- check-lightning-address-available ---
  program
    .command('check-lightning-address-available')
    .description('Check if a lightning address username is available')
    .argument('<username>', 'The username to check')
    .action(async (username) => {
      const sdk = getSdk()
      const res = await sdk.checkLightningAddressAvailable({ username })
      printValue(res)
    })

  // --- get-lightning-address ---
  program
    .command('get-lightning-address')
    .description('Get registered lightning address')
    .action(async () => {
      const sdk = getSdk()
      const res = await sdk.getLightningAddress()
      printValue(res)
    })

  // --- register-lightning-address ---
  program
    .command('register-lightning-address')
    .description('Register a lightning address')
    .argument('<username>', 'The lightning address username')
    .argument('[description]', 'Description in the lnurl response and the invoice')
    .option('--transfer-pubkey <pubkey>', 'Pubkey of the current owner when taking over a username')
    .option('--transfer-signature <signature>', "Signature by the current owner over 'transfer:{owner}-{username}-{self}'")
    .action(async (username, description, opts) => {
      const sdk = getSdk()
      if (Boolean(opts.transferPubkey) !== Boolean(opts.transferSignature)) {
        throw new Error('--transfer-pubkey and --transfer-signature must be provided together')
      }
      const transfer = opts.transferPubkey
        ? { pubkey: opts.transferPubkey, signature: opts.transferSignature }
        : undefined
      const res = await sdk.registerLightningAddress({
        username,
        description,
        transfer
      })
      printValue(res)
    })

  // --- accept-lightning-address-transfer ---
  program
    .command('accept-lightning-address-transfer')
    .description('Produce a transfer authorization for the current username, granting it to a transferee pubkey')
    .argument('<transferee_pubkey>', 'Hex-encoded secp256k1 compressed pubkey of the new owner')
    .action(async (transfereePubkey) => {
      const sdk = getSdk()
      const res = await sdk.acceptLightningAddressTransfer({
        transfereePubkey,
      })
      printValue(res)
    })

  // --- delete-lightning-address ---
  program
    .command('delete-lightning-address')
    .description('Delete lightning address')
    .action(async () => {
      const sdk = getSdk()
      await sdk.deleteLightningAddress()
      console.log('Lightning address deleted')
    })

  // --- list-fiat-currencies ---
  program
    .command('list-fiat-currencies')
    .description('List fiat currencies')
    .action(async () => {
      const sdk = getSdk()
      const res = await sdk.listFiatCurrencies()
      printValue(res)
    })

  // --- list-fiat-rates ---
  program
    .command('list-fiat-rates')
    .description('List available fiat rates')
    .action(async () => {
      const sdk = getSdk()
      const res = await sdk.listFiatRates()
      printValue(res)
    })

  // --- recommended-fees ---
  program
    .command('recommended-fees')
    .description('Get the recommended BTC fees based on the configured chain service')
    .action(async () => {
      const sdk = getSdk()
      const res = await sdk.recommendedFees()
      printValue(res)
    })

  // --- get-tokens-metadata ---
  program
    .command('get-tokens-metadata')
    .description('Get metadata for token(s)')
    .argument('<token_identifiers...>', 'The token identifiers to get metadata for')
    .action(async (tokenIdentifiers) => {
      const sdk = getSdk()
      const res = await sdk.getTokensMetadata({ tokenIdentifiers })
      printValue(res)
    })

  // --- fetch-conversion-limits ---
  program
    .command('fetch-conversion-limits')
    .description('Fetch conversion limits for a token')
    .argument('<token_identifier>', 'The token identifier')
    .option('-f, --from-bitcoin', 'Convert from bitcoin to token', false)
    .action(async (tokenIdentifier, options) => {
      const sdk = getSdk()

      let request
      if (options.fromBitcoin) {
        request = {
          conversionType: { type: 'fromBitcoin' },
          tokenIdentifier
        }
      } else {
        request = {
          conversionType: { type: 'toBitcoin', fromTokenIdentifier: tokenIdentifier },
          tokenIdentifier: undefined
        }
      }

      const res = await sdk.fetchConversionLimits(request)
      printValue(res)
    })

  // --- get-user-settings ---
  program
    .command('get-user-settings')
    .description('Get user settings')
    .action(async () => {
      const sdk = getSdk()
      const res = await sdk.getUserSettings()
      printValue(res)
    })

  // --- set-user-settings ---
  program
    .command('set-user-settings')
    .description('Update user settings')
    .option('-p, --private [value]', 'Whether spark private mode is enabled')
    .action(async (options) => {
      const sdk = getSdk()
      let sparkPrivateModeEnabled
      if (options.private != null) {
        sparkPrivateModeEnabled = options.private === 'true' || options.private === true
      }
      await sdk.updateUserSettings({ sparkPrivateModeEnabled })
      console.log('User settings updated')
    })

  // --- get-spark-status ---
  program
    .command('get-spark-status')
    .description('Get the status of the Spark network services')
    .action(async () => {
      const getSparkStatus = getGetSparkStatus()
      const res = await getSparkStatus()
      printValue(res)
    })

  // --- issuer subcommands ---
  registerIssuerCommands(program, getTokenIssuer)

  // --- contacts subcommands ---
  registerContactsCommands(program, getSdk)

  // --- webhooks subcommands ---
  registerWebhooksCommands(program, getSdk)

  return program
}

/**
 * Read payment options interactively based on the payment method type.
 *
 * @param {object} paymentMethod - The payment method from prepare response
 * @param {import('readline').Interface} rl - The readline interface
 * @returns {Promise<object|undefined>} The payment options or undefined
 */
async function readPaymentOptions(paymentMethod, rl) {
  if (!paymentMethod || !paymentMethod.type) {
    return undefined
  }

  switch (paymentMethod.type) {
    case 'bitcoinAddress': {
      const feeQuote = paymentMethod.feeQuote
      const fastFee = (feeQuote.speedFast.userFeeSat || 0) + (feeQuote.speedFast.l1BroadcastFeeSat || 0)
      const mediumFee = (feeQuote.speedMedium.userFeeSat || 0) + (feeQuote.speedMedium.l1BroadcastFeeSat || 0)
      const slowFee = (feeQuote.speedSlow.userFeeSat || 0) + (feeQuote.speedSlow.l1BroadcastFeeSat || 0)

      console.log('Please choose payment fee:')
      console.log(`1. Fast: ${fastFee}`)
      console.log(`2. Medium: ${mediumFee}`)
      console.log(`3. Slow: ${slowFee}`)

      const choice = await questionWithDefault(rl, 'Choose (1/2/3): ', '1')
      let confirmationSpeed
      switch (choice) {
        case '1': confirmationSpeed = 'fast'; break
        case '2': confirmationSpeed = 'medium'; break
        case '3': confirmationSpeed = 'slow'; break
        default: throw new Error('Invalid confirmation speed')
      }

      return { type: 'bitcoinAddress', confirmationSpeed }
    }

    case 'bolt11Invoice': {
      const sparkTransferFeeSats = paymentMethod.sparkTransferFeeSats
      const lightningFeeSats = paymentMethod.lightningFeeSats

      if (sparkTransferFeeSats != null) {
        console.log('Choose payment option:')
        console.log(`1. Spark transfer fee: ${sparkTransferFeeSats} sats`)
        console.log(`2. Lightning fee: ${lightningFeeSats} sats`)
        const choice = await questionWithDefault(rl, 'Choose (1/2): ', '1')
        if (choice === '1') {
          return {
            type: 'bolt11Invoice',
            preferSpark: true,
            completionTimeoutSecs: 0
          }
        }
      }

      return {
        type: 'bolt11Invoice',
        preferSpark: false,
        completionTimeoutSecs: 0
      }
    }

    case 'sparkAddress': {
      // HTLC options are only valid for Bitcoin payments, not token payments
      if (paymentMethod.tokenIdentifier) {
        return undefined
      }

      const htlcAnswer = await questionWithDefault(rl, 'Do you want to create an HTLC transfer? (y/n): ', 'n')
      if (htlcAnswer.toLowerCase() !== 'y') {
        return undefined
      }

      const paymentHash = await question(rl, 'Please enter the HTLC payment hash (hex string) or leave empty to generate a new preimage and associated hash: ')
      let finalPaymentHash
      if (paymentHash.trim() === '') {
        const preimageBytes = crypto.randomBytes(32)
        const preimage = preimageBytes.toString('hex')
        finalPaymentHash = crypto.createHash('sha256').update(preimageBytes).digest('hex')

        console.log(`Generated preimage: ${preimage}`)
        console.log(`Associated payment hash: ${finalPaymentHash}`)
      } else {
        finalPaymentHash = paymentHash.trim()
      }

      const expiryStr = await question(rl, 'Please enter the HTLC expiry duration in seconds: ')
      const expiryDurationSecs = parseInt(expiryStr, 10)
      if (isNaN(expiryDurationSecs)) {
        throw new Error('Invalid expiry duration')
      }

      return {
        type: 'sparkAddress',
        htlcOptions: {
          paymentHash: finalPaymentHash,
          expiryDurationSecs
        }
      }
    }

    case 'sparkInvoice': {
      return undefined
    }

    default:
      return undefined
  }
}

module.exports = { buildProgram, COMMAND_NAMES }
