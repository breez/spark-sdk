'use strict'

const { Command, Option } = require('commander')
const { singleKeyCpfpSigner } = require('@breeztech/breez-sdk-spark/nodejs')
const { printValue } = require('./serialization')

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
 * Parse a `txid:vout:value:pubkey` funding UTXO string into a CpfpInput
 * of the given kind.
 *
 * @param {string} s - The UTXO string
 * @param {string} kind - The funding kind ('p2wpkh' or 'p2tr')
 * @returns {object} The CpfpInput object
 */
function parseCpfpInput(s, kind) {
  const parts = s.split(':')
  if (parts.length !== 4) {
    throw new Error(`Invalid funding UTXO '${s}', expected txid:vout:value:pubkey`)
  }
  const [txid, voutStr, valueStr, pubkey] = parts
  const vout = parseInt(voutStr, 10)
  const value = parseInt(valueStr, 10)
  if (isNaN(vout) || isNaN(value)) {
    throw new Error(`Invalid funding UTXO '${s}': vout and value must be integers`)
  }
  return { type: kind, txid, vout, value, pubkey }
}

/**
 * Print each exit transaction with a copy-pasteable Package line.
 *
 * @param {object} response - The UnilateralExitResponse
 */
function printExitTransactions(response) {
  console.log(
    `Recoverable ${response.recoverableValueSat} sats, total fee ${response.totalFeeSat} sats, ${response.transactions.length} transaction(s):`
  )
  for (let i = 0; i < response.transactions.length; i++) {
    const tx = response.transactions[i]
    const after = tx.dependsOn && tx.dependsOn.length > 0
      ? `, after ${tx.dependsOn.join(',')}`
      : ''
    const csv = tx.csvTimelockBlocks != null
      ? `, csv ${tx.csvTimelockBlocks} blocks`
      : ''
    console.log(`  [${i}] ${tx.kind} status=${tx.status} txid=${tx.txid}${after}${csv}`)
    if (tx.status === 'confirmed') {
      console.log('      (already confirmed, nothing to broadcast)')
      continue
    }
    const pkg = tx.cpfpTxHex
      ? `${tx.txHex},${tx.cpfpTxHex}`
      : tx.txHex
    console.log(`      Package: ${pkg}`)
  }
}

/**
 * Register all advanced subcommands on the given commander program.
 *
 * @param {Command} program - The parent commander program
 * @param {() => object} getSdk - Function that returns the SDK instance
 * @param {import('readline').Interface} rl - The readline interface for interactive prompts
 */
function registerAdvancedCommands(program, getSdk, rl) {
  const advanced = program
    .command('advanced')
    .description('Expert-only commands that build raw transactions for you to broadcast yourself. Misuse can strand or lose funds.')

  // --- unilateral-exit ---
  advanced
    .command('unilateral-exit')
    .description('Build and sign a unilateral exit')
    .requiredOption('--fee-rate <rate>', 'Target fee rate in sat/vByte', parseInt)
    .option('--funding-kind <kind>', 'Funding UTXO kind (p2wpkh or p2tr)', 'p2tr')
    .requiredOption('--destination <address>', 'Destination address for the swept funds')
    .option('--leaf <ids...>', 'Leaf id(s) to exit (omit to auto-select every profitable leaf)')
    .action(async (options) => {
      const sdk = getSdk()

      const fundingKind = { type: options.fundingKind }
      const leafIds = options.leaf || []
      const selection = leafIds.length > 0
        ? { type: 'specific', leafIds }
        : { type: 'auto' }

      const prepared = await sdk.prepareUnilateralExit({
        feeRateSatPerVbyte: options.feeRate,
        fundingKind,
        destination: options.destination,
        selection
      })
      printValue(prepared)

      if (!prepared.leaves || prepared.leaves.length === 0) {
        console.log('No leaves to exit.')
        return
      }

      const utxoLine = await question(
        rl,
        'Funding UTXO(s) as txid:vout:value:pubkey (space-separated, blank to stop): '
      )
      if (utxoLine.trim() === '') {
        console.log('No funding provided; showing the quote only.')
        return
      }
      const fundingInputs = utxoLine.trim().split(/\s+/).map(
        (u) => parseCpfpInput(u, options.fundingKind)
      )

      const keyLine = await question(rl, 'Hex secret key for the funding UTXO(s): ')
      const secretKeyBytes = Buffer.from(keyLine.trim(), 'hex')
      const signer = singleKeyCpfpSigner(secretKeyBytes)

      const response = await sdk.unilateralExit(
        {
          prepared,
          fundingInputs
        },
        signer
      )
      printExitTransactions(response)
    })
}

module.exports = { registerAdvancedCommands }
