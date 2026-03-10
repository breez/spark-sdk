/**
 * Command registry and all command handlers for the Breez SDK React Native CLI.
 *
 * Mirrors ALL commands from the Rust CLI:
 *   get-info, get-payment, sync, list-payments, receive, pay, lnurl-pay,
 *   lnurl-withdraw, lnurl-auth, claim-htlc-payment, claim-deposit, parse,
 *   refund-deposit, list-unclaimed-deposits, buy-bitcoin,
 *   check-lightning-address-available, get-lightning-address,
 *   register-lightning-address, delete-lightning-address, list-fiat-currencies,
 *   list-fiat-rates, recommended-fees, get-tokens-metadata,
 *   fetch-conversion-limits, get-user-settings, set-user-settings,
 *   get-spark-status, issuer (subcommand), contacts (subcommand)
 */

import {
  InputType_Tags,
  ReceivePaymentMethod,
  SendPaymentMethod_Tags,
  SendPaymentOptions,
  OnchainConfirmationSpeed,
  ConversionType,
  ConversionOptions,
  FeePolicy,
  PaymentType,
  PaymentStatus,
  AssetFilter,
  PaymentDetailsFilter,
  MaxFee,
  Fee,
  SparkHtlcStatus,
  getSparkStatus,
} from '@breeztech/breez-sdk-spark-react-native'
import type {
  BreezSdkInterface,
  TokenIssuerInterface,
} from '@breeztech/breez-sdk-spark-react-native'
import { generateRandomBytes, sha256Hash, bytesToHex } from './crypto_utils'
import { formatValue } from './serialization'
import { dispatchIssuerCommand } from './issuer'
import { dispatchContactsCommand } from './contacts'

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** A registered CLI command. */
interface CommandDef {
  name: string
  description: string
  run: (sdk: BreezSdkInterface, tokenIssuer: TokenIssuerInterface, args: string[]) => Promise<string>
}

// ---------------------------------------------------------------------------
// Argument parsing helpers
// ---------------------------------------------------------------------------

/**
 * Parse a named flag value from args. Returns the value after the flag, or undefined.
 * Supports both --long-name and -short forms.
 */
function parseFlag(args: string[], ...flags: string[]): string | undefined {
  for (const flag of flags) {
    const idx = args.indexOf(flag)
    if (idx !== -1 && idx + 1 < args.length) {
      return args[idx + 1]
    }
  }
  return undefined
}

/**
 * Check if a boolean flag is present in the argument array.
 */
function hasFlag(args: string[], ...flags: string[]): boolean {
  return flags.some(f => args.includes(f))
}

/**
 * Parse an optional numeric flag. Returns undefined if not provided.
 */
function parseNumericFlag(args: string[], ...flags: string[]): number | undefined {
  const val = parseFlag(args, ...flags)
  if (val === undefined) return undefined
  const num = parseInt(val, 10)
  if (isNaN(num)) return undefined
  return num
}

/**
 * Parse an optional bigint flag. Returns undefined if not provided.
 */
function parseBigIntFlag(args: string[], ...flags: string[]): bigint | undefined {
  const val = parseFlag(args, ...flags)
  if (val === undefined) return undefined
  try {
    return BigInt(val)
  } catch {
    return undefined
  }
}

/**
 * Parse a multi-value flag (comma-separated). Returns undefined if not provided.
 */
function parseMultiFlag(args: string[], ...flags: string[]): string[] | undefined {
  const val = parseFlag(args, ...flags)
  if (val === undefined) return undefined
  return val.split(',').map(s => s.trim()).filter(s => s.length > 0)
}

/**
 * Split a command line string into arguments, handling double-quoted strings.
 */
export function splitArgs(line: string): string[] {
  const args: string[] = []
  let current = ''
  let inQuote = false

  for (const ch of line) {
    if (ch === '"') {
      inQuote = !inQuote
    } else if (ch === ' ' && !inQuote) {
      if (current.length > 0) {
        args.push(current)
        current = ''
      }
    } else {
      current += ch
    }
  }
  if (current.length > 0) {
    args.push(current)
  }
  return args
}

// ---------------------------------------------------------------------------
// Command Names (used for help and completion)
// ---------------------------------------------------------------------------

export const COMMAND_NAMES = [
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
  'issuer',
  'contacts',
]

// ---------------------------------------------------------------------------
// Command Registry
// ---------------------------------------------------------------------------

