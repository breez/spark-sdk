/**
 * Advanced subcommands.
 *
 * Mirrors the Rust CLI `advanced` subcommands:
 *   unilateral-exit, check-unilateral-exit, export-unilateral-exit-state,
 *   import-unilateral-exit-state
 */

import {
  singleKeyCpfpSigner,
  CpfpFundingKind,
  CpfpInput,
  ExitLeafSelection,
  ExitTransactionStatus_Tags,
  UnilateralExitVerdict_Tags,
} from '@breeztech/breez-sdk-spark-react-native'
import type {
  BreezSdkInterface,
} from '@breeztech/breez-sdk-spark-react-native'
import RNFS from 'react-native-fs'
import { formatValue } from './serialization'

function hexToArrayBuffer(hex: string): ArrayBuffer {
  const cleaned = hex.replace(/\s/g, '')
  const bytes = new Uint8Array(cleaned.length / 2)
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(cleaned.substring(i * 2, i * 2 + 2), 16)
  }
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength)
}

/** All advanced subcommand names for help and completion. */
export const ADVANCED_COMMAND_NAMES = [
  'unilateral-exit',
  'check-unilateral-exit',
  'export-unilateral-exit-state',
  'import-unilateral-exit-state',
]

/**
 * Parse a named flag value from args. Returns the value after the flag, or undefined.
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
 * Parse a multi-value flag (comma-separated). Returns undefined if not provided.
 */
function parseMultiFlag(args: string[], ...flags: string[]): string[] | undefined {
  const val = parseFlag(args, ...flags)
  if (val === undefined) return undefined
  return val.split(',').map(s => s.trim()).filter(s => s.length > 0)
}

/**
 * Parse repeated flags (each occurrence has its own value).
 */
function parseRepeatedFlag(args: string[], ...flags: string[]): string[] {
  const values: string[] = []
  for (let i = 0; i < args.length; i++) {
    if (flags.includes(args[i]) && i + 1 < args.length) {
      values.push(args[i + 1])
      i++
    }
  }
  return values
}

function resolvePath(path: string): string {
  return path.startsWith('/')
    ? path
    : `${RNFS.DocumentDirectoryPath}/${path}`
}

async function readExitFile(path: string): Promise<any> {
  const json = await RNFS.readFile(path, 'utf8')
  return JSON.parse(json)
}

async function writeExitFile(path: string, exit: unknown): Promise<void> {
  const json = JSON.stringify(
    exit,
    (_key, value) => (typeof value === 'bigint' ? Number(value) : value),
    2
  )
  await RNFS.writeFile(path, json, 'utf8')
}

/**
 * Dispatch an advanced subcommand.
 *
 * @param args - The arguments after "advanced" (e.g., ["unilateral-exit", "--fee-rate", "2", ...])
 * @param sdk - The BreezSdkInterface instance
 * @returns A string result to display
 */
export async function dispatchAdvancedCommand(
  args: string[],
  sdk: BreezSdkInterface
): Promise<string> {
  if (args.length === 0 || args[0] === 'help') {
    return printAdvancedHelp()
  }

  const subcommand = args[0]
  const subArgs = args.slice(1)

  switch (subcommand) {
    case 'unilateral-exit':
      return handleUnilateralExit(sdk, subArgs)
    case 'check-unilateral-exit':
      return handleCheckUnilateralExit(sdk, subArgs)
    case 'export-unilateral-exit-state':
      return handleExportUnilateralExitState(sdk, subArgs)
    case 'import-unilateral-exit-state':
      return handleImportUnilateralExitState(sdk, subArgs)
    default:
      return `Unknown advanced subcommand: ${subcommand}. Use 'advanced help' for available commands.`
  }
}

