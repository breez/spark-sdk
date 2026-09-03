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
  // Only valid for sends with no token leg (see Retry safety).
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

const exampleGetCrossChainReceiveRoutes = async (sdk: BreezSdk) => {
  // ANCHOR: cross-chain-get-receive-routes
  const routes = await sdk.getCrossChainRoutes({
    type: 'receive',
    contractAddress: undefined
  })

  for (const route of routes) {
    console.debug(
      `Route via ${route.provider}: ${route.chain}/${route.asset} -> Spark`
    )
  }
  // ANCHOR_END: cross-chain-get-receive-routes
}

const exampleReceivePaymentCrossChain = async (
  sdk: BreezSdk,
  route: CrossChainRoutePair
) => {
  // ANCHOR: cross-chain-receive
  // amount is in the route's source-asset base units (USD-stable parity:
  // 1_000_000 = $1 on 6-decimal routes). See the guide for feeMode,
  // destination, and the slippage/overpay overrides.
  const amount = '1000000'
  const optionalDestination = undefined
  const optionalMaxSlippageBps = 100
  const optionalTargetOverpayBps = undefined
  const optionalFeeMode = undefined

  const response = await sdk.receivePayment({
    paymentMethod: {
      type: 'crossChain',
      route,
      amount,
      destination: optionalDestination,
      feeMode: optionalFeeMode,
      maxSlippageBps: optionalMaxSlippageBps,
      targetOverpayBps: optionalTargetOverpayBps
    }
  })

  console.debug(`Payment request: ${response.paymentRequest}`)
  if (response.crossChainInfo !== undefined) {
    const {
      depositAddress,
      depositAmount,
      expectedReceivedAmount,
      tokenIdentifier,
      expiresAt
    } = response.crossChainInfo
    const denom = tokenIdentifier !== undefined ? 'USDB' : 'BTC'
    console.debug(`Deposit address: ${depositAddress}`)
    console.debug(`Deposit amount: ${depositAmount}`)
    console.debug(`Expected received: ${expectedReceivedAmount} ${denom}`)
    console.debug(`Expires at: ${expiresAt}`)
  }
  // ANCHOR_END: cross-chain-receive
}
