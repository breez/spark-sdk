/**
 * Stable balance subcommands.
 *
 * Mirrors the Rust CLI `stable-balance` subcommands:
 *   get, set, unset
 */

import { StableBalanceActiveLabel } from '@breeztech/breez-sdk-spark-react-native'
import type { BreezSdkInterface } from '@breeztech/breez-sdk-spark-react-native'
import { formatValue } from './serialization'

/** All stable-balance subcommand names for help and completion. */
export const STABLE_BALANCE_COMMAND_NAMES = [
  'get',
  'set',
  'unset',
]

/**
 * Dispatch a stable-balance subcommand.
 *
 * @param args - The arguments after "stable-balance" (e.g., ["set", "USDB"])
 * @param sdk - The BreezSdkInterface instance
 * @returns A string result to display
 */
export async function dispatchStableBalanceCommand(
  args: string[],
  sdk: BreezSdkInterface
): Promise<string> {
  if (args.length === 0 || args[0] === 'help') {
    return printStableBalanceHelp()
  }

  const subcommand = args[0]
  const subArgs = args.slice(1)

  switch (subcommand) {
    case 'get':
      return handleGet(sdk)
    case 'set':
      return handleSet(sdk, subArgs)
    case 'unset':
      return handleUnset(sdk)
    default:
      return `Unknown stable-balance subcommand: ${subcommand}. Use 'stable-balance help' for available commands.`
  }
}

function printStableBalanceHelp(): string {
  const lines = [
    '',
    'Stable balance subcommands:',
    '  stable-balance get                     Get the stable balance active label',
    '  stable-balance set <label>             Set the stable balance active label',
    '  stable-balance unset                   Unset stable balance',
    '',
  ]
  return lines.join('\n')
}

// --- get ---

async function handleGet(sdk: BreezSdkInterface): Promise<string> {
  const settings = await sdk.getUserSettings()
  return formatValue(settings.stableBalanceActiveLabel)
}

// --- set ---

async function handleSet(sdk: BreezSdkInterface, args: string[]): Promise<string> {
  if (args.length < 1) {
    return 'Usage: stable-balance set <label>'
  }

  const label = args[0]
  await sdk.updateUserSettings({
    sparkPrivateModeEnabled: undefined,
    stableBalanceActiveLabel: new StableBalanceActiveLabel.Set({ label }),
  })
  const settings = await sdk.getUserSettings()
  return formatValue(settings)
}

// --- unset ---

async function handleUnset(sdk: BreezSdkInterface): Promise<string> {
  await sdk.updateUserSettings({
    sparkPrivateModeEnabled: undefined,
    stableBalanceActiveLabel: new StableBalanceActiveLabel.Unset(),
  })
  const settings = await sdk.getUserSettings()
  return formatValue(settings)
}
