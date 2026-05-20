'use strict'

const { Command } = require('commander')
const { printValue } = require('./serialization')

/**
 * Register all stable-balance subcommands on the given commander program.
 *
 * @param {Command} program - The parent commander program (or subcommand)
 * @param {() => object} getSdk - Function that returns the SDK instance
 */
function registerStableBalanceCommands(program, getSdk) {
  const stableBalance = program
    .command('stable-balance')
    .description('Stable balance related commands')

  // --- get ---
  stableBalance
    .command('get')
    .description('Get the stable balance active label')
    .action(async () => {
      const sdk = getSdk()
      const settings = await sdk.getUserSettings()
      printValue(settings.stableBalanceActiveLabel)
    })

  // --- set ---
  stableBalance
    .command('set')
    .description('Set the stable balance active label')
    .argument('<label>', 'The label to activate (e.g. "USDB")')
    .action(async (label) => {
      const sdk = getSdk()
      await sdk.updateUserSettings({
        sparkPrivateModeEnabled: undefined,
        stableBalanceActiveLabel: { type: 'set', label }
      })
      const settings = await sdk.getUserSettings()
      printValue(settings)
    })

  // --- unset ---
  stableBalance
    .command('unset')
    .description('Unset stable balance')
    .action(async () => {
      const sdk = getSdk()
      await sdk.updateUserSettings({
        sparkPrivateModeEnabled: undefined,
        stableBalanceActiveLabel: { type: 'unset' }
      })
      const settings = await sdk.getUserSettings()
      printValue(settings)
    })
}

module.exports = { registerStableBalanceCommands }
