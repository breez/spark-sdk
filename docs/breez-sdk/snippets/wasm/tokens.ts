import type { BreezSdk, TokenConversionOptions } from '@breeztech/breez-sdk-spark'

const exampleFetchTokenBalances = async (sdk: BreezSdk) => {
  // ANCHOR: fetch-token-balances
  const info = await sdk.getInfo({
    // ensureSynced: true will ensure the SDK is synced with the Spark network
    // before returning the balance
    ensureSynced: false
  })

  // Token balances are a map of token identifier to balance
  const tokenBalances = info.tokenBalances
  for (const [tokenId, tokenBalance] of Object.entries(tokenBalances)) {
    console.log(`Token ID: ${tokenId}`)
    console.log(`Balance: ${tokenBalance.balance}`)
    console.log(`Name: ${tokenBalance.tokenMetadata.name}`)
    console.log(`Ticker: ${tokenBalance.tokenMetadata.ticker}`)
    console.log(`Decimals: ${tokenBalance.tokenMetadata.decimals}`)
  }
  // ANCHOR_END: fetch-token-balances
}

const exampleFetchTokenMetadata = async (sdk: BreezSdk) => {
  // ANCHOR: fetch-token-metadata
  const response = await sdk.getTokensMetadata({
    tokenIdentifiers: ['<token identifier 1>', '<token identifier 2>']
  })

  const tokensMetadata = response.tokensMetadata
  for (const tokenMetadata of tokensMetadata) {
    console.log(`Token ID: ${tokenMetadata.identifier}`)
    console.log(`Name: ${tokenMetadata.name}`)
    console.log(`Ticker: ${tokenMetadata.ticker}`)
    console.log(`Decimals: ${tokenMetadata.decimals}`)
    console.log(`Max Supply: ${tokenMetadata.maxSupply}`)
    console.log(`Is Freezable: ${tokenMetadata.isFreezable}`)
  }
  // ANCHOR_END: fetch-token-metadata
}

const exampleReceiveTokenPaymentSparkInvoice = async (sdk: BreezSdk) => {
  // ANCHOR: receive-token-payment-spark-invoice
  const tokenIdentifier = '<token identifier>'
  const optionalDescription = '<invoice description>'
  const optionalAmount = '5000'
  // Optionally set the expiry UNIX timestamp in seconds
  const optionalExpiryTimeSeconds = 1716691200
  const optionalSenderPublicKey = '<sender public key>'

  const response = await sdk.receivePayment({
    paymentMethod: {
      type: 'sparkInvoice',
      tokenIdentifier,
      description: optionalDescription,
      amount: optionalAmount,
      expiryTime: optionalExpiryTimeSeconds,
      senderPublicKey: optionalSenderPublicKey
    }
  })

  const paymentRequest = response.paymentRequest
  console.log(`Payment request: ${paymentRequest}`)
  const receiveFeeSats = response.fee
  console.log(`Fees: ${receiveFeeSats} token base units`)
  // ANCHOR_END: receive-token-payment-spark-invoice
}

const exampleSendTokenPayment = async (sdk: BreezSdk) => {
  // ANCHOR: send-token-payment
  const paymentRequest = '<spark address or invoice>'
  // Token identifier must match the invoice in case it specifies one.
  const tokenIdentifier = '<token identifier>'
  // Set the amount of tokens you wish to send.
  const optionalAmount = BigInt(1_000)

  const prepareResponse = await sdk.prepareSendPayment({
    paymentRequest,
    amount: optionalAmount,
    tokenIdentifier
  })

  // If the fees are acceptable, continue to send the token payment
  if (prepareResponse.paymentMethod.type === 'sparkAddress') {
    console.log(`Token ID: ${prepareResponse.paymentMethod.tokenIdentifier}`)
    console.log(`Fees: ${prepareResponse.paymentMethod.fee} token base units`)
  }
  if (prepareResponse.paymentMethod.type === 'sparkInvoice') {
    console.log(`Token ID: ${prepareResponse.paymentMethod.tokenIdentifier}`)
    console.log(`Fees: ${prepareResponse.paymentMethod.fee} token base units`)
  }

  // Send the token payment
  const sendResponse = await sdk.sendPayment({
    prepareResponse,
    options: undefined
  })
  const payment = sendResponse.payment
  console.log(`Payment: ${JSON.stringify(payment)}`)
  // ANCHOR_END: send-token-payment
}

const exampleFetchTokenConversionLimits = async (sdk: BreezSdk) => {
  // ANCHOR: fetch-token-conversion-limits
  // Fetch limits for converting Bitcoin to a token
  const fromBitcoinResponse = await sdk.fetchTokenConversionLimits({
    conversionType: { type: 'fromBitcoin' },
    tokenIdentifier: '<token identifier>'
  })

  if (fromBitcoinResponse.minFromAmount !== undefined) {
    console.log(`Minimum BTC to convert: ${fromBitcoinResponse.minFromAmount} sats`)
  }
  if (fromBitcoinResponse.minToAmount !== undefined) {
    console.log(`Minimum tokens to receive: ${fromBitcoinResponse.minToAmount} base units`)
  }

  // Fetch limits for converting a token to Bitcoin
  const toBitcoinResponse = await sdk.fetchTokenConversionLimits({
    conversionType: {
      type: 'toBitcoin',
      fromTokenIdentifier: '<token identifier>'
    },
    tokenIdentifier: undefined
  })

  if (toBitcoinResponse.minFromAmount !== undefined) {
    console.log(`Minimum tokens to convert: ${toBitcoinResponse.minFromAmount} base units`)
  }
  if (toBitcoinResponse.minToAmount !== undefined) {
    console.log(`Minimum BTC to receive: ${toBitcoinResponse.minToAmount} sats`)
  }
  // ANCHOR_END: fetch-token-conversion-limits
}

const examplePrepareSendPaymentTokenConversion = async (sdk: BreezSdk) => {
  // ANCHOR: prepare-send-payment-token-conversion
  const paymentRequest = '<spark address or invoice>'
  // Token identifier must match the invoice in case it specifies one.
  const tokenIdentifier = '<token identifier>'
  // Set the amount of tokens you wish to send.
  const optionalAmount = BigInt(1_000)
  // Optionally set to use token funds to pay via token conversion
  const optionalMaxSlippageBps = 50
  const optionalCompletionTimeoutSecs = 30
  const tokenConversionOptions: TokenConversionOptions = {
    conversionType: {
      type: 'fromBitcoin'
    },
    maxSlippageBps: optionalMaxSlippageBps,
    completionTimeoutSecs: optionalCompletionTimeoutSecs
  }

  const prepareResponse = await sdk.prepareSendPayment({
    paymentRequest,
    amount: optionalAmount,
    tokenIdentifier,
    tokenConversionOptions
  })

  // If the fees are acceptable, continue to send the token payment
  if (prepareResponse.tokenConversionFee !== undefined) {
    const tokenConversionFee = prepareResponse.tokenConversionFee
    console.log(`Estimated token conversion fee: ${tokenConversionFee} sats`)
  }
  // ANCHOR_END: prepare-send-payment-token-conversion
}
