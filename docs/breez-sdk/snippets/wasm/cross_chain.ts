import {
  type BreezSdk,
  type CrossChainAddressDetails,
  type CrossChainRoutePair,
  type PrepareSendPaymentResponse
} from '@breeztech/breez-sdk-spark'

const exampleGetCrossChainRoutes = async (sdk: BreezSdk) => {
  // ANCHOR: cross-chain-get-routes
  const input = '<recipient address>'
  const parsed = await sdk.parse(input)
  if (parsed.type !== 'crossChainAddress') {
    throw new Error('Not a cross-chain address')
  }

  const routes = await sdk.getCrossChainRoutes({
    type: 'send',
    addressDetails: parsed
  })

  for (const route of routes) {
    console.debug(`Route via ${route.provider}: ${route.chain}/${route.asset}`)
  }
  // ANCHOR_END: cross-chain-get-routes
}

const examplePrepareSendPaymentCrossChain = async (
  sdk: BreezSdk,
  addressDetails: CrossChainAddressDetails,
  route: CrossChainRoutePair
) => {
  // ANCHOR: cross-chain-prepare
  // Optionally set the maximum slippage in basis points (10 to 500)
  const optionalMaxSlippageBps = 100

  const prepareResponse = await sdk.prepareSendPayment({
    paymentRequest: {
      type: 'crossChain',
      address: addressDetails.address,
      route,
      maxSlippageBps: optionalMaxSlippageBps
    },
    amount: BigInt(50_000),
    tokenIdentifier: undefined,
    conversionOptions: undefined,
    feePolicy: undefined
  })

  if (prepareResponse.paymentMethod.type === 'crossChainAddress') {
    const { amountIn, estimatedOut, feeAmount, expiresAt } = prepareResponse.paymentMethod
    console.debug(`Amount in: ${amountIn}`)
    console.debug(`Estimated out: ${estimatedOut}`)
    console.debug(`Provider fee: ${feeAmount}`)
    console.debug(`Quote expires at: ${expiresAt}`)
  }
  // ANCHOR_END: cross-chain-prepare
}

const exampleSendPaymentCrossChain = async (
  sdk: BreezSdk,
  prepareResponse: PrepareSendPaymentResponse
) => {
  // ANCHOR: cross-chain-send
  const optionalIdempotencyKey = '<idempotency key uuid>'
  const sendResponse = await sdk.sendPayment({
    prepareResponse,
    options: undefined,
    idempotencyKey: optionalIdempotencyKey
  })
  const payment = sendResponse.payment
  console.debug('Payment:', payment)
  // ANCHOR_END: cross-chain-send
}
