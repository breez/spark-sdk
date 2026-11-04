import {
  BridgeStatus,
  defaultBridgingConfig,
  newBridgingClient,
  type BreezSdk,
  type BridgingClientInterface
} from '@breeztech/breez-sdk-spark-react-native'

const exampleNewBridgingClient = async (): Promise<BridgingClientInterface> => {
  // ANCHOR: new-bridging-client
  const config = defaultBridgingConfig('<your api key>')
  const client = await newBridgingClient(config)
  // ANCHOR_END: new-bridging-client
  return client
}

const exampleGetBridgingClient = (sdk: BreezSdk): BridgingClientInterface => {
  // ANCHOR: get-bridging-client
  const client = sdk.getBridgingClient()
  // ANCHOR_END: get-bridging-client
  return client
}

const exampleCreateBridge = async (client: BridgingClientInterface) => {
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
  client: BridgingClientInterface,
  bridgeId: string,
  statusToken: string | undefined
): Promise<void> => {
  // ANCHOR: get-bridge-status
  const response = await client.getBridgeStatus({ bridgeId, statusToken })
  switch (response.status) {
    case BridgeStatus.Completed:
      console.log(`Delivered ${response.amountOut ?? 'unknown'}`)
      break
    case BridgeStatus.Refunded:
    case BridgeStatus.Failed:
    case BridgeStatus.Expired:
      console.log(`Not delivered: ${BridgeStatus[response.status]}`)
      break
    default:
      console.log(`Not final yet: ${BridgeStatus[response.status]}`)
  }
  // ANCHOR_END: get-bridge-status
}
