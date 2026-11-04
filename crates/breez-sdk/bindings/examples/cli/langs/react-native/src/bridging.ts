/**
 * Bridging subcommands.
 *
 * Mirrors the Rust CLI `bridging` subcommands:
 *   create, from-cash-app, to-cash-app, status
 */

import {
  FeePolicy,
} from '@breeztech/breez-sdk-spark-react-native'
import type {
  BreezSdkInterface,
  BridgingClientInterface,
  BridgeRoutePair,
  BridgeRoutePairSide,
  CreateBridgeResponse,
} from '@breeztech/breez-sdk-spark-react-native'
import { formatValue } from './serialization'

/** All bridging subcommand names for help and completion. */
export const BRIDGING_COMMAND_NAMES = [
  'create',
  'from-cash-app',
  'to-cash-app',
  'status',
]

/**
 * Parse a named flag from an argument array.
 * Returns the value after the flag, or undefined if not found.
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

/** Check if a boolean flag is present in the argument array. */
function hasFlag(args: string[], ...flags: string[]): boolean {
  return flags.some(f => args.includes(f))
}

function parseNumericFlag(args: string[], ...flags: string[]): number | undefined {
  const val = parseFlag(args, ...flags)
  if (val === undefined) return undefined
  const num = parseInt(val, 10)
  if (isNaN(num)) return undefined
  return num
}

/**
 * Collect the arguments that are not a flag or a flag's value.
 */
function positionalArgs(args: string[], booleanFlags: string[] = []): string[] {
  const positional: string[] = []
  for (let i = 0; i < args.length; i++) {
    if (!args[i].startsWith('-')) {
      positional.push(args[i])
    } else if (!booleanFlags.includes(args[i])) {
      i++
    }
  }
  return positional
}

/**
 * Dispatch a bridging subcommand.
 */
export async function dispatchBridgingCommand(
  args: string[],
  sdk: BreezSdkInterface
): Promise<string> {
  if (args.length === 0 || args[0] === 'help') {
    return printBridgingHelp()
  }

  const subcommand = args[0]
  const subArgs = args.slice(1)
  const client = sdk.getBridgingClient()

  switch (subcommand) {
    case 'create':
      return handleCreate(client, subArgs)
    case 'from-cash-app':
      return handleFromCashApp(client, subArgs)
    case 'to-cash-app':
      return handleToCashApp(client, subArgs)
    case 'status':
      return handleStatus(client, subArgs)
    default:
      return `Unknown bridging subcommand: ${subcommand}. Use 'bridging help' for available commands.`
  }
}

function printBridgingHelp(): string {
  const lines = [
    '',
    'Bridging subcommands:',
    '  bridging create <recipient> --amount <n> [--refund-address <addr>] [--fees-included] [--max-slippage-bps <bps>] [--route <n>]',
    '  bridging from-cash-app <recipient> --amount <n> [--fees-included] [--max-slippage-bps <bps>] [--route <n>]',
    '  bridging to-cash-app <recipient> --amount <n> --refund-address <addr> [--fees-included] [--max-slippage-bps <bps>] [--route <n>]',
    '  bridging status <bridge_id> [--status-token <token>]',
    '',
  ]
  return lines.join('\n')
}

// ---------------------------------------------------------------------------
// Route selection
// ---------------------------------------------------------------------------

function sideLabel(side: BridgeRoutePairSide): string {
  const addr = side.contractAddress
  let addrSuffix = ''
  if (addr) {
    addrSuffix = addr.length > 12
      ? ` (${addr.slice(0, 6)}...${addr.slice(-6)})`
      : ` (${addr})`
  }
  return `${side.asset} on ${side.chain}${addrSuffix}`
}

function routeLabel(route: BridgeRoutePair): string {
  return `${sideLabel(route.source)} -> ${sideLabel(route.destination)} [${route.provider ?? 'unknown'}]`
}

function selectBridgeRoute(
  routes: BridgeRoutePair[],
  preferredIndex: number | undefined,
): { route: BridgeRoutePair; message: string } {
  if (routes.length === 0) {
    throw new Error('No bridge routes available')
  }

  if (routes.length === 1) {
    const route = routes[0]
    return { route, message: `Auto-selected route: ${routeLabel(route)}` }
  }

  const routeLines = ['Available bridge routes:']
  for (let i = 0; i < routes.length; i++) {
    routeLines.push(`  ${i + 1}. ${routeLabel(routes[i])}`)
  }

  const idx = (preferredIndex ?? 1) - 1
  if (idx < 0 || idx >= routes.length) {
    throw new Error(`Route selection out of range (1 to ${routes.length}). Use --route <n> to choose.`)
  }

  const route = routes[idx]
  routeLines.push(`Selected route ${idx + 1}. Use --route <n> to choose a different route.`)
  return { route, message: routeLines.join('\n') }
}

function serviceFeeDenomination(asset: string | undefined | null, decimals: number | undefined | null): string {
  if (!asset) return 'sats'
  if (decimals != null) return `${asset} (${decimals} decimals)`
  return asset
}

function printBridgeTracking(response: CreateBridgeResponse): string {
  const tokenPart = response.statusToken
    ? ` --status-token ${response.statusToken}`
    : ''
  return `Track it with: bridging status ${response.bridgeId}${tokenPart}`
}

// ---------------------------------------------------------------------------
// Subcommand handlers
// ---------------------------------------------------------------------------

