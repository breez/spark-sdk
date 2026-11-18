import {
  type BreezSdk,
  type BridgingClient,
  defaultBridgingConfig,
  newBridgingClient
} from '@breeztech/breez-sdk-spark'

const exampleNewBridgingClient = async () => {
  // ANCHOR: new-bridging-client
  const config = defaultBridgingConfig('<your api key>')
  const client = await newBridgingClient(config)
  // ANCHOR_END: new-bridging-client
  return client
}

const exampleGetBridgingClient = (sdk: BreezSdk) => {
  // ANCHOR: get-bridging-client
  const client = sdk.getBridgingClient()
  // ANCHOR_END: get-bridging-client
  return client
}

const exampleCreateBridge = async (client: BridgingClient) => {
  // ANCHOR: create-bridge
  // Who to pay: a Spark address, a Lightning address or an EVM/Solana/Tron
  // address.
  const recipient = '<recipient>'

  // List the routes that pay them and pick what the payer pays in, e.g.
  // USDC on Base.
  const { routes } = await client.getRoutes({ recipient })
  const route = routes.find((r) => r.source.asset === 'USDC' && r.source.chain === 'base')
  if (route === undefined) {
    throw new Error('No route from USDC on Base')
  }

  // Ask for $10. The amount is in the recipient's stablecoin when they are
  // paid in one, the payer's otherwise (6 decimals for USDC). The deposit is
  // refunded to the payer's address if the recipient can't be paid.
  const response = await client.createBridge({
    recipient,
    route,
    amount: BigInt(10_000_000),
    // Required to pay a Lightning address.
    refundAddress: '<payer address on Base>',
    feePolicy: undefined,
    maxSlippageBps: undefined
  })

  // Show the payer what to pay, and store the bridge id and status token to
  // look up the status later.
  console.log(`Pay with: ${response.paymentRequest}`)
  console.log(`Bridge id: ${response.bridgeId}`)
  // ANCHOR_END: create-bridge
}

const exampleGetBridgeStatus = async (
  client: BridgingClient,
  bridgeId: string,
  statusToken: string | undefined
) => {
  // ANCHOR: get-bridge-status
  const response = await client.getBridgeStatus({ bridgeId, statusToken })
  switch (response.status) {
    case 'completed': {
      console.log(`Delivered ${response.amountOut ?? 'unknown'}`)
      break
    }
    case 'refunded':
    case 'failed':
    case 'expired': {
      console.log(`Not delivered: ${response.status}`)
      break
    }
    default: {
      console.log(`Not final yet: ${response.status}`)
    }
  }
  // ANCHOR_END: get-bridge-status
}
