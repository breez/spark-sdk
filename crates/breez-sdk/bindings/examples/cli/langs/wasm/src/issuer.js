'use strict'

const { Command } = require('commander')
const { printValue } = require('./serialization')

/**
 * Register all issuer subcommands on the given commander program.
 *
 * @param {Command} program - The parent commander program (or subcommand)
 * @param {() => object} getTokenIssuer - Function that returns the TokenIssuer instance
 */
function registerIssuerCommands(program, getTokenIssuer) {
  const issuer = program
    .command('issuer')
    .description('Issuer related commands')

  // --- token-balance ---
  issuer
    .command('token-balance')
    .description('Get the issuer token balance')
    .action(async () => {
      const tokenIssuer = getTokenIssuer()
      const response = await tokenIssuer.getIssuerTokenBalance()
      printValue(response)
    })

  // --- token-metadata ---
  issuer
    .command('token-metadata')
    .description('Get the issuer token metadata')
    .action(async () => {
      const tokenIssuer = getTokenIssuer()
      const metadata = await tokenIssuer.getIssuerTokenMetadata()
      printValue(metadata)
    })

  // --- create-token ---
  issuer
    .command('create-token')
    .description('Create a new issuer token')
    .argument('<name>', 'Name of the token')
    .argument('<ticker>', 'Ticker symbol of the token')
    .argument('<decimals>', 'Number of decimal places for the token')
    .argument('<max_supply>', 'Maximum supply of the token')
    .option('-f, --is-freezable', 'Whether the token is freezable', false)
    .action(async (name, ticker, decimals, maxSupply, options) => {
      const tokenIssuer = getTokenIssuer()
      const metadata = await tokenIssuer.createIssuerToken({
        name,
        ticker,
        decimals: parseInt(decimals, 10),
        isFreezable: options.isFreezable,
        maxSupply: BigInt(maxSupply)
      })
      printValue(metadata)
    })

  // --- mint-token ---
  issuer
    .command('mint-token')
    .description('Mint supply of the issuer token')
    .argument('<amount>', 'Amount of the supply to mint')
    .action(async (amount) => {
      const tokenIssuer = getTokenIssuer()
      const payment = await tokenIssuer.mintIssuerToken({
        amount: BigInt(amount)
      })
      printValue(payment)
    })

  // --- burn-token ---
  issuer
    .command('burn-token')
    .description('Burn supply of the issuer token')
    .argument('<amount>', 'Amount of the supply to burn')
    .action(async (amount) => {
      const tokenIssuer = getTokenIssuer()
      const payment = await tokenIssuer.burnIssuerToken({
        amount: BigInt(amount)
      })
      printValue(payment)
    })

  // --- freeze-token ---
  issuer
    .command('freeze-token')
    .description('Freeze issuer tokens held at the specified address')
    .argument('<address>', 'Address holding the tokens to freeze')
    .action(async (address) => {
      const tokenIssuer = getTokenIssuer()
      const response = await tokenIssuer.freezeIssuerToken({
        address
      })
      printValue(response)
    })

  // --- unfreeze-token ---
  issuer
    .command('unfreeze-token')
    .description('Unfreeze issuer tokens held at the specified address')
    .argument('<address>', 'Address holding the tokens to unfreeze')
    .action(async (address) => {
      const tokenIssuer = getTokenIssuer()
      const response = await tokenIssuer.unfreezeIssuerToken({
        address
      })
      printValue(response)
    })
}

module.exports = { registerIssuerCommands }
