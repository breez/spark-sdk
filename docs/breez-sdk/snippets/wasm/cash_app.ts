import { type BridgingClient } from '@breeztech/breez-sdk-spark'

const exampleBridgeFromCashApp = async (client: BridgingClient) => {
  // ANCHOR: bridge-from-cash-app
  // List the routes to the recipient's external-chain address
  // (EVM/Solana/Tron) and pick a stablecoin Cash App can fund over
  // Lightning, e.g. USDC on Base.
  const recipient = '<recipient address>'
  const { routes } = await client.getRoutes({ recipient })
  const route = routes.find(
    (r) =>
      r.source.chain === 'lightning' &&
      r.destination.asset === 'USDC' &&
      r.destination.chain === 'base'
  )
  if (route === undefined) {
    throw new Error('No USDC route on Base')
  }

  // Send $10 of USDC, funded by Cash App over Lightning. The amount is in
  // USDC base units (6 decimals), so 10_000_000 = 10 USDC, about $10.
  const response = await client.bridgeFromCashApp({
    recipient,
    route,
    amount: BigInt(10_000_000),
    refundAddress: undefined,
    feePolicy: undefined,
    maxSlippageBps: undefined
  })

  // Open this Cash App link to pay. The recipient then receives the stablecoin.
  const info = response.info
  console.log(`Open this URL in Cash App: ${response.paymentRequest}`)
  console.log(`Recipient receives ~${info.expectedReceivedAmount} ${info.destinationAsset}`)
  // ANCHOR_END: bridge-from-cash-app
}

const exampleBridgeToCashApp = async (client: BridgingClient) => {
  // ANCHOR: bridge-to-cash-app
  // List the routes to the Cash App user and pick what the payer pays in,
  // e.g. USDC on Base.
  const recipient = '$alice'
  const { routes } = await client.getRoutes({ recipient })
  const route = routes.find((r) => r.source.asset === 'USDC' && r.source.chain === 'base')
  if (route === undefined) {
    throw new Error('No route from USDC on Base')
  }

  // Pay $10 of USDC to the Cash App user $alice. The amount is in USDC base
  // units (6 decimals), so 10_000_000 = 10 USDC, about $10. The deposit is
  // refunded to the payer's address if delivery fails.
  const response = await client.bridgeToCashApp({
    recipient,
    route,
    amount: BigInt(10_000_000),
    refundAddress: '<payer address>',
    feePolicy: undefined,
    maxSlippageBps: undefined
  })

  // Show the payer what to pay. The recipient then receives Bitcoin.
  const info = response.info
  console.log(`Pay with: ${response.paymentRequest}`)
  console.log(
    `Deposit ${info.depositAmount} to ${info.depositAddress}, ` +
      `recipient receives ~${info.expectedReceivedAmount} sats`
  )
  // ANCHOR_END: bridge-to-cash-app
}
