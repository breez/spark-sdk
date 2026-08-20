import { type BreezSdk } from '@breeztech/breez-sdk-spark'

const preparePaymentLinkViaCashapp = async (sdk: BreezSdk) => {
  // ANCHOR: prepare-payment-link-cashapp
  // Parse the recipient's external-chain address (EVM/Solana/Tron).
  const parsed = await sdk.parse('<recipient address>')
  if (parsed.type !== 'crossChainAddress') {
    throw new Error('Not a cross-chain address')
  }

  // List the stablecoin destinations you can send to and pick one, e.g. USDC on Base.
  // Restrict to Lightning-fundable routes, matching Cash App's Lightning funding.
  const routes = await sdk.getCrossChainRoutes({
    type: 'paymentLink',
    addressDetails: parsed
  })
  const route = routes.find((r) => r.asset === 'USDC' && r.chain === 'base')
  if (route === undefined) {
    throw new Error('No USDC on Base route available')
  }

  // Send $10 of USDC, funded by Cash App over Lightning. The amount is in the
  // route asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC,
  // about $10.
  const response = await sdk.preparePaymentLink({
    address: parsed.address,
    route,
    amount: BigInt(10_000_000),
    feePolicy: undefined,
    maxSlippageBps: undefined
  })
  console.log(`Open this URL in Cash App: ${response.url}`)
  console.log(`Recipient receives ~${response.estimatedOut} ${response.asset}`)
  // ANCHOR_END: prepare-payment-link-cashapp
}
