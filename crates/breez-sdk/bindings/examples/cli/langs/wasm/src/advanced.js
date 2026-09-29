'use strict'

const fs = require('fs')
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
  return { type: kind, txid, vout, valueSats: value, pubkey }
}

/**
 * Print each recovery transaction with a copy-pasteable Package line.
 *
 * @param {object} response - The RecoverFundsResponse
 */
function printRecovery(response) {
  console.log(
    `Recoverable ${response.recoverableValueSats} sats, ` +
    `total fee ${response.totalFeeSats} sats ` +
    `(cooperative ${response.cooperativeFeeSats}, cpfp ${response.cpfpFeeSats}, ` +
    `fanout ${response.fanoutFeeSats}, sweep ${response.sweepFeeSats}), ` +
    `${response.transactions.length} transaction(s):`
  )
  for (let i = 0; i < response.transactions.length; i++) {
    const tx = response.transactions[i]
    const after = tx.dependsOn.length > 0
      ? `, after ${tx.dependsOn.join(',')}`
      : ''
    const csv = tx.csvTimelockBlocks != null
      ? `, csv ${tx.csvTimelockBlocks} blocks`
      : ''
    const node = tx.nodeId != null
      ? ` node=${tx.nodeId}`
      : ''
    console.log(`  [${i}] ${tx.kind}${node} status=${JSON.stringify(tx.status)} txid=${tx.txid}${after}${csv}`)
    if (tx.status.type === 'confirmed') {
      if (tx.status.blockHeight != null) {
        console.log(`      (confirmed in block ${tx.status.blockHeight}, nothing to broadcast)`)
      } else {
        console.log('      (already confirmed, nothing to broadcast)')
      }
      continue
    }
    if (tx.status.type === 'waitingForDependencies') {
      console.log('      (waiting on the transactions it depends on)')
    }
    if (tx.status.type === 'waitingForTimelock') {
      if (tx.status.spendableAtHeight != null) {
        console.log(`      (waiting for its timelock, until block ${tx.status.spendableAtHeight})`)
      } else {
        console.log('      (waiting for its timelock)')
      }
    }
    const pkg = tx.cpfpTxHex != null
      ? `${tx.txHex},${tx.cpfpTxHex}`
      : tx.txHex
    console.log(`      Package: ${pkg}`)
  }
  if (response.failed.length > 0) {
    console.log(`Not recovered, ${response.failed.length} leaf(s):`)
  }
  for (const failure of response.failed) {
    console.log(
      `  leaf ${failure.leafId} ` +
      `(output ${failure.outputTxid}:${failure.outputVout}): ` +
      cooperativeRecoveryErrorMessage(failure.error)
    )
  }
}