async function handleCreate(client: BridgingClientInterface, args: string[]): Promise<string> {
  const pos = positionalArgs(args, ['--fees-included'])
  if (pos.length < 1) {
    return 'Usage: bridging create <recipient> --amount <n> [--refund-address <addr>] [--fees-included] [--max-slippage-bps <bps>] [--route <n>]'
  }

  const recipient = pos[0]
  const amountStr = parseFlag(args, '--amount')
  if (!amountStr) {
    return 'Error: --amount is required'
  }
  const amount = BigInt(amountStr)
  const refundAddress = parseFlag(args, '--refund-address')
  const feesIncluded = hasFlag(args, '--fees-included')
  const maxSlippageBps = parseNumericFlag(args, '--max-slippage-bps')
  const routeIndex = parseNumericFlag(args, '--route')

  const { routes } = await client.getRoutes({ recipient })
  const routeResult = selectBridgeRoute(routes, routeIndex)

  const feePolicy = feesIncluded ? FeePolicy.FeesIncluded : undefined

  const response: CreateBridgeResponse = await client.createBridge({
    recipient,
    route: routeResult.route,
    amount,
    refundAddress,
    feePolicy,
    maxSlippageBps,
  })

  const lines = [routeResult.message]
  lines.push('Ask the payer to pay:')
  lines.push(response.paymentRequest)
  lines.push(formatValue(response))
  lines.push(printBridgeTracking(response))
  return lines.join('\n')
}

async function handleFromCashApp(client: BridgingClientInterface, args: string[]): Promise<string> {
  const pos = positionalArgs(args, ['--fees-included'])
  if (pos.length < 1) {
    return 'Usage: bridging from-cash-app <recipient> --amount <n> [--fees-included] [--max-slippage-bps <bps>] [--route <n>]'
  }

  const recipient = pos[0]
  const amountStr = parseFlag(args, '--amount')
  if (!amountStr) {
    return 'Error: --amount is required'
  }
  const amount = BigInt(amountStr)
  const feesIncluded = hasFlag(args, '--fees-included')
  const maxSlippageBps = parseNumericFlag(args, '--max-slippage-bps')
  const routeIndex = parseNumericFlag(args, '--route')

  const { routes } = await client.getRoutes({ recipient })
  const lightningRoutes = routes.filter((r: BridgeRoutePair) => r.source.chain === 'lightning')
  const routeResult = selectBridgeRoute(lightningRoutes, routeIndex)

  const feePolicy = feesIncluded ? FeePolicy.FeesIncluded : undefined

  const response: CreateBridgeResponse = await client.bridgeFromCashApp({
    recipient,
    route: routeResult.route,
    amount,
    refundAddress: undefined,
    feePolicy,
    maxSlippageBps,
  })

  const info = response.info
  const serviceFeeAsset = serviceFeeDenomination(info.serviceFeeAsset, info.serviceFeeAssetDecimals)
  const lines = [routeResult.message]
  lines.push('Open this URL in a browser to complete the purchase:')
  lines.push(response.paymentRequest)
  lines.push(
    `Deposit ~${info.depositAmount} sats; recipient receives ~${info.expectedReceivedAmount} ${info.destinationAsset}, ` +
    `service fee ${info.serviceFeeAmount} ${serviceFeeAsset}, expires ${info.expiresAt}`
  )
  lines.push(printBridgeTracking(response))
  return lines.join('\n')
}

async function handleToCashApp(client: BridgingClientInterface, args: string[]): Promise<string> {
  const pos = positionalArgs(args, ['--fees-included'])
  if (pos.length < 1) {
    return 'Usage: bridging to-cash-app <recipient> --amount <n> --refund-address <addr> [--fees-included] [--max-slippage-bps <bps>] [--route <n>]'
  }

  const recipient = pos[0]
  const amountStr = parseFlag(args, '--amount')
  if (!amountStr) {
    return 'Error: --amount is required'
  }
  const amount = BigInt(amountStr)
  const refundAddress = parseFlag(args, '--refund-address')
  if (!refundAddress) {
    return 'Error: --refund-address is required'
  }
  const feesIncluded = hasFlag(args, '--fees-included')
  const maxSlippageBps = parseNumericFlag(args, '--max-slippage-bps')
  const routeIndex = parseNumericFlag(args, '--route')

  const { routes } = await client.getRoutes({ recipient })
  const routeResult = selectBridgeRoute(routes, routeIndex)

  const feePolicy = feesIncluded ? FeePolicy.FeesIncluded : undefined

  const response: CreateBridgeResponse = await client.bridgeToCashApp({
    recipient,
    route: routeResult.route,
    amount,
    refundAddress,
    feePolicy,
    maxSlippageBps,
  })

  const info = response.info
  const route = routeResult.route
  const serviceFeeAsset = serviceFeeDenomination(info.serviceFeeAsset, info.serviceFeeAssetDecimals)
  const lines = [routeResult.message]
  lines.push('Ask the payer to pay:')
  lines.push(response.paymentRequest)
  lines.push(
    `Deposit ${info.depositAmount} ${route.source.asset} on ${route.source.chain} to ${info.depositAddress}. ` +
    `Recipient receives ~${info.expectedReceivedAmount} sats, service fee ${info.serviceFeeAmount} ${serviceFeeAsset}, ` +
    `expires at ${info.expiresAt}`
  )
  lines.push(printBridgeTracking(response))
  return lines.join('\n')
}

async function handleStatus(client: BridgingClientInterface, args: string[]): Promise<string> {
  const pos = positionalArgs(args)
  if (pos.length < 1) {
    return 'Usage: bridging status <bridge_id> [--status-token <token>]'
  }

  const bridgeId = pos[0]
  const statusToken = parseFlag(args, '--status-token')

  const response = await client.getBridgeStatus({ bridgeId, statusToken })
  return formatValue(response)
}
