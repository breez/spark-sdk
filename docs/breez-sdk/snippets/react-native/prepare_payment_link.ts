import {
  CrossChainRouteFilter,
  InputType_Tags,
  type BreezSdk
} from '@breeztech/breez-sdk-spark-react-native'

const preparePaymentLinkViaCashapp = async (sdk: BreezSdk) => {
  // ANCHOR: prepare-payment-link-cashapp
  // Parse the recipient's external-chain address (EVM/Solana/Tron).
  const input = '<recipient address>'
  const parsed = await sdk.parse(input)
  if (parsed.tag !== InputType_Tags.CrossChainAddress) {
    throw new Error('Not a cross-chain address')
  }
  const addressDetails = parsed.inner[0]

  // List the stablecoin destinations you can send to and pick one, e.g. USDC
  // on Base. These routes are funded by an external rail, not the Spark wallet.
  // Restrict to Lightning-fundable routes.
  const routes = await sdk.getCrossChainRoutes(
    new CrossChainRouteFilter.PaymentLink({
      addressDetails
    })
  )
  const route = routes.find((r) => r.asset === 'USDC' && r.chain === 'base')
  if (route === undefined) {
    throw new Error('No USDC route on Base')
  }

  // Send $10 of USDC, funded by Cash App over Lightning. The amount is in USD
  // base units (6-decimal), so 10_000_000 = $10.00.
  const response = await sdk.preparePaymentLink({
    address: addressDetails.address,
    route,
    amount: BigInt(10_000_000),
    feePolicy: undefined,
    maxSlippageBps: undefined
  })

  // Open this Cash App URL to pay; the recipient then receives the stablecoin.
  console.log(`Open this URL in Cash App: ${response.url}`)
  console.log(`Recipient receives ~${response.estimatedOut} ${response.asset}`)
  // ANCHOR_END: prepare-payment-link-cashapp
}