export function buildCommandRegistry(): Map<string, CommandDef> {
  const registry = new Map<string, CommandDef>()

  const commands: CommandDef[] = [
    { name: 'get-info', description: 'Get balance information', run: handleGetInfo },
    { name: 'get-payment', description: 'Get the payment with the given ID', run: handleGetPayment },
    { name: 'sync', description: 'Sync wallet state', run: handleSync },
    { name: 'list-payments', description: 'List payments', run: handleListPayments },
    { name: 'receive', description: 'Receive a payment', run: handleReceive },
    { name: 'pay', description: 'Pay the given payment request', run: handlePay },
    { name: 'lnurl-pay', description: 'Pay using LNURL', run: handleLnurlPay },
    { name: 'lnurl-withdraw', description: 'Withdraw using LNURL', run: handleLnurlWithdraw },
    { name: 'lnurl-auth', description: 'Authenticate using LNURL', run: handleLnurlAuth },
    { name: 'claim-htlc-payment', description: 'Claim an HTLC payment', run: handleClaimHtlcPayment },
    { name: 'claim-deposit', description: 'Claim an on-chain deposit', run: handleClaimDeposit },
    { name: 'parse', description: 'Parse an input (invoice, address, LNURL)', run: handleParse },
    { name: 'refund-deposit', description: 'Refund an on-chain deposit', run: handleRefundDeposit },
    { name: 'list-unclaimed-deposits', description: 'List unclaimed on-chain deposits', run: handleListUnclaimedDeposits },
    { name: 'buy-bitcoin', description: 'Buy Bitcoin via MoonPay', run: handleBuyBitcoin },
    { name: 'check-lightning-address-available', description: 'Check if a lightning address username is available', run: handleCheckLightningAddress },
    { name: 'get-lightning-address', description: 'Get registered lightning address', run: handleGetLightningAddress },
    { name: 'register-lightning-address', description: 'Register a lightning address', run: handleRegisterLightningAddress },
    { name: 'delete-lightning-address', description: 'Delete lightning address', run: handleDeleteLightningAddress },
    { name: 'list-fiat-currencies', description: 'List fiat currencies', run: handleListFiatCurrencies },
    { name: 'list-fiat-rates', description: 'List available fiat rates', run: handleListFiatRates },
    { name: 'recommended-fees', description: 'Get recommended BTC fees', run: handleRecommendedFees },
    { name: 'get-tokens-metadata', description: 'Get metadata for token(s)', run: handleGetTokensMetadata },
    { name: 'fetch-conversion-limits', description: 'Fetch conversion limits for a token', run: handleFetchConversionLimits },
    { name: 'get-user-settings', description: 'Get user settings', run: handleGetUserSettings },
    { name: 'set-user-settings', description: 'Update user settings', run: handleSetUserSettings },
    { name: 'get-spark-status', description: 'Get Spark network service status', run: handleGetSparkStatus },
    { name: 'issuer', description: 'Token issuer commands (use "issuer help" for details)', run: handleIssuer },
    { name: 'contacts', description: 'Contacts commands (use "contacts help" for details)', run: handleContacts },
  ]

  for (const cmd of commands) {
    registry.set(cmd.name, cmd)
  }

  return registry
}

/**
 * Print a help message listing all available commands.
 */
export function printHelp(registry: Map<string, CommandDef>): string {
  const lines = ['', 'Available commands:']

  const names = Array.from(registry.keys()).sort()
  for (const name of names) {
    const cmd = registry.get(name)!
    lines.push(`  ${name.padEnd(40)} ${cmd.description}`)
  }

  lines.push('')
  lines.push(`  ${'exit / quit'.padEnd(40)} Exit the CLI`)
  lines.push(`  ${'help'.padEnd(40)} Show this help message`)
  lines.push('')

  return lines.join('\n')
}

/**
 * Execute a command string against the SDK.
 *
 * @param input - The raw command line string from the user
 * @param sdk - The BreezSdkInterface instance
 * @param tokenIssuer - The TokenIssuerInterface instance
 * @param registry - The command registry
 * @returns Object with output text and whether to continue the REPL
 */
export async function executeCommand(
  input: string,
  sdk: BreezSdkInterface,
  tokenIssuer: TokenIssuerInterface,
  registry: Map<string, CommandDef>
): Promise<{ output: string; shouldContinue: boolean }> {
  const trimmed = input.trim()

  if (trimmed === '' ) {
    return { output: '', shouldContinue: true }
  }

  if (trimmed === 'exit' || trimmed === 'quit') {
    return { output: 'Goodbye!', shouldContinue: false }
  }

  if (trimmed === 'help') {
    return { output: printHelp(registry), shouldContinue: true }
  }

  const allArgs = splitArgs(trimmed)
  const cmdName = allArgs[0]
  const cmdArgs = allArgs.slice(1)

  const cmd = registry.get(cmdName)
  if (!cmd) {
    return {
      output: `Unknown command: ${cmdName}. Type 'help' for available commands.`,
      shouldContinue: true,
    }
  }

  try {
    const output = await cmd.run(sdk, tokenIssuer, cmdArgs)
    return { output, shouldContinue: true }
  } catch (error: unknown) {
    const message = error instanceof Error ? error.message : String(error)
    return { output: `Error: ${message}`, shouldContinue: true }
  }
}

