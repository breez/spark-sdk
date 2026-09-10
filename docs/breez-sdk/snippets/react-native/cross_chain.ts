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
    // Amount bounds are published per accepted asset. Read the ones for
    // the asset you intend to pay with, before quoting.
    for (const accepted of route.acceptedAssets) {
      const limits = accepted.limits
      if (limits === undefined) {
        continue
      }
      console.debug(
        `  ${accepted.asset.tag} minimum: ${limits.minAmount} base units / ` +
          `${limits.minUsdCents} USD cents`
      )
      if (limits.dynamicLimitsPossible) {
        console.debug('  The provider may enforce a higher minimum than published')
      }
    }
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
  // amount is in the route's source-asset base units (USD-stable parity:
  // 1_000_000 = $1 on 6-decimal routes). See the guide for feeMode,
  // destination, and the slippage/overpay overrides.
  const amount = BigInt(1_000_000)
  const optionalDestination: SparkAsset | undefined = undefined
  const optionalMaxSlippageBps = 100
  const optionalTargetOverpayBps = undefined
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
      destinationAsset,
      expiresAt
    } = response.crossChainInfo
    console.debug(`Deposit address: ${depositAddress}`)
    console.debug(`Deposit amount: ${depositAmount}`)
    console.debug(
      `Expected received: ${expectedReceivedAmount} ${destinationAsset}`
    )
    console.debug(`Expires at: ${expiresAt}`)
  }
  // ANCHOR_END: cross-chain-receive
}
