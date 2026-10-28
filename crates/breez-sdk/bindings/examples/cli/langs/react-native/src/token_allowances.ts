/**
 * Token allowance subcommands.
 *
 * Mirrors the Rust CLI `token-allowances` subcommands:
 *   create, revoke, list
 */

import type { BreezSdkInterface } from '@breeztech/breez-sdk-spark-react-native'
import { TokenAllowanceLimit, TokenAllowanceRole } from '@breeztech/breez-sdk-spark-react-native'
import { formatValue } from './serialization'

/** All token-allowances subcommand names for help and completion. */
export const TOKEN_ALLOWANCES_COMMAND_NAMES = [
  'create',
  'revoke',
  'list',
]

function parseFlag(args: string[], ...flags: string[]): string | undefined {
  for (const flag of flags) {
    const idx = args.indexOf(flag)
    if (idx !== -1 && idx + 1 < args.length) {
      return args[idx + 1]
    }
  }
  return undefined
}

function hasFlag(args: string[], ...flags: string[]): boolean {
  return flags.some(f => args.includes(f))
}

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

function parseAllowanceLimit(s: string): TokenAllowanceLimit {
  if (s.toLowerCase() === 'unlimited') {
    return new TokenAllowanceLimit.Unlimited()
  }
  return new TokenAllowanceLimit.Amount({ amount: BigInt(s) })
}

/**
 * Dispatch a token-allowances subcommand.
 */
export async function dispatchTokenAllowancesCommand(
  args: string[],
  sdk: BreezSdkInterface,
): Promise<string> {
  if (args.length === 0 || args[0] === 'help') {
    return printTokenAllowancesHelp()
  }

  const subcommand = args[0]
  const subArgs = args.slice(1)

  switch (subcommand) {
    case 'create':
      return handleCreate(sdk, subArgs)
    case 'revoke':
      return handleRevoke(sdk, subArgs)
    case 'list':
      return handleList(sdk, subArgs)
    default:
      return `Unknown token-allowances subcommand: ${subcommand}. Use 'token-allowances help' for available commands.`
  }
}

function printTokenAllowancesHelp(): string {
  const lines = [
    '',
    'Token allowance subcommands:',
    '  token-allowances create <spender_address> <token_identifier> <expiry_time>',
    '      --max-per-payment <amount|unlimited> --max-total <amount|unlimited>',
    '      [-r <recipient> ...]',
    '                                         Grant a spender an allowance',
    '  token-allowances revoke <allowance_id>  Revoke a token allowance',
    '  token-allowances list [owner|spender]   List token allowances',
    '      [--counterparty-address <addr>] [--token-identifier <id>]',
    '      [--include-inactive] [--offset <n>] [--limit <n>]',
    '',
  ]
  return lines.join('\n')
}

// --- create ---

async function handleCreate(sdk: BreezSdkInterface, args: string[]): Promise<string> {
  const positional = positionalArgs(args, ['--include-inactive'])

  if (positional.length < 3) {
    return 'Usage: token-allowances create <spender_address> <token_identifier> <expiry_time> --max-per-payment <amount|unlimited> --max-total <amount|unlimited> [-r <recipient> ...]'
  }

  const spenderAddress = positional[0]
  const tokenIdentifier = positional[1]
  const expiryTime = BigInt(positional[2])

  const maxPerPaymentStr = parseFlag(args, '--max-per-payment')
  const maxTotalStr = parseFlag(args, '--max-total')

  if (!maxPerPaymentStr || !maxTotalStr) {
    return 'Error: --max-per-payment and --max-total are required'
  }

  const maxPerPayment = parseAllowanceLimit(maxPerPaymentStr)
  const maxTotal = parseAllowanceLimit(maxTotalStr)
  const allowedRecipients = parseRepeatedFlag(args, '-r', '--recipient')

  const response = await sdk.createTokenAllowance({
    spenderAddress,
    tokenIdentifier,
    maxPerPayment,
    maxTotal,
    expiryTime,
    allowedRecipients,
  })
  return formatValue(response.allowance)
}

// --- revoke ---

async function handleRevoke(sdk: BreezSdkInterface, args: string[]): Promise<string> {
  if (args.length < 1) {
    return 'Usage: token-allowances revoke <allowance_id>'
  }

  await sdk.revokeTokenAllowance({ allowanceId: args[0] })
  return 'Token allowance revoked'
}

// --- list ---

async function handleList(sdk: BreezSdkInterface, args: string[]): Promise<string> {
  const positional = positionalArgs(args, ['--include-inactive'])
  const roleStr = positional.length > 0 ? positional[0] : 'owner'

  let role: TokenAllowanceRole
  switch (roleStr.toLowerCase()) {
    case 'owner':
      role = TokenAllowanceRole.Owner
      break
    case 'spender':
      role = TokenAllowanceRole.Spender
      break
    default:
      return `Invalid role: ${roleStr}. Expected 'owner' or 'spender'.`
  }

  const counterpartyAddress = parseFlag(args, '--counterparty-address')
  const tokenIdentifier = parseFlag(args, '--token-identifier')
  const includeInactive = hasFlag(args, '--include-inactive') ? true : undefined

  const offsetStr = parseFlag(args, '--offset')
  const offset = offsetStr !== undefined ? parseInt(offsetStr, 10) : undefined

  const limitStr = parseFlag(args, '--limit')
  const limit = limitStr !== undefined ? parseInt(limitStr, 10) : undefined

  const response = await sdk.listTokenAllowances({
    role,
    counterpartyAddress,
    tokenIdentifier,
    includeInactive,
    offset,
    limit,
  })
  return formatValue(response.allowances)
}