// ---------------------------------------------------------------------------
// Command Handlers
// ---------------------------------------------------------------------------

// --- get-info ---

async function handleGetInfo(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  // --ensure-synced can be used as a bare flag (presence = true) or with a value
  const ensureSyncedStr = parseFlag(args, '--ensure-synced', '-e')
  let ensureSynced: boolean | undefined
  if (ensureSyncedStr === 'true' || ensureSyncedStr === 'false') {
    ensureSynced = ensureSyncedStr === 'true'
  } else if (hasFlag(args, '--ensure-synced', '-e')) {
    ensureSynced = true
  }

  const result = await sdk.getInfo({ ensureSynced })
  return formatValue(result)
}

// --- get-payment ---

async function handleGetPayment(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  if (args.length < 1) {
    return 'Usage: get-payment <payment_id>'
  }

  const result = await sdk.getPayment({ paymentId: args[0] })
  return formatValue(result)
}

// --- sync ---

async function handleSync(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, _args: string[]): Promise<string> {
  const result = await sdk.syncWallet({})
  return formatValue(result)
}

// --- list-payments ---

async function handleListPayments(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  const limit = parseNumericFlag(args, '--limit', '-l') ?? 10
  const offset = parseNumericFlag(args, '--offset', '-o') ?? 0
  const fromTimestamp = parseBigIntFlag(args, '--from-timestamp')
  const toTimestamp = parseBigIntFlag(args, '--to-timestamp')
  const sortAscendingStr = parseFlag(args, '--sort-ascending')
  const sortAscending = sortAscendingStr !== undefined ? sortAscendingStr === 'true' : undefined

  // Type filter: --type-filter or -t (comma-separated)
  const typeFilterStr = parseMultiFlag(args, '--type-filter', '-t')
  const typeFilter: PaymentType[] | undefined = typeFilterStr?.map(s => {
    switch (s.toLowerCase()) {
      case 'send': return PaymentType.Send
      case 'receive': return PaymentType.Receive
      default: return undefined
    }
  }).filter((v): v is PaymentType => v !== undefined)

  // Status filter: --status-filter or -s (comma-separated)
  const statusFilterStr = parseMultiFlag(args, '--status-filter', '-s')
  const statusFilter: PaymentStatus[] | undefined = statusFilterStr?.map(s => {
    switch (s.toLowerCase()) {
      case 'completed': return PaymentStatus.Completed
      case 'pending': return PaymentStatus.Pending
      case 'failed': return PaymentStatus.Failed
      default: return undefined
    }
  }).filter((v): v is PaymentStatus => v !== undefined)

  // Asset filter: --asset-filter or -a
  const assetFilterStr = parseFlag(args, '--asset-filter', '-a')
  let assetFilter: AssetFilter | undefined
  if (assetFilterStr) {
    if (assetFilterStr.toLowerCase() === 'bitcoin') {
      assetFilter = new AssetFilter.Bitcoin()
    } else if (assetFilterStr.toLowerCase() === 'token') {
      const assetTokenId = parseFlag(args, '--asset-token-id')
      assetFilter = new AssetFilter.Token({ tokenIdentifier: assetTokenId })
    }
  }

  // Payment details filter
  const sparkHtlcStatusFilterStr = parseMultiFlag(args, '--spark-htlc-status-filter')
  const txHash = parseFlag(args, '--tx-hash')
  const txType = parseFlag(args, '--tx-type')

  let paymentDetailsFilter: PaymentDetailsFilter[] | undefined
  if (sparkHtlcStatusFilterStr || txHash || txType) {
    paymentDetailsFilter = []
    if (sparkHtlcStatusFilterStr) {
      const htlcStatuses = sparkHtlcStatusFilterStr.map(s => {
        switch (s.toLowerCase()) {
          case 'waitingforpreimage': return SparkHtlcStatus.WaitingForPreimage
          case 'preimageshared': return SparkHtlcStatus.PreimageShared
          case 'returned': return SparkHtlcStatus.Returned
          default: return undefined
        }
      }).filter((v): v is SparkHtlcStatus => v !== undefined)
      paymentDetailsFilter.push(new PaymentDetailsFilter.Spark({
        htlcStatus: htlcStatuses.length > 0 ? htlcStatuses : undefined,
        conversionRefundNeeded: undefined,
      }))
    }
    if (txHash) {
      paymentDetailsFilter.push(new PaymentDetailsFilter.Token({
        txHash,
        txType: undefined,
        conversionRefundNeeded: undefined,
      }))
    }
    if (txType) {
      paymentDetailsFilter.push(new PaymentDetailsFilter.Token({
        txType: undefined,
        txHash: undefined,
        conversionRefundNeeded: undefined,
      }))
    }
  }

  const result = await sdk.listPayments({
    limit,
    offset,
    typeFilter,
    statusFilter,
    assetFilter,
    paymentDetailsFilter,
    fromTimestamp,
    toTimestamp,
    sortAscending,
  })
  return formatValue(result)
}

