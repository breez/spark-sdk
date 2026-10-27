'use strict'

const { Command, Option } = require('commander')
const { printValue } = require('./serialization')

const PULL_RETRY_INTERVAL_MS = 5000
const PULL_RETRY_WINDOW_MS = 10 * 60 * 1000

/**
 * Prompt the user for input via readline.
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
 */
async function questionWithDefault(rl, prompt, defaultVal) {
  const answer = await question(rl, prompt)
  return answer.trim() === '' ? defaultVal : answer.trim()
}

/**
 * Parse a token allowance limit string: a number or 'unlimited'.
 */
function parseTokenAllowanceLimit(s) {
  if (s.toLowerCase() === 'unlimited') {
    return { type: 'unlimited' }
  }
  return { type: 'amount', amount: BigInt(s) }
}

/**
 * Parse a pull receiver string: `amount` (this wallet) or `address:amount`.
 */
function parsePullReceiver(s) {
  const colonIdx = s.indexOf(':')
  if (colonIdx === -1) {
    return { receiverAddress: undefined, amount: BigInt(s) }
  }
  return {
    receiverAddress: s.slice(0, colonIdx),
    amount: BigInt(s.slice(colonIdx + 1))
  }
}

function mayStillSettle(result) {
  if (result.ok) {
    return result.value.status === 'pending'
  }
  const msg = String(result.error)
  return msg.includes('NetworkError') || msg.includes('SparkError')
}

async function pullUntilSettled(sdk, prepareResponse) {
  const deadlineMs = (Number(prepareResponse.expiryTime) * 1000) + PULL_RETRY_WINDOW_MS
  for (;;) {
    let result
    try {
      const response = await sdk.pullPayment({ prepareResponse })
      result = { ok: true, value: response }
    } catch (e) {
      result = { ok: false, error: e }
    }
    const now = Date.now()
    if (!mayStillSettle(result) || now >= deadlineMs) {
      if (result.ok) {
        return result.value
      }
      throw result.error
    }
    if (result.ok) {
      console.log(
        `The pull ${result.value.txHash} is pending, checking again in ${PULL_RETRY_INTERVAL_MS / 1000}s`
      )
    } else {
      console.log(
        `Sending the pull failed (${result.error}), trying again in ${PULL_RETRY_INTERVAL_MS / 1000}s`
      )
    }
    await new Promise((resolve) => setTimeout(resolve, PULL_RETRY_INTERVAL_MS))
  }
}

/**
 * Register all token-allowances subcommands and the top-level pull-payment
 * command on the given commander program.
 *
 * @param {Command} program - The parent commander program
 * @param {() => object} getSdk - Function that returns the SDK instance
 * @param {import('readline').Interface} rl - The readline interface for interactive prompts
 */
function registerTokenAllowanceCommands(program, getSdk, rl) {
  const tokenAllowances = program
    .command('token-allowances')
    .description('Token allowance related commands')

  // --- create ---
  tokenAllowances
    .command('create')
    .description('Grant a spender an allowance over this wallet\'s tokens')
    .argument('<spender_address>', 'Spark address of the spender')
    .argument('<token_identifier>', 'Identifier of the token')
    .argument('<expiry_time>', 'Expiry as a Unix timestamp in seconds')
    .requiredOption('--max-per-payment <limit>', 'Limit on one payment, in token base units, or "unlimited"')
    .requiredOption('--max-total <limit>', 'Limit on all payments together, in token base units, or "unlimited"')
    .option('-r, --recipient <addresses...>', 'Spark address the spender may pay (repeat for each recipient, or omit for any)')
    .action(async (spenderAddress, tokenIdentifier, expiryTime, options) => {
      const sdk = getSdk()
      const response = await sdk.createTokenAllowance({
        spenderAddress,
        tokenIdentifier,
        maxPerPayment: parseTokenAllowanceLimit(options.maxPerPayment),
        maxTotal: parseTokenAllowanceLimit(options.maxTotal),
        expiryTime: BigInt(expiryTime),
        allowedRecipients: options.recipient || []
      })
      printValue(response.allowance)
    })

  // --- revoke ---
  tokenAllowances
    .command('revoke')
    .description('Revoke a token allowance this wallet granted')
    .argument('<allowance_id>', 'Identifier of the allowance')
    .action(async (allowanceId) => {
      const sdk = getSdk()
      await sdk.revokeTokenAllowance({ allowanceId })
      console.log('Token allowance revoked')
    })

  // --- list ---
  tokenAllowances
    .command('list')
    .description('List token allowances this wallet granted (owner) or received (spender)')
    .argument('[role]', 'owner or spender', 'owner')
    .option('--counterparty-address <address>', 'Only allowances with this counterparty Spark address')
    .option('--token-identifier <id>', 'Only allowances for this token')
    .option('--include-inactive [value]', 'Include revoked, expired and exhausted allowances')
    .option('--offset <number>', 'Number of allowances to skip', parseInt)
    .option('--limit <number>', 'Maximum number of allowances to return', parseInt)
    .action(async (role, options) => {
      const sdk = getSdk()
      const includeInactive = options.includeInactive != null
        ? options.includeInactive === 'true' || options.includeInactive === true
        : undefined
      const response = await sdk.listTokenAllowances({
        role,
        counterpartyAddress: options.counterpartyAddress,
        tokenIdentifier: options.tokenIdentifier,
        includeInactive,
        offset: options.offset,
        limit: options.limit
      })
      printValue(response.allowances)
    })

  // --- pull-payment (top-level command) ---
  program
    .command('pull-payment')
    .description('Pull a payment from a wallet that granted this wallet an allowance')
    .argument('<payer_address>', 'Spark address of the payer')
    .argument('<token_identifier>', 'Identifier of the token')
    .requiredOption('-r, --receiver <values...>', 'A receiver as "amount" (this wallet) or "address:amount" (repeat for each receiver)')
    .action(async (payerAddress, tokenIdentifier, options) => {
      const sdk = getSdk()
      const receivers = options.receiver.map(parsePullReceiver)

      const prepareResponse = await sdk.preparePullPayment({
        payerAddress,
        tokenIdentifier,
        receivers
      })

      console.log(
        `Pulling ${prepareResponse.amount} base units of ${prepareResponse.tokenIdentifier} from ${prepareResponse.payerAddress}`
      )
      for (const receiver of prepareResponse.receivers) {
        console.log(`  ${receiver.amount} to ${receiver.receiverAddress || ''}`)
      }
      const answer = await questionWithDefault(rl, 'Do you want to continue (y/n): ', 'y')
      if (answer.toLowerCase() !== 'y') {
        throw new Error('Pull cancelled')
      }

      const response = await pullUntilSettled(sdk, prepareResponse)
      printValue(response)
    })
}

module.exports = { registerTokenAllowanceCommands }
