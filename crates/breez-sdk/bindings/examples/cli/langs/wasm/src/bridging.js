'use strict'

const { printValue } = require('./serialization')

/**
 * All bridging subcommand names, for REPL tab completion.
 */
const BRIDGING_COMMAND_NAMES = [
  'bridging create',
  'bridging from-cash-app',
  'bridging to-cash-app',
  'bridging status'
]

/**
 * Register all bridging subcommands on the given commander program.
 *
 * @param {Command} program - The parent commander program
 * @param {() => object} getSdk - Function that returns the SDK instance
 * @param {object} rl - The readline interface for interactive prompts
 */
function registerBridgingCommands(program, getSdk, rl) {
  const bridging = program
    .command('bridging')
    .description('Bridging commands: orders in which a payer on another chain or on Lightning pays a recipient, with no funds moving through the SDK')

  // --- create ---
  bridging
    .command('create')
    .description('Create a bridge that pays a recipient from another chain or over Lightning. Returns what the payer pays.')
    .argument('<recipient>', 'Who to pay: a Spark address, a Lightning address or Cash App $cashtag, or an EVM/Solana/Tron address')
    .requiredOption('--amount <number>', 'Amount in stablecoin base units')
    .option('--refund-address <address>', 'The payer\'s address on the source chain, refunded if delivery fails')
    .option('--fees-included', 'Deduct fees from the amount instead of adding them on top', false)
    .option('--max-slippage-bps <bps>', 'Maximum slippage in basis points', parseInt)
    .action(async (recipient, options) => {
      const sdk = getSdk()
      const client = sdk.getBridgingClient()
      const { routes } = await client.getRoutes({ recipient })
      const route = await chooseBridgeRoute(rl, routes)
      const feePolicy = options.feesIncluded ? 'feesIncluded' : undefined
      const response = await client.createBridge({
        recipient,
        route,
        amount: BigInt(options.amount),
        refundAddress: options.refundAddress,
        feePolicy,
        maxSlippageBps: options.maxSlippageBps
      })
      console.log('Ask the payer to pay:')
      console.log(response.paymentRequest)
      printValue(response)
      printBridgeTracking(response)
    })

  // --- from-cash-app ---
  bridging
    .command('from-cash-app')
    .description('Bridge from Cash App to USDC/USDT on an external chain. Returns a cash.app link for the payer to open.')
    .argument('<recipient>', 'Recipient address on the destination chain (EVM/Solana/Tron)')
    .requiredOption('--amount <number>', 'Amount in the destination stablecoin\'s base units (6 decimals for USDC/USDT)')
    .option('--refund-address <address>', 'Lightning address, $cashtag or amountless BOLT11 invoice refunded if the payment fails before the swap')
    .option('--fees-included', 'Deduct fees from the amount instead of adding them on top', false)
    .option('--max-slippage-bps <bps>', 'Maximum slippage in basis points', parseInt)
    .action(async (recipient, options) => {
      const sdk = getSdk()
      const client = sdk.getBridgingClient()
      const { routes } = await client.getRoutes({ recipient })
      const lightningRoutes = routes.filter((r) => r.source.chain === 'lightning')
      const route = await chooseBridgeRoute(rl, lightningRoutes)
      const feePolicy = options.feesIncluded ? 'feesIncluded' : undefined
      const response = await client.bridgeFromCashApp({
        recipient,
        route,
        amount: BigInt(options.amount),
        refundAddress: options.refundAddress,
        feePolicy,
        maxSlippageBps: options.maxSlippageBps
      })
      const info = response.info
      console.log('Open this URL in a browser to complete the purchase:')
      console.log(response.paymentRequest)
      console.log(
        `Deposit ~${info.depositAmount} sats; recipient receives ~${info.expectedReceivedAmount} ${info.destinationAsset}, ` +
        `service fee ${info.serviceFeeAmount} ${serviceFeeDenomination(info.serviceFeeAsset, info.serviceFeeAssetDecimals)}, ` +
        `expires ${info.expiresAt}`
      )
      printBridgeTracking(response)
    })

  // --- to-cash-app ---
  bridging
    .command('to-cash-app')
    .description('Bridge USDC/USDT from an external chain to a Cash App user. Returns the deposit for the payer to make.')
    .argument('<recipient>', 'Cash App $cashtag or Cash App Lightning address')
    .requiredOption('--amount <number>', 'Amount in the source stablecoin\'s base units (6 decimals for USDC/USDT)')
    .requiredOption('--refund-address <address>', 'The payer\'s address on the source chain, refunded if delivery fails')
    .option('--fees-included', 'Deduct fees from the amount instead of adding them on top', false)
    .option('--max-slippage-bps <bps>', 'Maximum slippage in basis points', parseInt)
    .action(async (recipient, options) => {
      const sdk = getSdk()
      const client = sdk.getBridgingClient()
      const { routes } = await client.getRoutes({ recipient })
      const route = await chooseBridgeRoute(rl, routes)
      const feePolicy = options.feesIncluded ? 'feesIncluded' : undefined
      const response = await client.bridgeToCashApp({
        recipient,
        route,
        amount: BigInt(options.amount),
        refundAddress: options.refundAddress,
        feePolicy,
        maxSlippageBps: options.maxSlippageBps
      })
      const info = response.info
      console.log('Ask the payer to pay:')
      console.log(response.paymentRequest)
      console.log(
        `Deposit ${info.depositAmount} ${route.source.asset} on ${route.source.chain} to ${info.depositAddress}. ` +
        `Recipient receives ~${info.expectedReceivedAmount} sats, ` +
        `service fee ${info.serviceFeeAmount} ${serviceFeeDenomination(info.serviceFeeAsset, info.serviceFeeAssetDecimals)}, ` +
        `expires at ${info.expiresAt}`
      )
      printBridgeTracking(response)
    })

  // --- status ---
  bridging
    .command('status')
    .description('Read a bridge\'s status')
    .argument('<bridge_id>', 'The bridge id `create` returned')
    .option('--status-token <token>', 'The status token `bridging create` returned')
    .action(async (bridgeId, options) => {
      const sdk = getSdk()
      const client = sdk.getBridgingClient()
      const response = await client.getBridgeStatus({
        bridgeId,
        statusToken: options.statusToken
      })
      printValue(response)
    })
}