// --- receive ---

async function handleReceive(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  const method = parseFlag(args, '--method', '-m')
  if (!method) {
    return 'Usage: receive -m <method> [options]\nMethods: sparkaddress, sparkinvoice, bitcoin, bolt11\n\nOptions:\n  -d, --description <desc>       Optional description\n  -a, --amount <amount>          Amount in sats or token base units\n  -t, --token-identifier <id>    Token identifier (sparkinvoice only)\n  -e, --expiry-secs <secs>       Expiry time in seconds from now\n  -s, --sender-public-key <key>  Sender public key (sparkinvoice only)\n  --hodl                         Create a HODL invoice (bolt11 only)'
  }

  const description = parseFlag(args, '--description', '-d')
  const amountStr = parseFlag(args, '--amount', '-a')
  const tokenIdentifier = parseFlag(args, '--token-identifier', '-t')
  const expirySecsStr = parseFlag(args, '--expiry-secs', '-e')
  const senderPublicKey = parseFlag(args, '--sender-public-key', '-s')
  const hodl = hasFlag(args, '--hodl')

  const amount = amountStr !== undefined ? BigInt(amountStr) : undefined
  const expirySecs = expirySecsStr !== undefined ? parseInt(expirySecsStr, 10) : undefined

  let paymentMethod: InstanceType<typeof ReceivePaymentMethod.SparkAddress>
    | InstanceType<typeof ReceivePaymentMethod.SparkInvoice>
    | InstanceType<typeof ReceivePaymentMethod.BitcoinAddress>
    | InstanceType<typeof ReceivePaymentMethod.Bolt11Invoice>

  const lines: string[] = []

  switch (method.toLowerCase()) {
    case 'sparkaddress':
      paymentMethod = new ReceivePaymentMethod.SparkAddress()
      break

    case 'sparkinvoice': {
      // Compute expiry time as UNIX timestamp from expiry_secs offset
      let expiryTime: bigint | undefined
      if (expirySecs !== undefined) {
        const nowSecs = BigInt(Math.floor(Date.now() / 1000))
        expiryTime = nowSecs + BigInt(expirySecs)
      }

      paymentMethod = new ReceivePaymentMethod.SparkInvoice({
        amount,
        tokenIdentifier,
        expiryTime,
        description,
        senderPublicKey,
      })
      break
    }

    case 'bitcoin':
      paymentMethod = new ReceivePaymentMethod.BitcoinAddress()
      break

    case 'bolt11': {
      let paymentHash: string | undefined

      if (hodl) {
        // Generate a random preimage and compute payment hash
        const preimageBytes = generateRandomBytes(32)
        const preimage = bytesToHex(preimageBytes)
        const hashBytes = sha256Hash(preimageBytes)
        paymentHash = bytesToHex(hashBytes)

        lines.push(`HODL invoice preimage: ${preimage}`)
        lines.push(`Payment hash: ${paymentHash}`)
        lines.push('Save the preimage! Use `claim-htlc-payment` with it to settle.')
      }

      const amountSats = amount !== undefined ? amount : undefined
      paymentMethod = new ReceivePaymentMethod.Bolt11Invoice({
        description: description ?? '',
        amountSats,
        expirySecs,
        paymentHash,
      })
      break
    }

    default:
      return `Invalid payment method: ${method}\nAvailable methods: sparkaddress, sparkinvoice, bitcoin, bolt11`
  }

  const result = await sdk.receivePayment({ paymentMethod })

  if (result.fee > 0) {
    lines.push(`Prepared payment requires fee of ${result.fee} sats/token base units`)
    lines.push('')
  }
  lines.push(formatValue(result))
  return lines.join('\n')
}

// --- pay ---

