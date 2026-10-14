import { type BreezSdk } from '@breeztech/breez-sdk-spark'

const exampleBridgeFromCashApp = async (sdk: BreezSdk) => {
  // ANCHOR: bridge-from-cash-app
  // Parse the recipient's external-chain address (EVM/Solana/Tron).
  const input = '<recipient address>'
  const parsed = await sdk.parse(input)
  if (parsed.type !== 'crossChainAddress') {
    throw new Error('Not a cross-chain address')
  }

  // List the stablecoin destinations Cash App can fund over Lightning and
  // pick one, e.g. USDC on Base.
  const routes = await sdk.getCrossChainRoutes({
    type: 'send',
    addressDetails: parsed,
    deliveryMethod: 'lightning'
  })
  const route = routes.find((r) => r.asset === 'USDC' && r.chain === 'base')
  if (route === undefined) {
    throw new Error('No USDC route on Base')
  }

  // Send $10 of USDC, funded by Cash App over Lightning. The amount is in the
  // route asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC,
  // about $10.
  const response = await sdk.bridgeFromCashApp({
    address: parsed.address,
    route,
    amount: BigInt(10_000_000),
    feePolicy: undefined,
    maxSlippageBps: undefined
  })

  // Open this Cash App URL to pay. The recipient then receives the stablecoin.
  console.log(`Open this URL in Cash App: ${response.url}`)
  console.log(`Recipient receives ~${response.estimatedOut} ${response.asset}`)
  // ANCHOR_END: bridge-from-cash-app
}

const exampleBridgeToCashApp = async (sdk: BreezSdk) => {
  // ANCHOR: bridge-to-cash-app
  // List the stablecoin sources that can pay a Cash App user over Lightning
  // and pick one, e.g. USDC on Base.
  const routes = await sdk.getCrossChainRoutes({
    type: 'receive',
    contractAddress: undefined,
    deliveryMethod: 'lightning'
  })
  const route = routes.find((r) => r.asset === 'USDC' && r.chain === 'base')
  if (route === undefined) {
    throw new Error('No USDC route on Base')
  }

  // Pay $10 of USDC to the Cash App user $alice. The amount is in the route
  // asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC, about $10.
  // The deposit is refunded to the payer's address if delivery fails.
  const response = await sdk.bridgeToCashApp({
    recipient: '$alice',
    route,
    amount: BigInt(10_000_000),
    feePolicy: undefined,
    refundAddress: '<payer address>',
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