function printAdvancedHelp(): string {
  const lines = [
    '',
    'Advanced subcommands (expert-only, misuse can strand or lose funds):',
    '  advanced unilateral-exit --fee-rate <rate> --destination <addr>',
    '    [--funding-kind p2wpkh|p2tr] [--leaf <id>,<id>,...]',
    '    [--utxo txid:vout:value:pubkey ...] [--secret-key <hex>]',
    '    [--output-file <path>]',
    '                                         Build and sign a unilateral exit',
    '  advanced check-unilateral-exit --input-file <path>',
    '    [--output-file <path>]',
    '                                         Check a signed exit against the chain',
    '  advanced export-unilateral-exit-state --output-file <path>',
    '                                         Export exit state to a file',
    '  advanced import-unilateral-exit-state --input-file <path>',
    '                                         Import exit state from a file',
    '',
  ]
  return lines.join('\n')
}

// --- unilateral-exit ---

function parseCpfpInput(s: string, kind: string): InstanceType<typeof CpfpInput.P2wpkh> | InstanceType<typeof CpfpInput.P2tr> {
  const parts = s.split(':')
  if (parts.length !== 4) {
    throw new Error(`Invalid funding UTXO '${s}', expected txid:vout:value:pubkey`)
  }
  const txid = parts[0]
  const vout = parseInt(parts[1], 10)
  if (isNaN(vout)) {
    throw new Error(`Invalid vout in '${s}'`)
  }
  const value = BigInt(parts[2])
  const pubkey = parts[3]

  if (kind === 'p2wpkh') {
    return new CpfpInput.P2wpkh({ txid, vout, value, pubkey })
  }
  return new CpfpInput.P2tr({ txid, vout, value, pubkey })
}

function formatExitTransactions(response: any): string[] {
  const lines: string[] = []
  lines.push(
    `Recoverable ${response.recoverableValueSat} sats, ` +
    `total fee ${response.totalFeeSat} sats ` +
    `(cpfp ${response.cpfpFeeSat}, fanout ${response.fanoutFeeSat}, sweep ${response.sweepFeeSat}), ` +
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
    lines.push(`  [${i}] ${tx.kind} status=${tx.status} txid=${tx.txid}${after}${csv}`)

    if (tx.status.tag === ExitTransactionStatus_Tags.Confirmed) {
      const blockHeight = tx.status.inner?.blockHeight
      if (blockHeight != null) {
        lines.push(`      (confirmed in block ${blockHeight}, nothing to broadcast)`)
      } else {
        lines.push('      (already confirmed, nothing to broadcast)')
      }
      continue
    }
    if (tx.status.tag === ExitTransactionStatus_Tags.WaitingForDependencies) {
      lines.push('      (waiting on the transactions it depends on)')
    }
    if (tx.status.tag === ExitTransactionStatus_Tags.WaitingForTimelock) {
      const spendableAtHeight = tx.status.inner?.spendableAtHeight
      if (spendableAtHeight != null) {
        lines.push(`      (waiting for its timelock, until block ${spendableAtHeight})`)
      } else {
        lines.push('      (waiting for its timelock)')
      }
    }

    const pkg = tx.cpfpTxHex
      ? `${tx.txHex},${tx.cpfpTxHex}`
      : tx.txHex
    lines.push(`      Package: ${pkg}`)
  }

  return lines
}