async function handlePay(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  const paymentRequest = parseFlag(args, '--payment-request', '-r')
  if (!paymentRequest) {
    return 'Usage: pay -r <payment_request> [-a <amount>] [-t <token_identifier>] [-i <idempotency_key>] [--from-bitcoin] [--from-token <token_id>] [-s <max_slippage_bps>] [--fees-included]'
  }

  const amount = parseBigIntFlag(args, '--amount', '-a')
  const tokenIdentifier = parseFlag(args, '--token-identifier', '-t')
  const idempotencyKey = parseFlag(args, '--idempotency-key', '-i')
  const convertFromBitcoin = hasFlag(args, '--from-bitcoin')
  const convertFromTokenIdentifier = parseFlag(args, '--from-token')
  const maxSlippageBps = parseNumericFlag(args, '--convert-max-slippage-bps', '-s')
  const feesIncluded = hasFlag(args, '--fees-included')

  // Build conversion options
  let conversionOptions: ConversionOptions | undefined

  if (convertFromBitcoin) {
    conversionOptions = ConversionOptions.create({
      conversionType: new ConversionType.FromBitcoin(),
      maxSlippageBps,
      completionTimeoutSecs: undefined,
    })
  } else if (convertFromTokenIdentifier) {
    conversionOptions = ConversionOptions.create({
      conversionType: new ConversionType.ToBitcoin({
        fromTokenIdentifier: convertFromTokenIdentifier,
      }),
      maxSlippageBps,
      completionTimeoutSecs: undefined,
    })
  }

  const feePolicy = feesIncluded ? FeePolicy.FeesIncluded : undefined

  const prepareResponse = await sdk.prepareSendPayment({
    paymentRequest,
    amount,
    tokenIdentifier,
    conversionOptions,
    feePolicy,
  })

  const lines: string[] = []

  // Show conversion estimate if present
  if (prepareResponse.conversionEstimate) {
    const est = prepareResponse.conversionEstimate
    const isFromBitcoin = est.options?.conversionType?.tag === 'FromBitcoin'
    const units = isFromBitcoin ? 'sats' : 'token base units'
    lines.push(`Estimated conversion of ${est.amount} ${units} with a ${est.fee} ${units} fee`)
  }

  // Determine payment options based on the payment method type
  let options: InstanceType<typeof SendPaymentOptions.BitcoinAddress>
    | InstanceType<typeof SendPaymentOptions.Bolt11Invoice>
    | InstanceType<typeof SendPaymentOptions.SparkAddress>
    | undefined

  if (prepareResponse.paymentMethod?.tag === SendPaymentMethod_Tags.BitcoinAddress) {
    const feeQuote = prepareResponse.paymentMethod.inner.feeQuote
    const fastFee = feeQuote.speedFast.userFeeSat + feeQuote.speedFast.l1BroadcastFeeSat
    const mediumFee = feeQuote.speedMedium.userFeeSat + feeQuote.speedMedium.l1BroadcastFeeSat
    const slowFee = feeQuote.speedSlow.userFeeSat + feeQuote.speedSlow.l1BroadcastFeeSat
    lines.push(`Fee options:`)
    lines.push(`  1. Fast: ${fastFee} sats`)
    lines.push(`  2. Medium: ${mediumFee} sats`)
    lines.push(`  3. Slow: ${slowFee} sats`)
    lines.push(`Using Medium speed by default. (In interactive mode, user would choose.)`)
    options = new SendPaymentOptions.BitcoinAddress({
      confirmationSpeed: OnchainConfirmationSpeed.Medium,
    })
  } else if (prepareResponse.paymentMethod?.tag === SendPaymentMethod_Tags.Bolt11Invoice) {
    const pm = prepareResponse.paymentMethod.inner
    if (pm.sparkTransferFeeSats !== undefined && pm.sparkTransferFeeSats !== null) {
      lines.push(`Spark transfer fee: ${pm.sparkTransferFeeSats} sats`)
      lines.push(`Lightning fee: ${pm.lightningFeeSats} sats`)
      lines.push(`Using Lightning by default.`)
    }
    options = new SendPaymentOptions.Bolt11Invoice({
      preferSpark: false,
      completionTimeoutSecs: 0,
    })
  } else if (prepareResponse.paymentMethod?.tag === SendPaymentMethod_Tags.SparkAddress) {
    const pm = prepareResponse.paymentMethod.inner
    // HTLC options are only valid for Bitcoin payments, not token payments
    const isTokenPayment = pm.tokenIdentifier !== undefined && pm.tokenIdentifier !== null

    if (!isTokenPayment) {
      // Check for HTLC flags
      const htlcPaymentHash = parseFlag(args, '--htlc-payment-hash')
      const htlcExpiry = parseNumericFlag(args, '--htlc-expiry-secs')

      if (htlcPaymentHash) {
        options = new SendPaymentOptions.SparkAddress({
          htlcOptions: {
            paymentHash: htlcPaymentHash,
            expiryDurationSecs: BigInt(htlcExpiry ?? 3600),
          },
        })
        lines.push(`Creating HTLC transfer with payment hash: ${htlcPaymentHash}`)
      }
    }
  }

  const sendResponse = await sdk.sendPayment({
    prepareResponse,
    options,
    idempotencyKey,
  })

  lines.push(formatValue(sendResponse))
  return lines.join('\n')
}