/**
 * Prompt the user to pick one of `routes`, auto-selecting a lone route.
 */
async function chooseBridgeRoute(rl, routes) {
  if (!routes || routes.length === 0) {
    throw new Error('No bridge routes available')
  }

  if (routes.length === 1) {
    const route = routes[0]
    console.log(`Auto-selected route: ${routeLabel(route)}`)
    return route
  }

  console.log('Available bridge routes:')
  for (let i = 0; i < routes.length; i++) {
    console.log(`  ${i + 1}. ${routeLabel(routes[i])}`)
  }

  const line = await question(rl, 'Select route: ')
  const choice = parseInt(line.trim(), 10)
  if (isNaN(choice) || choice < 1 || choice > routes.length) {
    throw new Error('Selection out of range')
  }

  return routes[choice - 1]
}

function routeLabel(route) {
  return `${sideLabel(route.source)} -> ${sideLabel(route.destination)} [${route.provider}]`
}

function sideLabel(side) {
  return `${side.asset} on ${side.chain}${maybeTruncateAddress(side.contractAddress)}`
}

function maybeTruncateAddress(addr) {
  if (!addr) return ''
  if (addr.length > 12) {
    return ` (${addr.slice(0, 6)}...${addr.slice(-6)})`
  }
  return ` (${addr})`
}

function serviceFeeDenomination(asset, decimals) {
  if (asset == null) return 'sats'
  if (decimals != null) return `${asset} (${decimals} decimals)`
  return asset
}

function printBridgeTracking(response) {
  const token = response.statusToken
    ? ` --status-token ${response.statusToken}`
    : ''
  console.log(`Track it with: bridging status ${response.bridgeId}${token}`)
}

function question(rl, prompt) {
  return new Promise((resolve) => {
    rl.question(prompt, (answer) => {
      resolve(answer)
    })
  })
}

module.exports = { registerBridgingCommands, BRIDGING_COMMAND_NAMES }