async function handleUnilateralExit(sdk: BreezSdkInterface, args: string[]): Promise<string> {
  const feeRateStr = parseFlag(args, '--fee-rate')
  const destination = parseFlag(args, '--destination')

  if (!feeRateStr || !destination) {
    return 'Usage: advanced unilateral-exit --fee-rate <rate> --destination <addr> [--funding-kind p2wpkh|p2tr] [--leaf <id>,<id>,...] [--utxo txid:vout:value:pubkey ...] [--secret-key <hex>] [--output-file <path>]'
  }

  const feeRate = BigInt(feeRateStr)
  const fundingKindStr = parseFlag(args, '--funding-kind') ?? 'p2tr'
  const leafIds = parseMultiFlag(args, '--leaf')
  const outputFile = parseFlag(args, '--output-file')

  const fundingKind = fundingKindStr === 'p2wpkh'
    ? new CpfpFundingKind.P2wpkh()
    : new CpfpFundingKind.P2tr()

  const selection = leafIds && leafIds.length > 0
    ? new ExitLeafSelection.Specific({ leafIds })
    : new ExitLeafSelection.Auto()

  const prepared = await sdk.prepareUnilateralExit({
    feeRateSatPerVbyte: feeRate,
    fundingKind,
    destination,
    selection,
  })

  const lines: string[] = [formatValue(prepared)]

  if (prepared.leaves.length === 0) {
    lines.push('No leaves to exit.')
    return lines.join('\n')
  }

  const utxoArgs = parseRepeatedFlag(args, '--utxo')
  if (utxoArgs.length === 0) {
    lines.push('No funding provided; showing the quote only.')
    lines.push('Provide --utxo txid:vout:value:pubkey and --secret-key <hex> to sign.')
    return lines.join('\n')
  }

  const secretKey = parseFlag(args, '--secret-key')
  if (!secretKey) {
    return 'Error: --secret-key is required when --utxo is provided'
  }

  const fundingInputs = utxoArgs.map(u => parseCpfpInput(u, fundingKindStr))
  const signer = singleKeyCpfpSigner(hexToArrayBuffer(secretKey.trim()))

  const response = await sdk.unilateralExit(
    { prepared, fundingInputs },
    signer
  )

  lines.push(...formatExitTransactions(response))

  if (outputFile) {
    const outputPath = resolvePath(outputFile)
    await writeExitFile(outputPath, response)
    lines.push(`Wrote the exit to ${outputPath}`)
  }

  return lines.join('\n')
}

// --- check-unilateral-exit ---

async function handleCheckUnilateralExit(sdk: BreezSdkInterface, args: string[]): Promise<string> {
  const inputFile = parseFlag(args, '--input-file')
  if (!inputFile) {
    return 'Usage: advanced check-unilateral-exit --input-file <path> [--output-file <path>]'
  }
  const outputFile = parseFlag(args, '--output-file')

  const inputPath = resolvePath(inputFile)
  const exit = await readExitFile(inputPath)

  const checked = await sdk.checkUnilateralExit({ exit })

  const lines: string[] = []
  lines.push(`Verdict: ${formatValue(checked.verdict)}`)
  if (checked.verdict.tag === UnilateralExitVerdict_Tags.Redo) {
    lines.push('  (this exit cannot be finished, quote and build it again)')
  }
  lines.push(...formatExitTransactions(checked.exit))

  const outputPath = resolvePath(outputFile ?? inputFile)
  await writeExitFile(outputPath, checked.exit)
  lines.push(`Wrote the exit to ${outputPath}`)

  return lines.join('\n')
}

// --- export-unilateral-exit-state ---

async function handleExportUnilateralExitState(sdk: BreezSdkInterface, args: string[]): Promise<string> {
  const outputFile = parseFlag(args, '--output-file')
  if (!outputFile) {
    return 'Usage: advanced export-unilateral-exit-state --output-file <path>'
  }

  const exported = await sdk.exportUnilateralExitState()
  const filePath = resolvePath(outputFile)
  await RNFS.writeFile(filePath, exported.exitState, 'utf8')

  return `Wrote ${exported.exitState.length} bytes to ${filePath}`
}

// --- import-unilateral-exit-state ---

async function handleImportUnilateralExitState(sdk: BreezSdkInterface, args: string[]): Promise<string> {
  const inputFile = parseFlag(args, '--input-file')
  if (!inputFile) {
    return 'Usage: advanced import-unilateral-exit-state --input-file <path>'
  }

  const filePath = resolvePath(inputFile)
  const exitState = await RNFS.readFile(filePath, 'utf8')

  const imported = await sdk.importUnilateralExitState({ exitState })

  return (
    `Imported ${imported.importedLeaves} leaf(s), ` +
    `skipped ${imported.skippedForeignLeaves} leaf(s) from a different wallet ` +
    `and ${imported.skippedConflictingLeaves} that disagree with what this wallet holds, ` +
    `left out the exit data of ${imported.skippedChains} leaf(s)`
  )
}