// --- lnurl-pay ---

async function handleLnurlPay(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  // The first positional argument is the LNURL/lightning address
  const positional = args.filter(a => !a.startsWith('-'))
  const flagArgs = args

  if (positional.length < 1) {
    return 'Usage: lnurl-pay <lnurl_or_address> [-a <amount_sats>] [-c <comment>] [-v <true/false>] [-i <idempotency_key>] [--from-token <token_id>] [-s <max_slippage_bps>] [--fees-included]'
  }

  const lnurl = positional[0]
  const comment = parseFlag(flagArgs, '--comment', '-c')
  const validateStr = parseFlag(flagArgs, '--validate', '-v')
  const validateSuccessUrl = validateStr !== undefined ? validateStr === 'true' : undefined
  const idempotencyKey = parseFlag(flagArgs, '--idempotency-key', '-i')
  const convertFromTokenIdentifier = parseFlag(flagArgs, '--from-token')
  const maxSlippageBps = parseNumericFlag(flagArgs, '--convert-max-slippage-bps', '-s')
  const feesIncluded = hasFlag(flagArgs, '--fees-included')

  // Build conversion options
  let conversionOptions: ConversionOptions | undefined

  if (convertFromTokenIdentifier) {
    conversionOptions = ConversionOptions.create({
      conversionType: new ConversionType.ToBitcoin({
        fromTokenIdentifier: convertFromTokenIdentifier,
      }),
      maxSlippageBps,
      completionTimeoutSecs: undefined,
    })
  }

  const feePolicy = feesIncluded ? FeePolicy.FeesIncluded : undefined

  const input = await sdk.parse(lnurl)

  let payRequest: unknown
  if (input.tag === InputType_Tags.LightningAddress) {
    payRequest = input.inner[0].payRequest
  } else if (input.tag === InputType_Tags.LnurlPay) {
    payRequest = input.inner[0]
  } else {
    return 'Error: Input is not an LNURL-pay or lightning address'
  }

  const lines: string[] = []

  // In the Rust CLI, this prompts for amount interactively.
  // Since we cannot prompt interactively in a single command, require amount as a flag.
  const amountSatsStr = parseFlag(flagArgs, '--amount', '-a')
  if (!amountSatsStr) {
    lines.push(formatValue(payRequest))
    lines.push('')
    lines.push('Please provide amount with -a flag: lnurl-pay <lnurl> -a <amount_sats>')
    return lines.join('\n')
  }

  const amountSats = BigInt(amountSatsStr)

  const prepareResponse = await sdk.prepareLnurlPay({
    amountSats,
    payRequest: payRequest as any,
    comment,
    validateSuccessActionUrl: validateSuccessUrl,
    conversionOptions,
    feePolicy,
  })

  if (prepareResponse.conversionEstimate) {
    const est = prepareResponse.conversionEstimate
    lines.push(`Estimated conversion of ${est.amount} token base units with a ${est.fee} token base units fee`)
  }

  lines.push(`Prepared payment: fees = ${prepareResponse.feeSats} sats`)

  const result = await sdk.lnurlPay({
    prepareResponse,
    idempotencyKey,
  })

  lines.push(formatValue(result))
  return lines.join('\n')
}

// --- lnurl-withdraw ---

async function handleLnurlWithdraw(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  const positional = args.filter(a => !a.startsWith('-'))

  if (positional.length < 1) {
    return 'Usage: lnurl-withdraw <lnurl> [-a <amount_sats>] [--timeout <secs>]'
  }

  const lnurl = positional[0]
  const timeoutStr = parseFlag(args, '--timeout', '-t')
  const completionTimeoutSecs = timeoutStr !== undefined ? parseInt(timeoutStr, 10) : undefined

  const input = await sdk.parse(lnurl)
  if (input.tag !== InputType_Tags.LnurlWithdraw) {
    return 'Error: Input is not an LNURL-withdraw'
  }

  const withdrawRequest = input.inner[0]
  const lines: string[] = []

  // In the Rust CLI, this prompts for amount. Here we require it as a flag.
  const amountSatsStr = parseFlag(args, '--amount', '-a')
  if (!amountSatsStr) {
    lines.push(formatValue(withdrawRequest))
    lines.push('')
    const minWithdrawable = Number((withdrawRequest.minWithdrawable + 999n) / 1000n)
    const maxWithdrawable = Number(withdrawRequest.maxWithdrawable / 1000n)
    lines.push(`Please provide amount with -a flag (min ${minWithdrawable} sat, max ${maxWithdrawable} sat)`)
    return lines.join('\n')
  }

  const amountSats = BigInt(amountSatsStr)

  const result = await sdk.lnurlWithdraw({
    amountSats,
    withdrawRequest,
    completionTimeoutSecs,
  })

  lines.push(formatValue(result))
  return lines.join('\n')
}

