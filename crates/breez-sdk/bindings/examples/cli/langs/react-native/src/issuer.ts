/**
 * Token issuer subcommands.
 *
 * Mirrors the Rust CLI `issuer` subcommands:
 *   token-balance, token-metadata, create-token, mint-token,
 *   burn-token, freeze-token, unfreeze-token
 */

import type { TokenIssuerInterface } from '@breeztech/breez-sdk-spark-react-native'
import { formatValue } from './serialization'

/** All issuer subcommand names for help and completion. */
export const ISSUER_COMMAND_NAMES = [
  'token-balance',
  'token-metadata',
  'create-token',
  'mint-token',
  'burn-token',
  'freeze-token',
  'unfreeze-token',
]

/**
 * Parse a named flag from an argument array.
 * Returns the value after the flag, or undefined if not found.
 */
function parseFlag(args: string[], flag: string): string | undefined {
  const idx = args.indexOf(flag)
  if (idx !== -1 && idx + 1 < args.length) {
    return args[idx + 1]
  }
  return undefined
}

/**
 * Check if a boolean flag is present in the argument array.
 */
function hasFlag(args: string[], flag: string): boolean {
  return args.includes(flag)
}

/**
 * Dispatch an issuer subcommand.
 *
 * @param args - The arguments after "issuer" (e.g., ["create-token", "--name", "MyToken", ...])
 * @param tokenIssuer - The TokenIssuerInterface instance from the SDK
 * @returns A string result to display
 */
export async function dispatchIssuerCommand(
  args: string[],
  tokenIssuer: TokenIssuerInterface
): Promise<string> {
  if (args.length === 0 || args[0] === 'help') {
    return printIssuerHelp()
  }

  const subcommand = args[0]
  const subArgs = args.slice(1)

  switch (subcommand) {
    case 'token-balance':
      return handleTokenBalance(tokenIssuer)
    case 'token-metadata':
      return handleTokenMetadata(tokenIssuer)
    case 'create-token':
      return handleCreateToken(tokenIssuer, subArgs)
    case 'mint-token':
      return handleMintToken(tokenIssuer, subArgs)
    case 'burn-token':
      return handleBurnToken(tokenIssuer, subArgs)
    case 'freeze-token':
      return handleFreezeToken(tokenIssuer, subArgs)
    case 'unfreeze-token':
      return handleUnfreezeToken(tokenIssuer, subArgs)
    default:
      return `Unknown issuer subcommand: ${subcommand}. Use 'issuer help' for available commands.`
  }
}

function printIssuerHelp(): string {
  const lines = [
    '',
    'Issuer subcommands:',
    '  issuer token-balance                   Get issuer token balance',
    '  issuer token-metadata                  Get issuer token metadata',
    '  issuer create-token <name> <ticker> <decimals> <max_supply> [-f]',
    '                                         Create a new issuer token',
    '  issuer mint-token <amount>             Mint supply of the issuer token',
    '  issuer burn-token <amount>             Burn supply of the issuer token',
    '  issuer freeze-token <address>          Freeze tokens at an address',
    '  issuer unfreeze-token <address>        Unfreeze tokens at an address',
    '',
  ]
  return lines.join('\n')
}

// --- token-balance ---

async function handleTokenBalance(tokenIssuer: TokenIssuerInterface): Promise<string> {
  const result = await tokenIssuer.getIssuerTokenBalance()
  return formatValue(result)
}

// --- token-metadata ---

async function handleTokenMetadata(tokenIssuer: TokenIssuerInterface): Promise<string> {
  const result = await tokenIssuer.getIssuerTokenMetadata()
  return formatValue(result)
}

// --- create-token ---

async function handleCreateToken(tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  // Support two forms:
  // 1. Positional: create-token <name> <ticker> <decimals> <max_supply> [-f]
  // 2. Named flags: create-token --name <name> --ticker <ticker> --decimals <decimals> --max-supply <supply> [--freezable]

  let name = parseFlag(args, '--name')
  let ticker = parseFlag(args, '--ticker')
  let decimalsStr = parseFlag(args, '--decimals')
  let maxSupplyStr = parseFlag(args, '--max-supply')
  let isFreezable = hasFlag(args, '--freezable') || hasFlag(args, '-f')

  // Fall back to positional arguments
  if (!name && !ticker) {
    const positional = args.filter(a => !a.startsWith('-'))
    if (positional.length < 4) {
      return 'Usage: issuer create-token <name> <ticker> <decimals> <max_supply> [-f]\n' +
        '   or: issuer create-token --name <name> --ticker <ticker> --decimals <decimals> --max-supply <supply> [--freezable]'
    }
    name = positional[0]
    ticker = positional[1]
    decimalsStr = positional[2]
    maxSupplyStr = positional[3]
  }

  if (!name || !ticker) {
    return 'Usage: issuer create-token --name <name> --ticker <ticker> --decimals <decimals> --max-supply <supply> [--freezable]'
  }

  const decimals = parseInt(decimalsStr ?? '6', 10)
  if (isNaN(decimals)) {
    return `Invalid decimals: ${decimalsStr}`
  }

  const maxSupply = BigInt(maxSupplyStr ?? '0')

  const result = await tokenIssuer.createIssuerToken({
    name,
    ticker,
    decimals,
    isFreezable,
    maxSupply,
  })
  return formatValue(result)
}

// --- mint-token ---

async function handleMintToken(tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  if (args.length < 1) {
    return 'Usage: issuer mint-token <amount>'
  }

  const amount = BigInt(args[0])
  const result = await tokenIssuer.mintIssuerToken({ amount })
  return formatValue(result)
}

// --- burn-token ---

async function handleBurnToken(tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  if (args.length < 1) {
    return 'Usage: issuer burn-token <amount>'
  }

  const amount = BigInt(args[0])
  const result = await tokenIssuer.burnIssuerToken({ amount })
  return formatValue(result)
}

// --- freeze-token ---

async function handleFreezeToken(tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  if (args.length < 1) {
    return 'Usage: issuer freeze-token <address>'
  }

  const result = await tokenIssuer.freezeIssuerToken({ address: args[0] })
  return formatValue(result)
}

// --- unfreeze-token ---

async function handleUnfreezeToken(tokenIssuer: TokenIssuerInterface, args: string[]): Promise<string> {
  if (args.length < 1) {
    return 'Usage: issuer unfreeze-token <address>'
  }

  const result = await tokenIssuer.unfreezeIssuerToken({ address: args[0] })
  return formatValue(result)
}
