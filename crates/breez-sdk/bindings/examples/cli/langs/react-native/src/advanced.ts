/**
 * Advanced subcommands.
 *
 * Mirrors the Rust CLI `advanced` subcommands:
 *   recover-funds, check-recover-funds, export-unilateral-exit-state,
 *   import-unilateral-exit-state
 */

import {
  singleKeyCpfpSigner,
  CooperativeRecoveryError,
  CooperativeRecoveryError_Tags,
  CooperativeRecoveryFailure,
  CpfpFundingKind,
  CpfpInput,
  CpfpInput_Tags,
  ExitLeafSelection,
  ExitTransactionStatus,
  ExitTransactionStatus_Tags,
  RecoverFundsLeaf,
  RecoverFundsResponse,
  RecoveryMethod,
  RecoveryRedoReason,
  RecoveryTransaction,
  RecoveryTxKind,
  RecoveryVerdict_Tags,
  SkippedLeafReason_Tags,
} from '@breeztech/breez-sdk-spark-react-native'
import type {
  BreezSdkInterface,
  CpfpSigner,
  PrepareRecoverFundsRequest,
  PrepareRecoverFundsResponse,
  SkippedLeaf,
  SkippedLeafReason,
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
  'recover-funds',
  'check-recover-funds',
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

/**
 * Dispatch an advanced subcommand.
 *
 * @param args - The arguments after "advanced" (e.g., ["recover-funds", "--fee-rate", "2", ...])
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
    case 'recover-funds':
      return handleRecoverFunds(sdk, subArgs)
    case 'check-recover-funds':
      return handleCheckRecoverFunds(sdk, subArgs)
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
    '  advanced recover-funds --fee-rate <rate> --destination <addr>',
    '    [--funding-kind p2wpkh|p2tr] [--all | --leaf <id> ...]',
    '    [--utxo txid:vout:value:pubkey ...] [--secret-key <hex>]',
    '    [--output-file <path>] [--sign]',
    '                                         Recover the funds that left the balance, or with --all every leaf',
    '  advanced check-recover-funds --input-file <path>',
    '    [--output-file <path>]',
    '                                         Check a recovery against the chain',
    '  advanced export-unilateral-exit-state --output-file <path>',
    '                                         Export exit state to a file',
    '  advanced import-unilateral-exit-state --input-file <path>',
    '                                         Import exit state from a file',
    '',
  ]
  return lines.join('\n')
}

// --- recover-funds ---

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
  const valueSats = BigInt(parts[2])
  const pubkey = parts[3]

  if (kind === 'p2wpkh') {
    return new CpfpInput.P2wpkh({ txid, vout, valueSats, pubkey })
  }
  return new CpfpInput.P2tr({ txid, vout, valueSats, pubkey })
}

function recoverySelection(all: boolean, leafIds: string[]): ExitLeafSelection {
  if (all) {
    return new ExitLeafSelection.All()
  }
  if (leafIds.length === 0) {
    return new ExitLeafSelection.RecoverableOnly()
  }
  return new ExitLeafSelection.Specific({ leafIds })
}

function formatQuote(prepared: PrepareRecoverFundsResponse): string[] {
  const cooperative = prepared.leaves.filter(
    (leaf: RecoverFundsLeaf) => leaf.method === RecoveryMethod.Cooperative
  ).length
  const lines = [
    formatValue(prepared),
    `${prepared.leaves.length} leaf(s), ${cooperative} cooperative and ` +
    `${prepared.leaves.length - cooperative} unilateral: ` +
    `recovering ${prepared.recoverableValueSats} sats for ${prepared.totalFeeSats} sats in fees`,
  ]
  lines.push(...formatSkipped(prepared.skipped))
  return lines
}

function formatSkipped(skipped: SkippedLeaf[]): string[] {
  const lines: string[] = []
  for (const leaf of skipped) {
    let reason: string
    if (leaf.reason.tag === SkippedLeafReason_Tags.FeeExceedsValue) {
      reason = 'recovering it costs too much at this fee rate'
    } else if (leaf.reason.tag === SkippedLeafReason_Tags.FundsNotFound) {
      reason = 'its funds were not found on-chain'
    } else if (leaf.reason.tag === SkippedLeafReason_Tags.Unverified) {
      reason = 'its funds could not be looked up'
    } else {
      reason = (leaf.reason as SkippedLeafReason).inner?.message ?? 'not recoverable'
    }
    lines.push(`Left out: leaf ${leaf.leafId} (${leaf.valueSats} sats): ${reason}`)
  }
  return lines
}

// Printed the way the Rust CLI prints it.
function formatStatus(status: ExitTransactionStatus): string {
  const height = (h: number | undefined) => (h != null ? `Some(${h})` : 'None')
  if (status.tag === ExitTransactionStatus_Tags.Confirmed) {
    return `Confirmed { block_height: ${height(status.inner.blockHeight)} }`
  }
  if (status.tag === ExitTransactionStatus_Tags.WaitingForTimelock) {
    return `WaitingForTimelock { spendable_at_height: ${height(status.inner.spendableAtHeight)} }`
  }
  return status.tag
}

function formatCooperativeRecoveryError(error: CooperativeRecoveryError): string {
  if (error.tag === CooperativeRecoveryError_Tags.ReplacementFeeTooLow) {
    return (
      'A recovery of this output is already on the network: replacing it takes at least ' +
      `${error.inner.requiredFeeSats} sats or ${error.inner.requiredFeeRateSatPerVbyte} sats/vbyte`
    )
  }
  if (error.tag === CooperativeRecoveryError_Tags.OperatorsUnavailable) {
    return `Operators unavailable: ${error.inner.message}`
  }
  return `Generic error: ${error.inner.message}`
}

function formatRecovery(response: RecoverFundsResponse): string[] {
  const lines: string[] = []
  lines.push(
    `Recoverable ${response.recoverableValueSats} sats, ` +
    `total fee ${response.totalFeeSats} sats ` +
    `(cooperative ${response.cooperativeFeeSats}, cpfp ${response.cpfpFeeSats}, ` +
    `fanout ${response.fanoutFeeSats}, sweep ${response.sweepFeeSats}), ` +
    `${response.transactions.length} transaction(s):`
  )

  for (let i = 0; i < response.transactions.length; i++) {
    const tx = response.transactions[i]
    const node = tx.nodeId != null ? ` node=${tx.nodeId}` : ''
    const after = tx.dependsOn.length > 0
      ? `, after ${tx.dependsOn.join(',')}`
      : ''
    const csv = tx.csvTimelockBlocks != null
      ? `, csv ${tx.csvTimelockBlocks} blocks`
      : ''
    lines.push(
      `  [${i}] ${RecoveryTxKind[tx.kind]}${node} status=${formatStatus(tx.status)} ` +
      `txid=${tx.txid}${after}${csv}`
    )

    if (tx.status.tag === ExitTransactionStatus_Tags.Confirmed) {
      const blockHeight = tx.status.inner.blockHeight
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
      const spendableAtHeight = tx.status.inner.spendableAtHeight
      if (spendableAtHeight != null) {
        lines.push(`      (waiting for its timelock, until block ${spendableAtHeight})`)
      } else {
        lines.push('      (waiting for its timelock)')
      }
    }

    const pkg = tx.cpfpTxHex != null
      ? `${tx.txHex},${tx.cpfpTxHex}`
      : tx.txHex
    lines.push(`      Package: ${pkg}`)
  }

  if (response.failed.length > 0) {
    lines.push(`Not recovered, ${response.failed.length} leaf(s):`)
  }
  for (const failure of response.failed) {
    lines.push(
      `  leaf ${failure.leafId} ` +
      `(output ${failure.outputTxid}:${failure.outputVout}): ` +
      formatCooperativeRecoveryError(failure.error)
    )
  }

  return lines
}

async function writeRecoveryFile(path: string, recovery: RecoverFundsResponse): Promise<void> {
  const json = JSON.stringify(
    recovery,
    (_key, value) => (typeof value === 'bigint' ? Number(value) : value),
    2
  )
  const temporary = `${path}.tmp`
  await RNFS.writeFile(temporary, json, 'utf8')
  // RNFS.moveFile fails on iOS when the destination exists.
  if (await RNFS.exists(path)) {
    await RNFS.unlink(path)
  }
  await RNFS.moveFile(temporary, path)
}

async function handleRecoverFunds(sdk: BreezSdkInterface, args: string[]): Promise<string> {
  const feeRateStr = parseFlag(args, '--fee-rate')
  const destination = parseFlag(args, '--destination')

  if (!feeRateStr || !destination) {
    return 'Usage: advanced recover-funds --fee-rate <rate> --destination <addr> [--funding-kind p2wpkh|p2tr] [--all | --leaf <id> ...] [--utxo txid:vout:value:pubkey ...] [--secret-key <hex>] [--output-file <path>] [--sign]'
  }

  const fundingKindStr = parseFlag(args, '--funding-kind') ?? 'p2tr'
  if (fundingKindStr !== 'p2wpkh' && fundingKindStr !== 'p2tr') {
    return 'Error: --funding-kind must be p2wpkh or p2tr'
  }
  const all = args.includes('--all')
  const leafIds = parseRepeatedFlag(args, '--leaf')
  if (all && leafIds.length > 0) {
    return 'Error: --all and --leaf are mutually exclusive'
  }
  const utxoArgs = parseRepeatedFlag(args, '--utxo')
  const secretKey = parseFlag(args, '--secret-key')
  if (utxoArgs.length > 0 && !secretKey) {
    return 'Error: --secret-key is required when --utxo is provided'
  }
  const outputFile = parseFlag(args, '--output-file')
  const sign = args.includes('--sign')

  const request: PrepareRecoverFundsRequest = {
    feeRateSatPerVbyte: BigInt(feeRateStr),
    fundingKind: fundingKindStr === 'p2wpkh'
      ? new CpfpFundingKind.P2wpkh()
      : new CpfpFundingKind.P2tr(),
    destination,
    selection: recoverySelection(all, leafIds),
  }

  let prepared = await sdk.prepareRecoverFunds(request)
  if (prepared.leaves.length === 0) {
    const lines = ['Nothing to recover.']
    lines.push(...formatSkipped(prepared.skipped))
    return lines.join('\n')
  }
  const lines = formatQuote(prepared)
  if (!outputFile) {
    lines.push('Without --output-file the recovery is only printed: check-recover-funds cannot read it back.')
  }

  let fundingInputs: CpfpInput[] = []
  let signer: CpfpSigner | undefined
  if (prepared.funding != null) {
    if (utxoArgs.length === 0) {
      lines.push(
        `Provide --utxo txid:vout:value:pubkey (at least ${prepared.funding.singleUtxoSats} sats; ` +
        'for P2TR the internal key) and --secret-key <hex> to include the unilateral exit.'
      )
      const cooperative = prepared.leaves
        .filter((leaf: RecoverFundsLeaf) => leaf.method === RecoveryMethod.Cooperative)
        .map((leaf: RecoverFundsLeaf) => leaf.leafId)
      if (cooperative.length === 0) {
        lines.push('Nothing to recover without funding.')
        return lines.join('\n')
      }
      lines.push('Recovering only the cooperative leaves:')
      prepared = await sdk.prepareRecoverFunds({
        ...request,
        selection: new ExitLeafSelection.Specific({ leafIds: cooperative }),
      })
      lines.push(...formatQuote(prepared))
    } else {
      fundingInputs = utxoArgs.map(u => parseCpfpInput(u, fundingKindStr))
      signer = singleKeyCpfpSigner(hexToArrayBuffer(secretKey!.trim()))
    }
  }

  if (!sign) {
    lines.push('Add --sign to sign this recovery.')
    return lines.join('\n')
  }
  const response = await sdk.recoverFunds({ prepared, fundingInputs }, signer)
  lines.push(...formatRecovery(response))

  if (outputFile) {
    const outputPath = resolvePath(outputFile)
    await writeRecoveryFile(outputPath, response)
    lines.push(`Wrote the recovery to ${outputPath}`)
    lines.push(
      'Next: broadcast the Ready packages. After new blocks, run check-recover-funds ' +
      `--input-file ${outputPath} to see what is ready next.`
    )
  } else {
    lines.push('Next: broadcast the Ready packages.')
  }

  return lines.join('\n')
}

// --- check-recover-funds ---

function decodeCooperativeRecoveryError(raw: any): CooperativeRecoveryError {
  switch (raw.tag) {
    case CooperativeRecoveryError_Tags.ReplacementFeeTooLow:
      return new CooperativeRecoveryError.ReplacementFeeTooLow({
        requiredFeeSats: BigInt(raw.inner.requiredFeeSats),
        requiredFeeRateSatPerVbyte: BigInt(raw.inner.requiredFeeRateSatPerVbyte),
      })
    case CooperativeRecoveryError_Tags.OperatorsUnavailable:
      return new CooperativeRecoveryError.OperatorsUnavailable({ message: raw.inner.message })
    case CooperativeRecoveryError_Tags.Generic:
      return new CooperativeRecoveryError.Generic({ message: raw.inner.message })
    default:
      throw new Error(`Unknown cooperative recovery error '${raw.tag}'`)
  }
}

function decodeExitTransactionStatus(raw: any): ExitTransactionStatus {
  switch (raw.tag) {
    case ExitTransactionStatus_Tags.Confirmed:
      return new ExitTransactionStatus.Confirmed({ blockHeight: raw.inner.blockHeight })
    case ExitTransactionStatus_Tags.Ready:
      return new ExitTransactionStatus.Ready()
    case ExitTransactionStatus_Tags.WaitingForDependencies:
      return new ExitTransactionStatus.WaitingForDependencies()
    case ExitTransactionStatus_Tags.WaitingForTimelock:
      return new ExitTransactionStatus.WaitingForTimelock({
        spendableAtHeight: raw.inner.spendableAtHeight,
      })
    case ExitTransactionStatus_Tags.Unverified:
      return new ExitTransactionStatus.Unverified()
    default:
      throw new Error(`Unknown transaction status '${raw.tag}'`)
  }
}

function decodeCpfpInput(raw: any): CpfpInput {
  const { txid, vout } = raw.inner
  const valueSats = BigInt(raw.inner.valueSats)
  switch (raw.tag) {
    case CpfpInput_Tags.P2wpkh:
      return new CpfpInput.P2wpkh({ txid, vout, valueSats, pubkey: raw.inner.pubkey })
    case CpfpInput_Tags.P2tr:
      return new CpfpInput.P2tr({ txid, vout, valueSats, pubkey: raw.inner.pubkey })
    case CpfpInput_Tags.Custom:
      return new CpfpInput.Custom({
        txid,
        vout,
        valueSats,
        scriptPubkeyHex: raw.inner.scriptPubkeyHex,
        signedInputWeight: BigInt(raw.inner.signedInputWeight),
      })
    default:
      throw new Error(`Unknown funding input '${raw.tag}'`)
  }
}

// writeRecoveryFile stores u64 fields as numbers, which the SDK rejects, so each
// field is rebuilt as its binding type.
async function readRecoveryFile(path: string): Promise<RecoverFundsResponse> {
  const raw = JSON.parse(await RNFS.readFile(path, 'utf8'))
  return RecoverFundsResponse.create({
    recoverableValueSats: BigInt(raw.recoverableValueSats),
    totalFeeSats: BigInt(raw.totalFeeSats),
    cooperativeFeeSats: BigInt(raw.cooperativeFeeSats),
    cpfpFeeSats: BigInt(raw.cpfpFeeSats),
    fanoutFeeSats: BigInt(raw.fanoutFeeSats),
    sweepFeeSats: BigInt(raw.sweepFeeSats),
    leaves: raw.leaves.map((leaf: any) => RecoverFundsLeaf.create({
      leafId: leaf.leafId,
      valueSats: BigInt(leaf.valueSats),
      method: leaf.method,
    })),
    failed: raw.failed.map((failure: any) => CooperativeRecoveryFailure.create({
      leafId: failure.leafId,
      outputTxid: failure.outputTxid,
      outputVout: failure.outputVout,
      error: decodeCooperativeRecoveryError(failure.error),
    })),
    transactions: raw.transactions.map((tx: any) => RecoveryTransaction.create({
      kind: tx.kind,
      nodeId: tx.nodeId,
      txid: tx.txid,
      txHex: tx.txHex,
      cpfpTxHex: tx.cpfpTxHex,
      csvTimelockBlocks: tx.csvTimelockBlocks,
      dependsOn: tx.dependsOn,
      status: decodeExitTransactionStatus(tx.status),
    })),
    fundingInputs: raw.fundingInputs.map(decodeCpfpInput),
    feeRateSatPerVbyte: BigInt(raw.feeRateSatPerVbyte),
    destination: raw.destination,
  })
}

async function handleCheckRecoverFunds(sdk: BreezSdkInterface, args: string[]): Promise<string> {
  const inputFile = parseFlag(args, '--input-file')
  if (!inputFile) {
    return 'Usage: advanced check-recover-funds --input-file <path> [--output-file <path>]'
  }
  const outputFile = parseFlag(args, '--output-file')

  const inputPath = resolvePath(inputFile)
  const recovery = await readRecoveryFile(inputPath)

  const checked = await sdk.checkRecoverFunds({ recovery })

  const lines: string[] = []
  if (checked.verdict.tag === RecoveryVerdict_Tags.Redo) {
    lines.push(`Verdict: Redo { reason: ${RecoveryRedoReason[checked.verdict.inner.reason]} }`)
    lines.push(`  (this recovery cannot finish: run ${redoCommand(checked.recovery)})`)
  } else {
    lines.push(`Verdict: ${checked.verdict.tag}`)
  }
  lines.push(...formatRecovery(checked.recovery))

  const outputPath = resolvePath(outputFile ?? inputFile)
  await writeRecoveryFile(outputPath, checked.recovery)
  lines.push(`Wrote the recovery to ${outputPath}`)

  return lines.join('\n')
}

function redoCommand(recovery: RecoverFundsResponse): string {
  let command =
    `recover-funds --fee-rate ${recovery.feeRateSatPerVbyte} --destination ${recovery.destination}`
  for (const leaf of recovery.leaves) {
    command += ` --leaf ${leaf.leafId}`
  }
  return command
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