// --- lnurl-auth ---

async function handleLnurlAuth(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  if (args.length < 1) {
    return 'Usage: lnurl-auth <lnurl>'
  }

  const input = await sdk.parse(args[0])
  if (input.tag !== InputType_Tags.LnurlAuth) {
    return 'Error: Input is not an LNURL-auth'
  }

  const authRequest = input.inner[0]
  const action = authRequest.action ?? 'auth'
  const lines: string[] = []
  lines.push(`Authenticating with ${authRequest.domain} (action: ${action})`)

  const result = await sdk.lnurlAuth(authRequest)
  lines.push(formatValue(result))
  return lines.join('\n')
}

// --- claim-htlc-payment ---

async function handleClaimHtlcPayment(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  if (args.length < 1) {
    return 'Usage: claim-htlc-payment <preimage>'
  }

  const result = await sdk.claimHtlcPayment({ preimage: args[0] })
  return formatValue(result)
}

// --- claim-deposit ---

async function handleClaimDeposit(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  const positional = args.filter(a => !a.startsWith('-'))
  if (positional.length < 2) {
    return 'Usage: claim-deposit <txid> <vout> [--fee-sat N | --sat-per-vbyte N | --recommended-fee-leeway N]'
  }

  const txid = positional[0]
  const vout = parseInt(positional[1], 10)
  if (isNaN(vout)) {
    return `Invalid vout: ${positional[1]}`
  }

  const feeSatStr = parseFlag(args, '--fee-sat')
  const satPerVbyteStr = parseFlag(args, '--sat-per-vbyte')
  const recommendedFeeLeewayStr = parseFlag(args, '--recommended-fee-leeway')

  let maxFee: MaxFee | undefined

  if (recommendedFeeLeewayStr !== undefined) {
    if (feeSatStr !== undefined || satPerVbyteStr !== undefined) {
      return 'Cannot specify fee_sat or sat_per_vbyte when using recommended fee'
    }
    maxFee = new MaxFee.NetworkRecommended({ leewaySatPerVbyte: BigInt(recommendedFeeLeewayStr) })
  } else if (feeSatStr !== undefined && satPerVbyteStr !== undefined) {
    return 'Cannot specify both --fee-sat and --sat-per-vbyte'
  } else if (feeSatStr !== undefined) {
    maxFee = new MaxFee.Fixed({ amount: BigInt(feeSatStr) })
  } else if (satPerVbyteStr !== undefined) {
    maxFee = new MaxFee.Rate({ satPerVbyte: BigInt(satPerVbyteStr) })
  }

  const result = await sdk.claimDeposit({
    txid,
    vout,
    maxFee,
  })
  return formatValue(result)
}

// --- parse ---

async function handleParse(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  if (args.length < 1) {
    return 'Usage: parse <input>'
  }

  const result = await sdk.parse(args[0])
  return formatValue(result)
}

// --- refund-deposit ---

async function handleRefundDeposit(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  const positional = args.filter(a => !a.startsWith('-'))
  if (positional.length < 3) {
    return 'Usage: refund-deposit <txid> <vout> <destination_address> [--fee-sat N | --sat-per-vbyte N]'
  }

  const txid = positional[0]
  const vout = parseInt(positional[1], 10)
  if (isNaN(vout)) {
    return `Invalid vout: ${positional[1]}`
  }
  const destinationAddress = positional[2]

  const feeSatStr = parseFlag(args, '--fee-sat')
  const satPerVbyteStr = parseFlag(args, '--sat-per-vbyte')

  if (feeSatStr !== undefined && satPerVbyteStr !== undefined) {
    return 'Cannot specify both --fee-sat and --sat-per-vbyte'
  }

  let fee: Fee
  if (feeSatStr !== undefined) {
    fee = new Fee.Fixed({ amount: BigInt(feeSatStr) })
  } else if (satPerVbyteStr !== undefined) {
    fee = new Fee.Rate({ satPerVbyte: BigInt(satPerVbyteStr) })
  } else {
    return 'Must specify either --fee-sat or --sat-per-vbyte'
  }

  const result = await sdk.refundDeposit({
    txid,
    vout,
    destinationAddress,
    fee,
  })
  return formatValue(result)
}

// --- list-unclaimed-deposits ---

async function handleListUnclaimedDeposits(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, _args: string[]): Promise<string> {
  const result = await sdk.listUnclaimedDeposits({})
  return formatValue(result)
}

// --- buy-bitcoin ---