function cooperativeRecoveryErrorMessage(error) {
  switch (error.type) {
    case 'replacementFeeTooLow':
      return 'A recovery of this output is already on the network: replacing it takes at least ' +
        `${error.requiredFeeSats} sats or ${error.requiredFeeRateSatPerVbyte} sats/vbyte`
    case 'operatorsUnavailable':
      return `Operators unavailable: ${error.message}`
    case 'generic':
      return `Generic error: ${error.message}`
    default:
      return JSON.stringify(error)
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

  // --- recover-funds ---
  advanced
    .command('recover-funds')
    .description('Recover the funds that left the balance, or with --all every leaf. Quotes the recovery (which leaves, how each is recovered, the fees, how much to fund), asks for funding UTXOs and their key when a unilateral exit needs them, and signs it once you confirm. A cooperative recovery needs the operators online')
    .requiredOption('--fee-rate <rate>', 'Target fee rate in sat/vByte', parseInt)
    .option('--funding-kind <kind>', 'Funding UTXO kind (p2wpkh or p2tr)', 'p2tr')
    .requiredOption('--destination <address>', 'Destination address for the recovered funds')
    .option('--all', 'Recover every leaf worth it, including the ones still in the balance. Only for when the operators are unreachable or refuse to serve the wallet')
    .option('--leaf <ids...>', 'Leaf id(s) to recover (omit to recover the leaves that left the balance)')
    .option('--output-file <path>', 'File to write the signed recovery to, for check-recover-funds to read back')
    .action(async (options) => {
      const sdk = getSdk()
      const leafIds = options.leaf || []
      if (options.all && leafIds.length > 0) {
        throw new Error('Cannot specify both --all and --leaf')
      }
      const request = {
        feeRateSatPerVbyte: options.feeRate,
        fundingKind: { type: options.fundingKind },
        destination: options.destination,
        selection: recoverySelection(options.all, leafIds)
      }
      await recoverFunds(rl, sdk, request, options.fundingKind, options.outputFile)
    })

  // --- check-recover-funds ---
  advanced
    .command('check-recover-funds')
    .description('Read a recovery written by recover-funds back against the chain: which of its transactions confirmed, which are ready to broadcast now, and whether it can still finish')
    .requiredOption('--input-file <path>', 'File the recovery was written to')
    .option('--output-file <path>', 'File to write the updated recovery to (defaults to --input-file)')
    .action(async (options) => {
      await checkRecoverFunds(getSdk(), options.inputFile, options.outputFile)
    })

  // --- export-unilateral-exit-state ---
  advanced
    .command('export-unilateral-exit-state')
    .description('Export the wallet\'s unilateral exit state to a file, for safekeeping outside the wallet\'s own storage')
    .requiredOption('--output-file <path>', 'File to write the exit state to')
    .action(async (options) => {
      const sdk = getSdk()
      const exported = await sdk.exportUnilateralExitState()
      fs.writeFileSync(options.outputFile, exported.exitState)
      console.log(`Wrote ${exported.exitState.length} bytes to ${options.outputFile}`)
    })

  // --- import-unilateral-exit-state ---
  advanced
    .command('import-unilateral-exit-state')
    .description('Import a unilateral exit state previously written by export-unilateral-exit-state, merging it into the wallet')
    .requiredOption('--input-file <path>', 'File the exit state was exported to')
    .action(async (options) => {
      const sdk = getSdk()
      const exitState = fs.readFileSync(options.inputFile, 'utf-8')
      const imported = await sdk.importUnilateralExitState({ exitState })
      console.log(
        `Imported ${imported.importedLeaves} leaf(s), ` +
        `skipped ${imported.skippedForeignLeaves} leaf(s) from a different wallet ` +
        `and ${imported.skippedConflictingLeaves} that disagree with what this wallet holds, ` +
        `left out the exit data of ${imported.skippedChains} leaf(s)`
      )
    })
}

/**
 * Quote the recovery, ask for funding when a unilateral exit needs it, and
 * sign it once you confirm. Without funding, only the cooperative leaves are
 * recovered.
 */
async function recoverFunds(rl, sdk, request, fundingKind, outputFile) {
  let prepared = await sdk.prepareRecoverFunds(request)
  if (prepared.leaves.length === 0) {
    console.log(
      'Nothing to recover: each selected leaf is finished, not worth recovering at this fee ' +
      'rate, or its funds were not found.'
    )
    return
  }
  printQuote(prepared)
  if (!outputFile) {
    console.log('Without --output-file the recovery is only printed: check-recover-funds cannot read it back.')
  }

  let fundingInputs = []
  let signer
  if (prepared.funding != null) {
    const utxoLine = await question(
      rl,
      `Funding UTXO(s) of at least ${prepared.funding.singleUtxoSats} sats, as txid:vout:value:pubkey ` +
      '(space-separated; for P2TR the internal key; blank to skip the unilateral exit): '
    )
    if (utxoLine.trim() === '') {
      const cooperative = prepared.leaves
        .filter((leaf) => leaf.method === 'cooperative')
        .map((leaf) => leaf.leafId)
      if (cooperative.length === 0) {
        console.log('Nothing to recover without funding.')
        return
      }
      console.log('Recovering only the cooperative leaves:')
      prepared = await sdk.prepareRecoverFunds({
        ...request,
        selection: { type: 'specific', leafIds: cooperative }
      })
      printQuote(prepared)
    } else {
      fundingInputs = utxoLine.trim().split(/\s+/).map(
        (u) => parseCpfpInput(u, fundingKind)
      )
      const keyLine = await question(rl, 'Hex secret key for the funding UTXO(s): ')
      signer = singleKeyCpfpSigner(Buffer.from(keyLine.trim(), 'hex'))
    }
  }

  const answer = await questionWithDefault(rl, 'Sign this recovery? (y/n): ', 'y')
  if (answer.toLowerCase() !== 'y') {
    return
  }
  const response = await sdk.recoverFunds({ prepared, fundingInputs }, signer)
  printRecovery(response)
  if (outputFile) {
    writeRecovery(outputFile, response)
    console.log(
      'Next: broadcast the Ready packages. After new blocks, run check-recover-funds ' +
      `--input-file ${outputFile} to see what is ready next.`
    )
  } else {
    console.log('Next: broadcast the Ready packages.')
  }
}

function printQuote(prepared) {
  printValue(prepared)
  const cooperative = prepared.leaves.filter((leaf) => leaf.method === 'cooperative').length
  console.log(
    `${prepared.leaves.length} leaf(s), ${cooperative} cooperative and ` +
    `${prepared.leaves.length - cooperative} unilateral: ` +
    `recovering ${prepared.recoverableValueSats} sats for ${prepared.totalFeeSats} sats in fees`
  )
}

async function checkRecoverFunds(sdk, inputFile, outputFile) {
  const recovery = readRecovery(inputFile)
  const checked = await sdk.checkRecoverFunds({ recovery })
  console.log(`Verdict: ${JSON.stringify(checked.verdict)}`)
  if (checked.verdict.type === 'redo') {
    console.log(`  (this recovery cannot finish: run ${redoCommand(checked.recovery)})`)
  }
  printRecovery(checked.recovery)
  writeRecovery(outputFile || inputFile, checked.recovery)
}

function redoCommand(recovery) {
  let command =
    `recover-funds --fee-rate ${recovery.feeRateSatPerVbyte} --destination ${recovery.destination}`
  for (const leaf of recovery.leaves) {
    command += ` --leaf ${leaf.leafId}`
  }
  return command
}

function readRecovery(filePath) {
  return JSON.parse(fs.readFileSync(filePath, 'utf-8'))
}

/**
 * Write through a temporary file, so an interrupted write leaves the previous
 * recovery intact.
 */
function writeRecovery(filePath, recovery) {
  const temporary = `${filePath}.tmp`
  fs.writeFileSync(temporary, JSON.stringify(recovery, null, 2))
  fs.renameSync(temporary, filePath)
  console.log(`Wrote the recovery to ${filePath}`)
}

/**
 * --all, the named leaves, or else the leaves that left the balance.
 */
function recoverySelection(all, leafIds) {
  if (all) {
    return { type: 'all' }
  }
  if (leafIds.length === 0) {
    return { type: 'recoverableOnly' }
  }
  return { type: 'specific', leafIds }
}

module.exports = { registerAdvancedCommands }
