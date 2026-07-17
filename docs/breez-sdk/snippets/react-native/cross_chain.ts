import {
  CrossChainRouteFilter,
  InputType_Tags,
  PaymentRequest,
  ReceivePaymentMethod,
  SendPaymentMethod_Tags,
  type BreezSdk,
  type CrossChainAddressDetails,
  type CrossChainRoutePair,
  type PrepareSendPaymentResponse,
  type SparkAsset
} from '@breeztech/breez-sdk-spark-react-native'

const exampleGetCrossChainRoutes = async (sdk: BreezSdk) => {
  // ANCHOR: cross-chain-get-routes
  const input = '<recipient address>'
  const parsed = await sdk.parse(input)
  if (parsed.tag !== InputType_Tags.CrossChainAddress) {
    throw new Error('Not a cross-chain address')
  }
  const addressDetails = parsed.inner[0]

  const routes = await sdk.getCrossChainRoutes(
    new CrossChainRouteFilter.Send({ addressDetails })
  )

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
    paymentRequest: new PaymentRequest.CrossChain({
      address: addressDetails.address,
      route,
      maxSlippageBps: optionalMaxSlippageBps,
      targetOverpayBps: undefined
    }),
    amount: BigInt(50_000),
    tokenIdentifier: undefined,
    conversionOptions: undefined,
    feePolicy: undefined
  })

  if (prepareResponse.paymentMethod?.tag === SendPaymentMethod_Tags.CrossChainAddress) {
    const inner = prepareResponse.paymentMethod.inner
    console.debug(`Amount in: ${inner.amountIn}`)
    console.debug(`Estimated out: ${inner.estimatedOut}`)
    console.debug(`Provider fee: ${inner.feeAmount}`)
    console.debug(`Quote expires at: ${inner.expiresAt}`)
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
  console.debug('Payment:', sendResponse.payment)
  // ANCHOR_END: cross-chain-send
}

const exampleGetCrossChainReceiveRoutes = async (sdk: BreezSdk) => {
  // ANCHOR: cross-chain-get-receive-routes
  const routes = await sdk.getCrossChainRoutes(
    new CrossChainRouteFilter.Receive({ contractAddress: undefined })
  )

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
  // With the default FeesExcluded mode, amount is the receiver's net target
  // on Spark in destination-asset base units (sats for BTC, token base units
  // for USDB). The SDK pads the sender's deposit to cover fees + overpay.
  // With FeesIncluded, amount is the sender's deposit in source-asset units.
  const amount = BigInt(1_000)
  // Optionally set the destination Spark-side asset. undefined = auto:
  // active stable-balance token if the route supports it, otherwise BTC.
  const optionalDestination: SparkAsset | undefined = undefined
  // Optionally set the maximum slippage in basis points (10 to 500)
  const optionalMaxSlippageBps = 100
  // Optionally override the overpay buffer (0 to 500 bps). Defaults to 15.
  const optionalTargetOverpayBps = undefined
  // Optionally override the fee mode. Defaults to FeesExcluded.
  const optionalFeeMode = undefined

  const response = await sdk.receivePayment({
    paymentMethod: new ReceivePaymentMethod.CrossChain({
      route,
      amount,
      destination: optionalDestination,
      feeMode: optionalFeeMode,
      maxSlippageBps: optionalMaxSlippageBps,
      targetOverpayBps: optionalTargetOverpayBps
    })
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