async function handleBuyBitcoin(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  const lockedAmountSatStr = parseFlag(args, '--locked-amount-sat', '--amount')
  const redirectUrl = parseFlag(args, '--redirect-url')

  const lockedAmountSat = lockedAmountSatStr !== undefined ? BigInt(lockedAmountSatStr) : undefined

  const result = await sdk.buyBitcoin({
    lockedAmountSat,
    redirectUrl,
  })

  const lines = [
    'Open this URL in a browser to complete the purchase:',
    result.url,
  ]
  return lines.join('\n')
}

// --- check-lightning-address-available ---

async function handleCheckLightningAddress(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  if (args.length < 1) {
    return 'Usage: check-lightning-address-available <username>'
  }

  const result = await sdk.checkLightningAddressAvailable({ username: args[0] })
  return formatValue(result)
}

// --- get-lightning-address ---

async function handleGetLightningAddress(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, _args: string[]): Promise<string> {
  const result = await sdk.getLightningAddress()
  if (result === null || result === undefined) {
    return 'No lightning address registered'
  }
  return formatValue(result)
}

// --- register-lightning-address ---

async function handleRegisterLightningAddress(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  const positional = args.filter(a => !a.startsWith('-'))
  if (positional.length < 1) {
    return 'Usage: register-lightning-address <username> [-d <description>]'
  }

  const username = positional[0]
  const description = parseFlag(args, '--description', '-d')

  const result = await sdk.registerLightningAddress({
    username,
    description,
  })
  return formatValue(result)
}

// --- delete-lightning-address ---

async function handleDeleteLightningAddress(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, _args: string[]): Promise<string> {
  await sdk.deleteLightningAddress()
  return 'Lightning address deleted'
}

// --- list-fiat-currencies ---

async function handleListFiatCurrencies(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, _args: string[]): Promise<string> {
  const result = await sdk.listFiatCurrencies()
  return formatValue(result)
}

// --- list-fiat-rates ---

async function handleListFiatRates(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, _args: string[]): Promise<string> {
  const result = await sdk.listFiatRates()
  return formatValue(result)
}

// --- recommended-fees ---

async function handleRecommendedFees(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, _args: string[]): Promise<string> {
  const result = await sdk.recommendedFees()
  return formatValue(result)
}

// --- get-tokens-metadata ---

async function handleGetTokensMetadata(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  if (args.length < 1) {
    return 'Usage: get-tokens-metadata <token_id> [<token_id2> ...]'
  }

  const result = await sdk.getTokensMetadata({ tokenIdentifiers: args })
  return formatValue(result)
}

// --- fetch-conversion-limits ---

async function handleFetchConversionLimits(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  const fromBitcoin = hasFlag(args, '--from-bitcoin', '-f')
  const tokenIdentifier = parseFlag(args, '--token', '--token-identifier')

  // Also accept positional argument for token identifier
  const positional = args.filter(a => !a.startsWith('-'))
  const tokenId = tokenIdentifier ?? (positional.length > 0 ? positional[0] : undefined)

  if (!tokenId) {
    return 'Usage: fetch-conversion-limits <token_id> [--from-bitcoin]\n   or: fetch-conversion-limits --token <token_id> [--from-bitcoin]'
  }

  let conversionType: InstanceType<typeof ConversionType.FromBitcoin> | InstanceType<typeof ConversionType.ToBitcoin>
  let tokenIdentifierParam: string | undefined

  if (fromBitcoin) {
    conversionType = new ConversionType.FromBitcoin()
    tokenIdentifierParam = tokenId
  } else {
    conversionType = new ConversionType.ToBitcoin({
      fromTokenIdentifier: tokenId,
    })
    tokenIdentifierParam = undefined
  }

  const result = await sdk.fetchConversionLimits({
    conversionType,
    tokenIdentifier: tokenIdentifierParam,
  })
  return formatValue(result)
}

// --- get-user-settings ---

async function handleGetUserSettings(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, _args: string[]): Promise<string> {
  const result = await sdk.getUserSettings()
  return formatValue(result)
}

// --- set-user-settings ---

async function handleSetUserSettings(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  const privateModeStr = parseFlag(args, '--private', '-p', '--spark-private-mode')
  let sparkPrivateModeEnabled: boolean | undefined
  if (privateModeStr !== undefined) {
    sparkPrivateModeEnabled = privateModeStr === 'true'
  }

  await sdk.updateUserSettings({
    sparkPrivateModeEnabled,
  })
  return 'User settings updated'
}

// --- get-spark-status ---

async function handleGetSparkStatus(_sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, _args: string[]): Promise<string> {
  const result = await getSparkStatus()
  return formatValue(result)
}

// --- issuer (delegation) ---

async function handleIssuer(_sdk: BreezSdkInterface, tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  return dispatchIssuerCommand(args, tokenIssuer)
}

// --- contacts (delegation) ---

async function handleContacts(sdk: BreezSdkInterface, _tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  return dispatchContactsCommand(args, sdk)
}
